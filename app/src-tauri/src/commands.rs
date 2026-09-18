use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde::Serialize;
use tauri::State;
use tidytrail_core::{categorize, format_bytes, move_all_to_trash, scan, Node, ScanIssue};

/// One active-scan cancellation flag, reset at the start of every scan.
/// A single flag is enough because the UI only ever runs one scan at a
/// time; starting a new scan implicitly abandons interest in the old one.
#[derive(Default)]
pub struct ScanState {
    pub cancelled: Arc<AtomicBool>,
}

#[derive(Serialize)]
pub struct NodeDto {
    pub name: String,
    pub path: String,
    pub size: u64,
    pub size_label: String,
    pub is_dir: bool,
    pub category: &'static str,
    pub children: Vec<NodeDto>,
}

impl From<Node> for NodeDto {
    fn from(node: Node) -> Self {
        let category = categorize(&node.path, node.is_dir).as_str();
        NodeDto {
            name: node.name,
            path: node.path.to_string_lossy().into_owned(),
            size: node.size,
            size_label: format_bytes(node.size),
            is_dir: node.is_dir,
            category,
            children: node.children.into_iter().map(NodeDto::from).collect(),
        }
    }
}

#[derive(Serialize)]
pub struct ScanIssueDto {
    pub path: String,
    pub message: String,
}

impl From<ScanIssue> for ScanIssueDto {
    fn from(issue: ScanIssue) -> Self {
        ScanIssueDto {
            path: issue.path.to_string_lossy().into_owned(),
            message: issue.message,
        }
    }
}

#[derive(Serialize)]
pub struct ScanResultDto {
    pub root: NodeDto,
    pub issues: Vec<ScanIssueDto>,
}

#[derive(Serialize)]
pub struct RootEntry {
    pub name: String,
    pub path: String,
}

#[tauri::command]
pub async fn scan_path(state: State<'_, ScanState>, path: String) -> Result<ScanResultDto, String> {
    let cancelled = state.cancelled.clone();
    cancelled.store(false, Ordering::SeqCst);

    let root = PathBuf::from(path);
    if !root.exists() {
        return Err(format!("{} does not exist", root.display()));
    }

    let result = tauri::async_runtime::spawn_blocking(move || {
        scan(&root, &|| cancelled.load(Ordering::SeqCst))
    })
    .await
    .map_err(|e| e.to_string())?;

    Ok(ScanResultDto {
        root: result.root.into(),
        issues: result.issues.into_iter().map(ScanIssueDto::from).collect(),
    })
}

#[tauri::command]
pub fn cancel_scan(state: State<'_, ScanState>) {
    state.cancelled.store(true, Ordering::SeqCst);
}

#[derive(Serialize)]
pub struct DeleteOutcome {
    pub deleted: Vec<String>,
    pub failed: Vec<DeleteFailure>,
}

#[derive(Serialize)]
pub struct DeleteFailure {
    pub path: String,
    pub message: String,
}

#[tauri::command]
pub async fn delete_paths(paths: Vec<String>) -> Result<DeleteOutcome, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let path_bufs: Vec<PathBuf> = paths.iter().map(PathBuf::from).collect();
        let refs: Vec<&Path> = path_bufs.iter().map(|p| p.as_path()).collect();
        let failures = move_all_to_trash(refs);

        let failed_paths: std::collections::HashSet<&Path> =
            failures.iter().map(|(p, _)| p.as_path()).collect();
        let deleted = path_bufs
            .iter()
            .filter(|p| !failed_paths.contains(p.as_path()))
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        let failed = failures
            .into_iter()
            .map(|(path, message)| DeleteFailure {
                path: path.to_string_lossy().into_owned(),
                message,
            })
            .collect();

        DeleteOutcome { deleted, failed }
    })
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn list_roots() -> Vec<RootEntry> {
    list_roots_impl()
}

#[cfg(target_os = "windows")]
fn list_roots_impl() -> Vec<RootEntry> {
    let mut roots = Vec::new();
    for c in b'A'..=b'Z' {
        let letter = c as char;
        let path = format!("{letter}:\\");
        if Path::new(&path).exists() {
            roots.push(RootEntry {
                name: format!("{letter}:\\"),
                path,
            });
        }
    }
    roots
}

#[cfg(not(target_os = "windows"))]
fn list_roots_impl() -> Vec<RootEntry> {
    let mut roots = vec![RootEntry {
        name: "Root (/)".to_string(),
        path: "/".to_string(),
    }];

    if let Some(home) = std::env::var_os("HOME").map(|h| h.to_string_lossy().into_owned()) {
        roots.push(RootEntry {
            name: "Home".to_string(),
            path: home,
        });
    }

    for base in ["/media", "/mnt"] {
        let Ok(entries) = std::fs::read_dir(base) else {
            continue;
        };
        for entry in entries.flatten() {
            if entry.path().is_dir() {
                roots.push(RootEntry {
                    name: entry.file_name().to_string_lossy().into_owned(),
                    path: entry.path().to_string_lossy().into_owned(),
                });
            }
        }
    }

    roots
}
