use std::fs;

use serde_json::Value;
use tempfile::tempdir;
use tidytrail_core::{trash_with_audit, AuditLog, DeleteGuard, MAX_PATHS_PER_REQUEST};

fn read_log(log: &AuditLog) -> Vec<Value> {
    fs::read_to_string(log.path())
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

#[test]
fn dry_run_touches_nothing_but_records_what_would_happen() {
    let dir = tempdir().unwrap();
    let scanned = dir.path().join("scanned");
    fs::create_dir(&scanned).unwrap();
    let file = scanned.join("old.log");
    fs::write(&file, b"x").unwrap();
    let outside = dir.path().join("outside.txt");
    fs::write(&outside, b"x").unwrap();

    let guard = DeleteGuard::with_protected(&scanned, Vec::new(), Vec::new()).unwrap();
    let mut log = AuditLog::open_in(&dir.path().join("logs")).unwrap();
    let paths = vec![
        file.to_string_lossy().into_owned(),
        outside.to_string_lossy().into_owned(),
    ];

    let report = trash_with_audit(&guard, &paths, &mut log, true).unwrap();

    assert_eq!(report.trashed, vec![paths[0].clone()]);
    assert_eq!(report.failed.len(), 1);
    assert!(report.failed[0].1.contains("outside the scanned folder"));
    assert!(file.exists(), "a dry run must not move anything");

    let records = read_log(&log);
    assert_eq!(records.len(), 2);
    assert_eq!(records[0]["outcome"], "would-trash");
    assert_eq!(records[1]["outcome"], "failed");
    assert!(records[0]["user"].is_string());
}

#[test]
fn real_run_moves_allowed_paths_and_audits_each() {
    let dir = tempdir().unwrap();
    let scanned = dir.path().join("scanned");
    fs::create_dir(&scanned).unwrap();
    let file = scanned.join("big.iso");
    fs::write(&file, b"x").unwrap();

    let guard = DeleteGuard::with_protected(&scanned, Vec::new(), Vec::new()).unwrap();
    let mut log = AuditLog::open_in(&dir.path().join("logs")).unwrap();
    let paths = vec![file.to_string_lossy().into_owned()];

    let report = trash_with_audit(&guard, &paths, &mut log, false).unwrap();

    assert_eq!(report.trashed, paths);
    assert!(!file.exists());
    assert_eq!(read_log(&log)[0]["outcome"], "trashed");
}

#[test]
fn oversized_requests_are_rejected_before_anything_happens() {
    let dir = tempdir().unwrap();
    let guard = DeleteGuard::with_protected(dir.path(), Vec::new(), Vec::new()).unwrap();
    let mut log = AuditLog::open_in(&dir.path().join("logs")).unwrap();
    let paths: Vec<String> = (0..=MAX_PATHS_PER_REQUEST)
        .map(|i| format!("/x/{i}"))
        .collect();

    assert!(trash_with_audit(&guard, &paths, &mut log, false).is_err());
    assert!(read_log(&log).is_empty());
}

#[cfg(unix)]
#[test]
fn audit_log_is_owner_only_on_unix() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempdir().unwrap();
    let log = AuditLog::open_in(dir.path()).unwrap();
    let mode = fs::metadata(log.path()).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
}
