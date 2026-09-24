use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, Runtime, State};
use tidytrail_core::{
    categorize, format_bytes, scan, trash_with_audit, AuditLog, DeleteGuard, DeleteRejection, Node,
    ScanIssue, AUDIT_LOG_DIR_ENV, AUDIT_LOG_FILE_NAME, MAX_PATHS_PER_REQUEST,
};

#[derive(Serialize, Clone)]
pub struct ScanProgress {
    pub visited: u64,
}

/// What the backend remembers about the most recent scan. The UI only ever
/// runs one scan at a time, but a user can start a new scan before the old
/// one finishes, so each scan gets its own cancellation flag and generation
/// number: starting a scan cancels the previous one, and a superseded scan's
/// result is discarded instead of overwriting the newer one.
///
/// `guard` is set only once a scan completes, and is what `delete_paths`
/// checks every requested path against - the webview never gets to decide
/// on its own what may be deleted.
#[derive(Default)]
pub struct ScanState {
    current: Mutex<ActiveScan>,
}

#[derive(Default)]
struct ActiveScan {
    generation: u64,
    cancelled: Arc<AtomicBool>,
    guard: Option<DeleteGuard>,
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
pub async fn scan_path<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, ScanState>,
    path: String,
) -> Result<ScanResultDto, String> {
    let root = PathBuf::from(&path);
    if !root.is_absolute() {
        return Err(format!("{} is not an absolute path", root.display()));
    }
    if !root.is_dir() {
        return Err(format!("{} is not a folder that exists", root.display()));
    }

    let (generation, cancelled) = {
        let mut current = state.current.lock().map_err(|e| e.to_string())?;
        current.cancelled.store(true, Ordering::SeqCst);
        current.generation += 1;
        current.cancelled = Arc::new(AtomicBool::new(false));
        current.guard = None;
        (current.generation, current.cancelled.clone())
    };

    let scan_root = root.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        scan(
            &scan_root,
            &|| cancelled.load(Ordering::SeqCst),
            &|visited| {
                let _ = app.emit("scan://progress", ScanProgress { visited });
            },
        )
    })
    .await
    .map_err(|e| e.to_string())?;

    {
        let mut current = state.current.lock().map_err(|e| e.to_string())?;
        if current.generation != generation {
            return Err("scan was superseded by a newer scan".to_string());
        }
        current.guard = Some(DeleteGuard::new(&root).map_err(|e| e.to_string())?);
    }

    Ok(ScanResultDto {
        root: result.root.into(),
        issues: result.issues.into_iter().map(ScanIssueDto::from).collect(),
    })
}

#[tauri::command]
pub fn cancel_scan(state: State<'_, ScanState>) {
    if let Ok(current) = state.current.lock() {
        current.cancelled.store(true, Ordering::SeqCst);
    }
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
pub async fn delete_paths<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, ScanState>,
    paths: Vec<String>,
) -> Result<DeleteOutcome, String> {
    if paths.len() > MAX_PATHS_PER_REQUEST {
        return Err(format!(
            "refusing to delete more than {MAX_PATHS_PER_REQUEST} items in one request"
        ));
    }
    let guard = state
        .current
        .lock()
        .map_err(|e| e.to_string())?
        .guard
        .clone()
        .ok_or_else(|| DeleteRejection::NoActiveScan.to_string())?;

    // Fail closed: if the audit log can't be opened, nothing is deleted.
    let dir = audit_log_dir(&app)?;
    let mut log = AuditLog::open_in(&dir)
        .map_err(|e| format!("cannot open audit log in {}: {e}", dir.display()))?;

    let report = tauri::async_runtime::spawn_blocking(move || {
        trash_with_audit(&guard, &paths, &mut log, false)
    })
    .await
    .map_err(|e| e.to_string())??;

    Ok(DeleteOutcome {
        deleted: report.trashed,
        failed: report
            .failed
            .into_iter()
            .map(|(path, message)| DeleteFailure { path, message })
            .collect(),
    })
}

fn audit_log_dir<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, String> {
    match std::env::var_os(AUDIT_LOG_DIR_ENV) {
        Some(dir) if !dir.is_empty() => Ok(PathBuf::from(dir)),
        _ => app.path().app_log_dir().map_err(|e| e.to_string()),
    }
}

/// Where the deletion audit log lives, so the UI can tell the operator.
#[tauri::command]
pub fn audit_log_location<R: Runtime>(app: AppHandle<R>) -> Result<String, String> {
    audit_log_dir(&app).map(|d| d.join(AUDIT_LOG_FILE_NAME).to_string_lossy().into_owned())
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

/// IPC-level tests: these call the real command handlers through Tauri's
/// mock runtime exactly the way the webview does, including the way script
/// injected into a compromised webview could, and check that the Rust side
/// enforces the delete policy on its own.
#[cfg(test)]
mod ipc_tests {
    use super::*;
    use serde_json::{json, Value};
    use tauri::test::{get_ipc_response, mock_builder, mock_context, noop_assets, MockRuntime};
    use tauri::webview::InvokeRequest;
    use tauri::WebviewWindow;

    fn make_webview() -> (tauri::App<MockRuntime>, WebviewWindow<MockRuntime>) {
        let app = mock_builder()
            .manage(ScanState::default())
            .invoke_handler(tauri::generate_handler![
                scan_path,
                delete_paths,
                cancel_scan
            ])
            .build(mock_context(noop_assets()))
            .expect("mock app");
        let webview = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .expect("mock webview");
        (app, webview)
    }

    fn invoke(
        webview: &WebviewWindow<MockRuntime>,
        cmd: &str,
        body: Value,
    ) -> Result<Value, Value> {
        get_ipc_response(
            webview,
            InvokeRequest {
                cmd: cmd.into(),
                callback: tauri::ipc::CallbackFn(0),
                error: tauri::ipc::CallbackFn(1),
                url: if cfg!(windows) {
                    "http://tauri.localhost"
                } else {
                    "tauri://localhost"
                }
                .parse()
                .unwrap(),
                body: tauri::ipc::InvokeBody::Json(body),
                headers: Default::default(),
                invoke_key: tauri::test::INVOKE_KEY.to_string(),
            },
        )
        .map(|b| b.deserialize::<Value>().unwrap())
    }

    fn failures(outcome: &Value) -> Vec<String> {
        outcome["failed"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f["message"].as_str().unwrap().to_string())
            .collect()
    }

    fn p(path: &Path) -> String {
        path.to_string_lossy().into_owned()
    }

    /// One sequential test, because it sets the process-wide audit-log
    /// environment variable.
    #[test]
    fn backend_enforces_delete_policy_and_audits_every_attempt() {
        let tmp = tempfile::tempdir().unwrap();
        let logs = tmp.path().join("audit");
        std::env::set_var(AUDIT_LOG_DIR_ENV, &logs);

        let scanned = tmp.path().join("scanned");
        let outside = tmp.path().join("outside");
        std::fs::create_dir_all(scanned.join("sub")).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        let victim = outside.join("victim.txt");
        let junk = scanned.join("sub").join("junk.bin");
        std::fs::write(&victim, b"keep me").unwrap();
        std::fs::write(&junk, b"junk").unwrap();

        let (_app, webview) = make_webview();

        // Before any scan there is no scope at all: nothing may be deleted.
        let err = invoke(&webview, "delete_paths", json!({ "paths": [p(&victim)] })).unwrap_err();
        assert!(
            err.as_str().unwrap().contains("no folder has been scanned"),
            "{err}"
        );

        invoke(&webview, "scan_path", json!({ "path": p(&scanned) })).expect("scan");

        // Outside the scanned folder, `..` traversal, and the scan root
        // itself are all refused by the backend.
        let traversal = scanned.join("..").join("outside").join("victim.txt");
        let outcome = invoke(
            &webview,
            "delete_paths",
            json!({ "paths": [p(&victim), p(&traversal), p(&scanned)] }),
        )
        .unwrap();
        assert!(
            outcome["deleted"].as_array().unwrap().is_empty(),
            "{outcome}"
        );
        let messages = failures(&outcome);
        assert!(
            messages[0].contains("outside the scanned folder"),
            "{messages:?}"
        );
        assert!(messages[1].contains("'..'"), "{messages:?}");
        assert!(
            messages[2].contains("scanned folder itself"),
            "{messages:?}"
        );
        assert!(victim.exists(), "a refused path must never be touched");

        // A real file inside the scanned folder is moved to the trash.
        let outcome = invoke(&webview, "delete_paths", json!({ "paths": [p(&junk)] })).unwrap();
        assert_eq!(outcome["deleted"], json!([p(&junk)]), "{outcome}");
        assert!(!junk.exists());

        // Every attempt, refused or not, is in the audit log.
        let log = std::fs::read_to_string(logs.join("deletions.jsonl")).unwrap();
        let lines: Vec<Value> = log
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(lines.len(), 4, "{log}");
        assert_eq!(lines.iter().filter(|l| l["outcome"] == "failed").count(), 3);
        assert_eq!(lines[3]["outcome"], "trashed");

        // Oversized requests are rejected outright.
        let many: Vec<String> = (0..=MAX_PATHS_PER_REQUEST)
            .map(|i| format!("/x/{i}"))
            .collect();
        assert!(invoke(&webview, "delete_paths", json!({ "paths": many })).is_err());

        std::env::remove_var(AUDIT_LOG_DIR_ENV);
    }

    #[test]
    fn scan_rejects_relative_and_missing_paths() {
        let (_app, webview) = make_webview();
        assert!(invoke(&webview, "scan_path", json!({ "path": "relative/dir" })).is_err());
        let missing = std::env::temp_dir().join("tidytrail-definitely-missing-dir");
        assert!(invoke(&webview, "scan_path", json!({ "path": p(&missing) })).is_err());
    }
}
