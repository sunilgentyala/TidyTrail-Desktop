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

    let result = scan(dir.path(), &|| false);

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

    let result = scan(dir.path(), &|| false);
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
    let result = scan(dir.path(), &|| false);
    assert_eq!(result.root.size, 0);
    assert!(result.root.children.is_empty());
}

#[test]
fn cancelling_immediately_still_returns_a_root_node() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("a.txt"), vec![0u8; 100]).unwrap();

    let result = scan(dir.path(), &|| true);

    assert_eq!(result.root.path, dir.path());
}
