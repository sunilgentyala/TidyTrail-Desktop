use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

/// A single file or directory discovered by a scan. Directories carry the
/// summed size of everything beneath them and their children sorted largest
/// first, so both the treemap layout and a plain sorted list can consume the
/// same tree without re-sorting.
#[derive(Debug, Clone, Serialize)]
pub struct Node {
    pub name: String,
    pub path: PathBuf,
    pub size: u64,
    pub is_dir: bool,
    pub children: Vec<Node>,
}

/// A path the scanner could not read (permission denied, broken symlink,
/// removed mid-scan, ...). Reported back instead of failing the whole scan,
/// since a single locked file should not hide the size of everything else
/// TidyTrail *could* read.
#[derive(Debug, Clone, Serialize)]
pub struct ScanIssue {
    pub path: PathBuf,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScanResult {
    pub root: Node,
    pub issues: Vec<ScanIssue>,
}

/// Recursively scans `root`, returning a size-sorted tree plus any paths
/// that could not be read. `is_cancelled` is polled between entries so a
/// scan of a large volume can be stopped from the UI without waiting for it
/// to finish.
pub fn scan(root: &Path, is_cancelled: &dyn Fn() -> bool) -> ScanResult {
    let mut issues = Vec::new();
    let node = scan_entry(root, is_cancelled, &mut issues);
    let root_node = node.unwrap_or_else(|| Node {
        name: display_name(root),
        path: root.to_path_buf(),
        size: 0,
        is_dir: true,
        children: Vec::new(),
    });
    ScanResult {
        root: root_node,
        issues,
    }
}

fn scan_entry(
    path: &Path,
    is_cancelled: &dyn Fn() -> bool,
    issues: &mut Vec<ScanIssue>,
) -> Option<Node> {
    if is_cancelled() {
        return None;
    }

    let metadata = match fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) => {
            issues.push(ScanIssue {
                path: path.to_path_buf(),
                message: e.to_string(),
            });
            return None;
        }
    };

    // Symlinks and (on Windows) junctions/mount points are reported as
    // zero-size leaves rather than followed, so a link that points back up
    // the tree can never turn a scan into an infinite loop. `is_symlink()`
    // alone is not enough on Windows: it only matches the SYMLINK reparse
    // tag, not the MOUNT_POINT tag junctions use, and self-referential
    // junctions such as `AppData\Local\Application Data` otherwise send the
    // scanner into unbounded recursion.
    if metadata.is_symlink() || is_reparse_point(&metadata) {
        return Some(Node {
            name: display_name(path),
            path: path.to_path_buf(),
            size: 0,
            is_dir: false,
            children: Vec::new(),
        });
    }

    if metadata.is_file() {
        return Some(Node {
            name: display_name(path),
            path: path.to_path_buf(),
            size: metadata.len(),
            is_dir: false,
            children: Vec::new(),
        });
    }

    if !metadata.is_dir() {
        return None;
    }

    let entries = match fs::read_dir(path) {
        Ok(e) => e,
        Err(e) => {
            issues.push(ScanIssue {
                path: path.to_path_buf(),
                message: e.to_string(),
            });
            return Some(Node {
                name: display_name(path),
                path: path.to_path_buf(),
                size: 0,
                is_dir: true,
                children: Vec::new(),
            });
        }
    };

    let mut children = Vec::new();
    for entry in entries {
        if is_cancelled() {
            break;
        }
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                issues.push(ScanIssue {
                    path: path.to_path_buf(),
                    message: e.to_string(),
                });
                continue;
            }
        };
        if let Some(child) = scan_entry(&entry.path(), is_cancelled, issues) {
            children.push(child);
        }
    }

    children.sort_by_key(|c| std::cmp::Reverse(c.size));
    let total: u64 = children.iter().map(|c| c.size).sum();

    Some(Node {
        name: display_name(path),
        path: path.to_path_buf(),
        size: total,
        is_dir: true,
        children,
    })
}

#[cfg(windows)]
fn is_reparse_point(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn is_reparse_point(_metadata: &fs::Metadata) -> bool {
    false
}

#[cfg(all(test, windows))]
mod windows_junction_tests {
    use super::*;
    use std::process::Command;

    fn make_junction(link: &Path, target: &Path) {
        let status = Command::new("cmd")
            .args([
                "/C",
                "mklink",
                "/J",
                link.to_str().unwrap(),
                target.to_str().unwrap(),
            ])
            .status()
            .expect("mklink should run");
        assert!(status.success(), "mklink /J failed");
    }

    #[test]
    fn junctions_are_detected_as_reparse_points() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target");
        fs::create_dir(&target).unwrap();
        let link = dir.path().join("link");
        make_junction(&link, &target);

        let metadata = fs::symlink_metadata(&link).unwrap();
        assert!(is_reparse_point(&metadata));
    }

    /// A junction that points back at its own parent directory is the
    /// real-world shape that hung the scanner before this fix (Windows
    /// ships several, e.g. `AppData\Local\Application Data`). Regression
    /// test for that: if junctions were ever followed again, this would
    /// recurse forever instead of returning.
    #[test]
    fn self_referential_junction_does_not_recurse() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), vec![0u8; 100]).unwrap();
        let link = dir.path().join("self_link");
        make_junction(&link, dir.path());

        let result = scan(dir.path(), &|| false);

        assert_eq!(
            result.root.size, 100,
            "the junction itself must be a zero-size leaf"
        );
        let link_node = result
            .root
            .children
            .iter()
            .find(|c| c.name == "self_link")
            .unwrap();
        assert!(!link_node.is_dir);
        assert!(link_node.children.is_empty());
    }
}

fn display_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}
