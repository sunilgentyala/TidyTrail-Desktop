const { invoke } = window.__TAURI__.core;
const { open } = window.__TAURI__.dialog;

const CATEGORY_COLORS = {
  directory: "#4c72b0",
  archive: "#dd8452",
  audio: "#8172b3",
  video: "#c44e52",
  image: "#55a868",
  document: "#64b5cd",
  code: "#937860",
  executable: "#da8bc3",
  system: "#8c8c8c",
  other: "#ccb974",
};

const CATEGORY_LABELS = {
  directory: "Folder",
  archive: "Archive",
  audio: "Audio",
  video: "Video",
  image: "Image",
  document: "Document",
  code: "Code",
  executable: "Executable",
  system: "System",
  other: "Other",
};

function colorFor(node) {
  return CATEGORY_COLORS[node.category] || CATEGORY_COLORS.other;
}

function formatBytes(bytes) {
  const units = ["B", "KB", "MB", "GB", "TB", "PB"];
  if (bytes === 0) return "0 B";
  let value = bytes;
  let i = 0;
  while (value >= 1024 && i < units.length - 1) {
    value /= 1024;
    i += 1;
  }
  return i === 0 ? `${bytes} B` : `${value.toFixed(1)} ${units[i]}`;
}

/**
 * Squarified treemap layout (Bruls, Huizing, van Wijk 2000). Mirrors
 * core/src/treemap.rs so the visual layout matches the tested Rust
 * reference implementation; kept in JS here so it can re-run on every
 * resize/drill-down without an IPC round trip.
 */
function squarify(items, bounds) {
  const sorted = items.filter((i) => i.value > 0).sort((a, b) => b.value - a.value);
  const out = [];
  if (sorted.length === 0 || bounds.w <= 0 || bounds.h <= 0) return out;

  const total = sorted.reduce((s, i) => s + i.value, 0);
  if (total <= 0) return out;
  const scale = (bounds.w * bounds.h) / total;

  let remaining = { ...bounds };
  let row = [];
  let rowItems = [];
  let idx = 0;

  const worstRatio = (r, side) => {
    if (r.length === 0 || side <= 0) return Infinity;
    const sum = r.reduce((s, v) => s + v, 0);
    const max = Math.max(...r);
    const min = Math.min(...r);
    const sideSq = side * side;
    const sumSq = sum * sum;
    return Math.max((sideSq * max) / sumSq, sumSq / (sideSq * min));
  };

  const layOutRow = (r, rItems, space) => {
    const rowSum = r.reduce((s, v) => s + v, 0);
    if (rowSum <= 0) return space;
    if (space.w >= space.h) {
      const stripW = rowSum / space.h;
      let y = space.y;
      for (let k = 0; k < r.length; k += 1) {
        const h = r[k] / stripW;
        out.push({ data: rItems[k].data, rect: { x: space.x, y, w: stripW, h } });
        y += h;
      }
      return { x: space.x + stripW, y: space.y, w: space.w - stripW, h: space.h };
    }
    const stripH = rowSum / space.w;
    let x = space.x;
    for (let k = 0; k < r.length; k += 1) {
      const w = r[k] / stripH;
      out.push({ data: rItems[k].data, rect: { x, y: space.y, w, h: stripH } });
      x += w;
    }
    return { x: space.x, y: space.y + stripH, w: space.w, h: space.h - stripH };
  };

  while (idx < sorted.length) {
    const item = sorted[idx];
    const value = item.value * scale;
    const side = Math.min(remaining.w, remaining.h);

    if (row.length === 0 || worstRatio(row, side) >= worstRatio([...row, value], side)) {
      row.push(value);
      rowItems.push(item);
      idx += 1;
    } else {
      remaining = layOutRow(row, rowItems, remaining);
      row = [];
      rowItems = [];
    }
  }
  if (row.length > 0) layOutRow(row, rowItems, remaining);

  return out;
}

const state = {
  path: [], // stack of NodeDto from scan root to the currently viewed folder
  issues: [],
  viewMode: "treemap",
  scanning: false,
  selected: new Set(), // selected child paths within the current view
  pendingDelete: null,
  expandedPaths: new Set(), // directory paths expanded in the folder tree panel
};

const el = {
  rootList: document.getElementById("root-list"),
  legend: document.getElementById("legend"),
  breadcrumb: document.getElementById("breadcrumb"),
  upBtn: document.getElementById("up-btn"),
  viewToggleBtn: document.getElementById("view-toggle-btn"),
  rescanBtn: document.getElementById("rescan-btn"),
  deleteBtn: document.getElementById("delete-btn"),
  statusText: document.getElementById("status-text"),
  issuesToggle: document.getElementById("issues-toggle"),
  view: document.getElementById("view"),
  browseBtn: document.getElementById("browse-btn"),
  folderTree: document.getElementById("folder-tree"),
};

function currentNode() {
  return state.path[state.path.length - 1] || null;
}

function selectedNodes() {
  const node = currentNode();
  if (!node) return [];
  return node.children.filter((c) => state.selected.has(c.path));
}

async function loadRoots() {
  try {
    const roots = await invoke("list_roots");
    el.rootList.innerHTML = "";
    for (const root of roots) {
      const li = document.createElement("li");
      const btn = document.createElement("button");
      btn.type = "button";
      btn.textContent = `${root.name}`;
      btn.title = root.path;
      btn.addEventListener("click", () => scanPath(root.path));
      li.appendChild(btn);
      el.rootList.appendChild(li);
    }
  } catch (e) {
    console.error("Failed to list roots", e);
  }
}

function renderLegend() {
  el.legend.innerHTML = "";
  for (const [key, label] of Object.entries(CATEGORY_LABELS)) {
    const row = document.createElement("div");
    row.className = "legend-row";
    const swatch = document.createElement("span");
    swatch.className = "legend-swatch";
    swatch.style.background = CATEGORY_COLORS[key];
    row.appendChild(swatch);
    const text = document.createElement("span");
    text.textContent = label;
    row.appendChild(text);
    el.legend.appendChild(row);
  }
}

async function scanPath(path) {
  state.scanning = true;
  state.selected.clear();
  renderToolbar();
  el.statusText.textContent = `Scanning ${path}…`;
  el.view.innerHTML = `<div class="scanning-state"><strong>Scanning…</strong><span>${path}</span><span id="scan-progress-count" class="scan-progress-count"></span><button class="toolbar-btn" id="cancel-scan-btn" type="button">Cancel</button></div>`;
  document.getElementById("cancel-scan-btn").addEventListener("click", () => {
    invoke("cancel_scan").catch(() => {});
  });

  const progressEl = document.getElementById("scan-progress-count");
  const unlisten = await window.__TAURI__.event.listen("scan://progress", (event) => {
    if (progressEl) {
      progressEl.textContent = `${event.payload.visited.toLocaleString()} items scanned…`;
    }
  });

  try {
    const result = await invoke("scan_path", { path });
    state.path = [result.root];
    state.issues = result.issues;
    state.expandedPaths = new Set([result.root.path]);
  } catch (e) {
    el.statusText.textContent = `Could not scan ${path}: ${e}`;
  } finally {
    unlisten();
  }

  state.scanning = false;
  renderAll();
}

function goUp() {
  if (state.path.length > 1) {
    state.path.pop();
    state.selected.clear();
    renderAll();
  }
}

function drillInto(node) {
  if (!node.is_dir) return;
  state.path.push(node);
  state.selected.clear();
  expandAncestors(state.path);
  renderAll();
}

function goToBreadcrumb(index) {
  state.path = state.path.slice(0, index + 1);
  state.selected.clear();
  expandAncestors(state.path);
  renderAll();
}

/** Finds the chain of nodes from `root` down to the node at `targetPath`,
 * root inclusive. Only descends into directories, since the tree panel only
 * shows directories. Returns null if targetPath isn't under root. */
function findChain(root, targetPath) {
  if (root.path === targetPath) return [root];
  for (const child of root.children) {
    if (!child.is_dir) continue;
    const chain = findChain(child, targetPath);
    if (chain) return [root, ...chain];
  }
  return null;
}

function expandAncestors(chain) {
  for (const node of chain) state.expandedPaths.add(node.path);
}

/** Jumps the main view straight to any folder already in the scanned tree,
 * the way clicking a node in TreeSize's folder tree does, rather than only
 * being able to move one level at a time via drill-down/breadcrumb. */
function navigateToPath(targetPath) {
  if (state.path.length === 0) return;
  const chain = findChain(state.path[0], targetPath);
  if (!chain) return;
  state.path = chain;
  state.selected.clear();
  expandAncestors(chain);
  renderAll();
}

function renderBreadcrumb() {
  el.breadcrumb.innerHTML = "";
  state.path.forEach((node, index) => {
    if (index > 0) {
      const sep = document.createElement("span");
      sep.className = "sep";
      sep.textContent = "/";
      el.breadcrumb.appendChild(sep);
    }
    const btn = document.createElement("button");
    btn.type = "button";
    btn.textContent = node.name || node.path;
    btn.disabled = index === state.path.length - 1;
    btn.addEventListener("click", () => goToBreadcrumb(index));
    el.breadcrumb.appendChild(btn);
  });
}

function renderToolbar() {
  const node = currentNode();
  el.upBtn.disabled = state.scanning || state.path.length <= 1;
  el.viewToggleBtn.disabled = state.scanning || !node;
  el.rescanBtn.disabled = state.scanning || !node;
  el.viewToggleBtn.textContent = state.viewMode === "treemap" ? "List view" : "Treemap view";
  el.deleteBtn.disabled = state.scanning || state.selected.size === 0;
  el.browseBtn.disabled = state.scanning;
}

function renderStatusBar() {
  const node = currentNode();
  if (!node) {
    el.statusText.textContent = "Pick a folder or drive to scan.";
  } else {
    const count = node.children.length;
    let text = `${node.name || node.path} · ${node.size_label} across ${count} item${count === 1 ? "" : "s"}`;
    if (state.selected.size > 0) {
      const total = selectedNodes().reduce((s, n) => s + n.size, 0);
      text += ` · ${state.selected.size} selected (${formatBytes(total)})`;
    }
    el.statusText.textContent = text;
  }

  if (state.issues.length > 0) {
    el.issuesToggle.hidden = false;
    el.issuesToggle.textContent = `${state.issues.length} path${state.issues.length === 1 ? "" : "s"} skipped`;
  } else {
    el.issuesToggle.hidden = true;
  }
}

function renderAll() {
  renderToolbar();
  renderBreadcrumb();
  renderStatusBar();
  renderView();
  renderFolderTree();
}

function renderFolderTree() {
  el.folderTree.innerHTML = "";
  const root = state.path[0];
  if (!root) {
    const empty = document.createElement("div");
    empty.className = "tree-empty";
    empty.textContent = "Scan a folder to see its structure here.";
    el.folderTree.appendChild(empty);
    return;
  }
  renderTreeNode(root, 0, el.folderTree);
}

function renderTreeNode(node, depth, container) {
  const isExpanded = state.expandedPaths.has(node.path);
  const active = currentNode();
  const isActive = active && active.path === node.path;
  const dirChildren = node.children.filter((c) => c.is_dir);

  const row = document.createElement("div");
  row.className = "tree-row" + (isActive ? " active" : "");
  row.style.paddingLeft = `${4 + depth * 14}px`;
  row.title = node.path;

  const toggle = document.createElement("span");
  toggle.className = "tree-toggle";
  if (dirChildren.length > 0) {
    toggle.textContent = isExpanded ? "▾" : "▸";
    toggle.addEventListener("click", (evt) => {
      evt.stopPropagation();
      if (isExpanded) {
        state.expandedPaths.delete(node.path);
      } else {
        state.expandedPaths.add(node.path);
      }
      renderFolderTree();
    });
  }
  row.appendChild(toggle);

  const icon = document.createElement("span");
  icon.className = "tree-icon";
  icon.textContent = "📁";
  row.appendChild(icon);

  const label = document.createElement("span");
  label.className = "tree-label";
  label.textContent = node.name || node.path;
  row.appendChild(label);

  const size = document.createElement("span");
  size.className = "tree-size";
  size.textContent = node.size_label;
  row.appendChild(size);

  row.addEventListener("click", () => navigateToPath(node.path));

  container.appendChild(row);

  if (isExpanded) {
    for (const child of dirChildren) {
      renderTreeNode(child, depth + 1, container);
    }
  }
}

function renderView() {
  const node = currentNode();
  if (!node) {
    el.view.innerHTML = `<div class="empty-state"><strong>No scan yet</strong><span>Choose a folder or drive on the left to see what's taking up space.</span></div>`;
    return;
  }
  if (node.children.length === 0) {
    el.view.innerHTML = `<div class="empty-state"><strong>Empty</strong><span>This folder has no files TidyTrail could read.</span></div>`;
    return;
  }
  if (state.viewMode === "treemap") {
    renderTreemapView(node);
  } else {
    renderListView(node);
  }
}

let hoverTooltipEl = null;

function renderTreemapView(node) {
  el.view.innerHTML = `<canvas id="treemap-canvas"></canvas>`;
  const canvas = document.getElementById("treemap-canvas");
  const ctx = canvas.getContext("2d");

  let placements = [];

  function draw() {
    const dpr = window.devicePixelRatio || 1;
    const rect = el.view.getBoundingClientRect();
    canvas.width = rect.width * dpr;
    canvas.height = rect.height * dpr;
    canvas.style.width = `${rect.width}px`;
    canvas.style.height = `${rect.height}px`;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, rect.width, rect.height);

    const items = node.children.map((c) => ({ value: c.size, data: c }));
    placements = squarify(items, { x: 0, y: 0, w: rect.width, h: rect.height });

    for (const { data, rect: r } of placements) {
      const selected = state.selected.has(data.path);
      ctx.fillStyle = colorFor(data);
      ctx.globalAlpha = selected ? 1 : 0.92;
      ctx.fillRect(r.x, r.y, r.w, r.h);
      ctx.globalAlpha = 1;

      ctx.strokeStyle = selected ? "#ffffff" : "rgba(0,0,0,0.25)";
      ctx.lineWidth = selected ? 2 : 1;
      ctx.strokeRect(r.x + 0.5, r.y + 0.5, Math.max(r.w - 1, 0), Math.max(r.h - 1, 0));

      if (r.w > 46 && r.h > 20) {
        ctx.save();
        ctx.beginPath();
        ctx.rect(r.x, r.y, r.w, r.h);
        ctx.clip();
        ctx.fillStyle = "rgba(255,255,255,0.96)";
        ctx.shadowColor = "rgba(0,0,0,0.6)";
        ctx.shadowBlur = 3;
        ctx.font = "12px -apple-system, Segoe UI, sans-serif";
        ctx.fillText(data.name, r.x + 5, r.y + 15, r.w - 10);
        if (r.h > 34) {
          ctx.font = "11px -apple-system, Segoe UI, sans-serif";
          ctx.fillStyle = "rgba(255,255,255,0.85)";
          ctx.fillText(data.size_label, r.x + 5, r.y + 30, r.w - 10);
        }
        ctx.restore();
      }
    }
  }

  function hitTest(evt) {
    const bounds = canvas.getBoundingClientRect();
    const x = evt.clientX - bounds.left;
    const y = evt.clientY - bounds.top;
    for (const { data, rect: r } of placements) {
      if (x >= r.x && x <= r.x + r.w && y >= r.y && y <= r.y + r.h) return data;
    }
    return null;
  }

  canvas.addEventListener("click", (evt) => {
    const data = hitTest(evt);
    if (!data) return;
    state.selected.clear();
    state.selected.add(data.path);
    renderToolbar();
    renderStatusBar();
    draw();
  });

  canvas.addEventListener("dblclick", (evt) => {
    const data = hitTest(evt);
    if (data && data.is_dir) drillInto(data);
  });

  canvas.addEventListener("mousemove", (evt) => {
    const data = hitTest(evt);
    if (!data) {
      hideTooltip();
      return;
    }
    showTooltip(evt, data);
  });
  canvas.addEventListener("mouseleave", hideTooltip);

  const observer = new ResizeObserver(() => draw());
  observer.observe(el.view);
  draw();
}

function showTooltip(evt, data) {
  if (!hoverTooltipEl) {
    hoverTooltipEl = document.createElement("div");
    hoverTooltipEl.className = "tooltip";
    document.body.appendChild(hoverTooltipEl);
  }
  hoverTooltipEl.innerHTML = `<strong>${data.name}</strong><br>${data.size_label} · ${CATEGORY_LABELS[data.category] || "Other"}`;
  hoverTooltipEl.style.left = `${evt.clientX + 14}px`;
  hoverTooltipEl.style.top = `${evt.clientY + 14}px`;
  hoverTooltipEl.style.display = "block";
}

function hideTooltip() {
  if (hoverTooltipEl) hoverTooltipEl.style.display = "none";
}

function renderListView(node) {
  const container = document.createElement("div");
  container.className = "list-view";
  const table = document.createElement("table");
  table.innerHTML = `<thead><tr><th></th><th>Name</th><th>Size</th><th class="bar-cell">Share</th></tr></thead>`;
  const tbody = document.createElement("tbody");

  const maxSize = node.children.reduce((m, c) => Math.max(m, c.size), 1);
  const sorted = [...node.children].sort((a, b) => b.size - a.size);

  for (const child of sorted) {
    const tr = document.createElement("tr");
    if (state.selected.has(child.path)) tr.classList.add("selected");

    const checkTd = document.createElement("td");
    const checkbox = document.createElement("input");
    checkbox.type = "checkbox";
    checkbox.checked = state.selected.has(child.path);
    checkbox.addEventListener("click", (evt) => {
      evt.stopPropagation();
      toggleSelected(child.path);
    });
    checkTd.appendChild(checkbox);

    const nameTd = document.createElement("td");
    const nameWrap = document.createElement("div");
    nameWrap.className = "name-cell";
    const swatch = document.createElement("span");
    swatch.className = "swatch";
    swatch.style.background = colorFor(child);
    const nameSpan = document.createElement("span");
    nameSpan.className = "name";
    nameSpan.textContent = child.is_dir ? `📁 ${child.name}` : child.name;
    nameWrap.appendChild(swatch);
    nameWrap.appendChild(nameSpan);
    nameTd.appendChild(nameWrap);

    const sizeTd = document.createElement("td");
    sizeTd.textContent = child.size_label;

    const barTd = document.createElement("td");
    barTd.className = "bar-cell";
    const track = document.createElement("div");
    track.className = "bar-track";
    const fill = document.createElement("div");
    fill.className = "bar-fill";
    fill.style.width = `${Math.max((child.size / maxSize) * 100, 2)}%`;
    fill.style.background = colorFor(child);
    track.appendChild(fill);
    barTd.appendChild(track);

    tr.appendChild(checkTd);
    tr.appendChild(nameTd);
    tr.appendChild(sizeTd);
    tr.appendChild(barTd);

    tr.addEventListener("click", () => toggleSelected(child.path));
    tr.addEventListener("dblclick", () => drillInto(child));

    tbody.appendChild(tr);
  }

  table.appendChild(tbody);
  container.appendChild(table);
  el.view.innerHTML = "";
  el.view.appendChild(container);
}

function toggleSelected(path) {
  if (state.selected.has(path)) {
    state.selected.delete(path);
  } else {
    state.selected.add(path);
  }
  renderToolbar();
  renderStatusBar();
  renderView();
}

function requestDelete() {
  const nodes = selectedNodes();
  if (nodes.length === 0) return;
  state.pendingDelete = nodes;
  const totalSize = nodes.reduce((s, n) => s + n.size, 0);
  const panel = document.createElement("div");
  panel.className = "confirm-panel";
  panel.innerHTML = `
    <div class="confirm-box">
      <h2>Move to Trash?</h2>
      <p>${nodes.length} item${nodes.length === 1 ? "" : "s"} (${formatBytes(totalSize)}) will be moved to the system trash/recycle bin, not permanently deleted.</p>
      <div class="confirm-actions">
        <button class="toolbar-btn" id="cancel-delete-btn" type="button">Cancel</button>
        <button class="toolbar-btn danger" id="confirm-delete-btn" type="button">Move to Trash</button>
      </div>
    </div>`;
  el.view.appendChild(panel);
  document.getElementById("cancel-delete-btn").addEventListener("click", () => panel.remove());
  document.getElementById("confirm-delete-btn").addEventListener("click", () => {
    panel.remove();
    performDelete(nodes);
  });
}

async function performDelete(nodes) {
  const paths = nodes.map((n) => n.path);
  el.statusText.textContent = `Moving ${paths.length} item${paths.length === 1 ? "" : "s"} to trash…`;
  try {
    const outcome = await invoke("delete_paths", { paths });
    const node = currentNode();
    if (node) {
      const deletedSet = new Set(outcome.deleted);
      node.children = node.children.filter((c) => !deletedSet.has(c.path));
      node.size = node.children.reduce((s, c) => s + c.size, 0);
    }
    state.selected.clear();
    if (outcome.failed.length > 0) {
      el.statusText.textContent = `Deleted ${outcome.deleted.length}, failed ${outcome.failed.length}: ${outcome.failed
        .map((f) => f.message)
        .join("; ")}`;
    }
    renderAll();
  } catch (e) {
    el.statusText.textContent = `Delete failed: ${e}`;
  }
}

el.browseBtn.addEventListener("click", async () => {
  try {
    const dir = await open({ directory: true, multiple: false });
    if (dir) scanPath(Array.isArray(dir) ? dir[0] : dir);
  } catch (e) {
    console.error("Folder picker failed", e);
  }
});

el.upBtn.addEventListener("click", goUp);
el.viewToggleBtn.addEventListener("click", () => {
  state.viewMode = state.viewMode === "treemap" ? "list" : "treemap";
  renderAll();
});
el.rescanBtn.addEventListener("click", () => {
  if (state.path.length > 0) scanPath(state.path[0].path);
});
el.deleteBtn.addEventListener("click", requestDelete);
el.issuesToggle.addEventListener("click", () => {
  const messages = state.issues.map((i) => `${i.path}: ${i.message}`).join("\n");
  window.__TAURI__.dialog.message(messages || "No issues.", { title: "Skipped paths", kind: "info" });
});

renderLegend();
loadRoots();
renderAll();
