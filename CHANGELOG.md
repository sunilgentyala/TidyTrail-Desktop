# Changelog

## 0.2.0 - 2026-09-24

Security and robustness hardening ahead of use on managed servers.

### Security
- **Delete requests are now authorized by the backend.** `delete_paths`
  previously trashed any path the webview sent. Every path is now
  re-validated in Rust against the last completed scan: absolute, no
  `.`/`..`, strictly inside the scanned folder (checked via the canonical
  parent, so symlinked parents can't escape), not the scanned folder itself,
  and not a protected OS location (`C:\Windows`, `/usr`, `/etc`, volume and
  profile roots, ...). New `core/src/delete_guard.rs`, with unit tests and
  IPC-level tests through Tauri's mock runtime.
- **Fixed HTML injection from file names.** The treemap tooltip and the
  "Scanning..." view built HTML with `innerHTML` from file names and paths,
  which any user who can create a file in a scanned folder controls. Both
  now use `textContent`.
- **Deletion audit log.** Every delete attempt, allowed or refused, is
  appended to `deletions.jsonl` (location configurable with
  `TIDYTRAIL_AUDIT_LOG_DIR`); deletion fails closed if the log can't be
  written.
- Stricter CSP (no `'unsafe-inline'` styles; `object-src`, `base-uri`,
  `form-action`, `frame-src` locked down) and `freezePrototype`.
- Least-privilege capabilities: removed the unused `opener` plugin and
  narrowed `dialog:default` to `dialog:allow-open` + `dialog:allow-message`.
- Release pipeline: actions pinned to commit SHAs, read-only default token,
  no build cache on release builds, SHA-256 checksums and signed SLSA
  build-provenance attestations for every asset, tag name passed via env
  instead of script interpolation.
- CI: `cargo fmt`, `cargo clippy -D warnings`, `cargo deny` (RustSec,
  licenses, sources), CodeQL, Dependabot. Added `SECURITY.md`.

### Fixed
- **Stack overflow on deep directory trees.** The recursive scanner crashed
  on a tree only ~260 levels deep (reproduced in a test). The scan is now
  iterative, and the result tree is capped at 256 levels (reported as a
  scan issue).
- **Scanning `/` on Linux counted kernel pseudo-filesystems**, e.g.
  `/proc/kcore` reporting ~128 TB. `/proc`, `/sys` and other pseudo-fs mounts
  (detected by `statfs` magic, not by name) are now skipped and reported.
- Starting a new scan before the previous one finished left the old scan
  running and let its result overwrite the new one; each scan now has its
  own cancellation flag and superseded results are discarded.
- Delete and scan error messages were immediately overwritten by the normal
  status line, so partial delete failures went unnoticed.
- Folder sizes above the current folder stayed stale after a delete.
- Each re-render leaked a `ResizeObserver` that kept redrawing old canvases.
- The delete confirmation now lists the exact paths being trashed, with
  Cancel focused by default.

## 0.1.0

Initial release.
