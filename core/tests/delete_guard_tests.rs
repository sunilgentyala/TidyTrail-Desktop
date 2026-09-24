use std::fs;
use std::path::{Path, PathBuf};

use tempfile::tempdir;
use tidytrail_core::{DeleteGuard, DeleteRejection};

fn guard_for(root: &Path) -> DeleteGuard {
    DeleteGuard::with_protected(root, Vec::new(), Vec::new()).unwrap()
}

#[test]
fn allows_a_file_inside_the_scanned_folder() {
    let dir = tempdir().unwrap();
    let file = dir.path().join("big.iso");
    fs::write(&file, b"x").unwrap();

    let resolved = guard_for(dir.path()).authorize(&file).unwrap();
    assert_eq!(resolved.file_name().unwrap(), "big.iso");
}

#[test]
fn allows_a_nested_folder_inside_the_scanned_folder() {
    let dir = tempdir().unwrap();
    let nested = dir.path().join("a").join("b");
    fs::create_dir_all(&nested).unwrap();

    assert!(guard_for(dir.path()).authorize(&nested).is_ok());
}

#[test]
fn rejects_the_scanned_folder_itself() {
    let dir = tempdir().unwrap();
    let guard = guard_for(dir.path());
    assert_eq!(
        guard.authorize(dir.path()),
        Err(DeleteRejection::IsScanRoot)
    );
}

#[test]
fn rejects_a_path_outside_the_scanned_folder() {
    let outer = tempdir().unwrap();
    let scanned = outer.path().join("scanned");
    fs::create_dir(&scanned).unwrap();
    let sibling = outer.path().join("not-scanned.txt");
    fs::write(&sibling, b"x").unwrap();

    assert_eq!(
        guard_for(&scanned).authorize(&sibling),
        Err(DeleteRejection::OutsideScanRoot)
    );
}

/// `/data/app` must not authorize `/data/app-backup/...` just because the
/// string starts the same way: containment has to be checked per component.
#[test]
fn rejects_a_sibling_whose_name_shares_the_scan_root_prefix() {
    let outer = tempdir().unwrap();
    let scanned = outer.path().join("app");
    let lookalike = outer.path().join("app-backup");
    fs::create_dir(&scanned).unwrap();
    fs::create_dir(&lookalike).unwrap();
    let file = lookalike.join("db.sqlite");
    fs::write(&file, b"x").unwrap();

    assert_eq!(
        guard_for(&scanned).authorize(&file),
        Err(DeleteRejection::OutsideScanRoot)
    );
}

#[test]
fn rejects_parent_dir_traversal() {
    let outer = tempdir().unwrap();
    let scanned = outer.path().join("scanned");
    fs::create_dir(&scanned).unwrap();
    fs::write(outer.path().join("secret.txt"), b"x").unwrap();

    let sneaky = scanned.join("..").join("secret.txt");
    assert_eq!(
        guard_for(&scanned).authorize(&sneaky),
        Err(DeleteRejection::NonNormalPath)
    );
}

#[test]
fn rejects_relative_paths() {
    let dir = tempdir().unwrap();
    assert_eq!(
        guard_for(dir.path()).authorize(&PathBuf::from("relative.txt")),
        Err(DeleteRejection::NotAbsolute)
    );
}

#[test]
fn rejects_paths_that_do_not_exist() {
    let dir = tempdir().unwrap();
    let missing = dir.path().join("gone.txt");
    assert!(matches!(
        guard_for(dir.path()).authorize(&missing),
        Err(DeleteRejection::NotFound(_))
    ));
}

#[test]
fn rejects_anything_inside_a_protected_tree() {
    let root = tempdir().unwrap();
    let system = root.path().join("system");
    let deep = system.join("drivers");
    fs::create_dir_all(&deep).unwrap();
    let file = deep.join("disk.sys");
    fs::write(&file, b"x").unwrap();

    let guard = DeleteGuard::with_protected(root.path(), vec![system.clone()], Vec::new()).unwrap();
    assert!(matches!(
        guard.authorize(&file),
        Err(DeleteRejection::ProtectedLocation(_))
    ));
    assert!(matches!(
        guard.authorize(&system),
        Err(DeleteRejection::ProtectedLocation(_))
    ));
}

#[test]
fn protected_exact_path_blocks_only_the_folder_not_its_contents() {
    let root = tempdir().unwrap();
    let logs = root.path().join("log");
    fs::create_dir(&logs).unwrap();
    let old_log = logs.join("app.log.1");
    fs::write(&old_log, b"x").unwrap();

    let guard = DeleteGuard::with_protected(root.path(), Vec::new(), vec![logs.clone()]).unwrap();
    assert!(matches!(
        guard.authorize(&logs),
        Err(DeleteRejection::ProtectedLocation(_))
    ));
    assert!(guard.authorize(&old_log).is_ok());
}

/// Scanning the whole system volume must never make the OS directory
/// itself deletable.
#[test]
fn default_policy_protects_the_os_directory() {
    #[cfg(windows)]
    let (volume, os_dir) = {
        let drive = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into());
        let windir = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
        (PathBuf::from(format!(r"{drive}\")), PathBuf::from(windir))
    };
    #[cfg(not(windows))]
    let (volume, os_dir) = (PathBuf::from("/"), PathBuf::from("/usr"));

    let guard = DeleteGuard::new(&volume).unwrap();
    assert!(matches!(
        guard.authorize(&os_dir),
        Err(DeleteRejection::ProtectedLocation(_))
    ));
}

#[cfg(unix)]
#[test]
fn a_symlink_is_authorized_as_the_link_not_its_target() {
    let outer = tempdir().unwrap();
    let scanned = outer.path().join("scanned");
    fs::create_dir(&scanned).unwrap();
    let target = outer.path().join("outside.txt");
    fs::write(&target, b"x").unwrap();
    let link = scanned.join("link");
    std::os::unix::fs::symlink(&target, &link).unwrap();

    let resolved = guard_for(&scanned).authorize(&link).unwrap();
    assert_eq!(resolved.file_name().unwrap(), "link");
    assert!(fs::symlink_metadata(&resolved)
        .unwrap()
        .file_type()
        .is_symlink());
}

#[cfg(unix)]
#[test]
fn rejects_a_path_reached_through_a_symlinked_parent_that_escapes() {
    let outer = tempdir().unwrap();
    let scanned = outer.path().join("scanned");
    let elsewhere = outer.path().join("elsewhere");
    fs::create_dir(&scanned).unwrap();
    fs::create_dir(&elsewhere).unwrap();
    fs::write(elsewhere.join("victim.txt"), b"x").unwrap();
    std::os::unix::fs::symlink(&elsewhere, scanned.join("escape")).unwrap();

    let via_link = scanned.join("escape").join("victim.txt");
    assert_eq!(
        guard_for(&scanned).authorize(&via_link),
        Err(DeleteRejection::OutsideScanRoot)
    );
}
