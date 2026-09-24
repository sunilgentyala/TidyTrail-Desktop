use std::cell::RefCell;
use std::fs;
use tempfile::tempdir;
use tidytrail_core::scan;

#[test]
fn sums_nested_directory_sizes() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("a.txt"), vec![0u8; 100]).unwrap();
    let sub = dir.path().join("sub");
    fs::create_dir(&sub).unwrap();
    fs::write(sub.join("b.txt"), vec![0u8; 250]).unwrap();

    let result = scan(dir.path(), &|| false, &|_| {});

    assert!(result.issues.is_empty());
    assert_eq!(result.root.size, 350);
    assert!(result.root.is_dir);

    let sub_node = result
        .root
        .children
        .iter()
        .find(|c| c.name == "sub")
        .unwrap();
    assert_eq!(sub_node.size, 250);
}

#[test]
fn children_are_sorted_largest_first() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("small.txt"), vec![0u8; 10]).unwrap();
    fs::write(dir.path().join("big.txt"), vec![0u8; 1000]).unwrap();
    fs::write(dir.path().join("medium.txt"), vec![0u8; 100]).unwrap();

    let result = scan(dir.path(), &|| false, &|_| {});
    let names: Vec<&str> = result
        .root
        .children
        .iter()
        .map(|c| c.name.as_str())
        .collect();

    assert_eq!(names, vec!["big.txt", "medium.txt", "small.txt"]);
}

#[test]
fn empty_directory_has_zero_size() {
    let dir = tempdir().unwrap();
    let result = scan(dir.path(), &|| false, &|_| {});
    assert_eq!(result.root.size, 0);
    assert!(result.root.children.is_empty());
}

#[test]
fn cancelling_immediately_still_returns_a_root_node() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("a.txt"), vec![0u8; 100]).unwrap();

    let result = scan(dir.path(), &|| true, &|_| {});

    assert_eq!(result.root.path, dir.path());
}

#[test]
fn on_progress_fires_with_a_running_total() {
    let dir = tempdir().unwrap();
    // Comfortably over PROGRESS_INTERVAL (200) so the callback is
    // guaranteed to fire at least once.
    for i in 0..450 {
        fs::write(dir.path().join(format!("f{i}.txt")), b"x").unwrap();
    }

    let calls: RefCell<Vec<u64>> = RefCell::new(Vec::new());
    let result = scan(dir.path(), &|| false, &|count| {
        calls.borrow_mut().push(count)
    });

    let calls = calls.into_inner();
    assert_eq!(result.root.children.len(), 450);
    assert!(!calls.is_empty(), "on_progress should fire at least once");
    assert!(
        calls.windows(2).all(|w| w[0] < w[1]),
        "counts should be strictly increasing: {calls:?}"
    );
    assert!(
        *calls.last().unwrap() <= 451,
        "should not overcount: {calls:?}"
    );
}

/// A directory tree nested far deeper than any real one (easy for a local
/// user to create under a shared folder) must not be able to exhaust the
/// stack; the scanner stops at `MAX_DEPTH` and says so.
#[test]
fn stops_descending_past_max_depth_and_reports_it() {
    use tidytrail_core::MAX_DEPTH;

    let dir = tempdir().unwrap();
    let mut deepest = dir.path().to_path_buf();
    for _ in 0..(MAX_DEPTH + 5) {
        deepest.push("d");
    }
    fs::create_dir_all(&deepest).unwrap();
    fs::write(deepest.join("hidden.bin"), vec![0u8; 42]).unwrap();

    let result = scan(dir.path(), &|| false, &|_| {});

    assert_eq!(
        result.root.size, 0,
        "content below the depth cap is not counted"
    );
    assert!(result
        .issues
        .iter()
        .any(|i| i.message.contains("nested deeper than")));
}

#[cfg(target_os = "linux")]
#[test]
fn proc_and_sys_are_virtual_filesystems_but_temp_dirs_are_not() {
    use tidytrail_core::is_virtual_filesystem;

    assert!(is_virtual_filesystem(std::path::Path::new("/proc")));
    assert!(is_virtual_filesystem(std::path::Path::new("/sys")));
    let dir = tempdir().unwrap();
    assert!(!is_virtual_filesystem(dir.path()));
}
