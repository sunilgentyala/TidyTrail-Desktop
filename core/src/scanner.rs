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

/// How many entries a scan visits between `on_progress` calls. High enough
/// that the callback (which on the Tauri side means an IPC event) never
/// becomes the bottleneck, low enough that a live scan still feels like
/// it's moving on a folder with thousands of files.
const PROGRESS_INTERVAL: u64 = 200;

/// Deepest directory nesting the scanner will descend into. Scanning,
/// converting and serializing the tree are all recursive, so without a cap a
/// directory tree nested a few thousand levels deep (easy for any local user
/// to create under a shared folder) could overflow the stack and crash the
/// app. Real-world trees are nowhere near this deep; anything past the cap
/// is reported as a scan issue instead of silently dropped.
pub const MAX_DEPTH: usize = 256;

/// One directory the scanner is part-way through reading.
struct DirFrame {
    path: PathBuf,
    dev: Option<u64>,
    depth: usize,
    entries: fs::ReadDir,
    children: Vec<Node>,
}

impl DirFrame {
    fn finish(mut self) -> Node {
        self.children.sort_by_key(|c| std::cmp::Reverse(c.size));
        let total: u64 = self.children.iter().map(|c| c.size).sum();
        Node {
            name: display_name(&self.path),
            path: self.path,
            size: total,
            is_dir: true,
            children: self.children,
        }
    }
}

/// What to do with one entry once its metadata is known.
enum Visit {
    Leaf(Node),
    Descend(Box<DirFrame>),
    Skip,
}

/// Recursively scans `root`, returning a size-sorted tree plus any paths
/// that could not be read. `is_cancelled` is polled between entries so a
/// scan of a large volume can be stopped from the UI without waiting for it
/// to finish. `on_progress` is called every [`PROGRESS_INTERVAL`] entries
/// with the running total visited, so a long scan can show live progress
/// instead of sitting on a static "Scanning..." message.
///
/// The walk uses an explicit stack rather than recursion, so no directory
/// layout, however deep, can overflow the thread's stack. (An earlier
/// recursive version crashed on a tree only ~260 levels deep in a debug
/// build.) [`MAX_DEPTH`] still bounds the *result* tree, because converting
/// and serializing it for the UI is recursive.
pub fn scan(root: &Path, is_cancelled: &dyn Fn() -> bool, on_progress: &dyn Fn(u64)) -> ScanResult {
    let mut issues = Vec::new();
    let mut visited: u64 = 0;

    if is_cancelled() {
        return ScanResult {
            root: leaf(root, 0, true),
            issues,
        };
    }

    let root_metadata = match fs::symlink_metadata(root) {
        Ok(m) => m,
        Err(e) => {
            issues.push(issue(root, e.to_string()));
            return ScanResult {
                root: leaf(root, 0, true),
                issues,
            };
        }
    };

    let mut stack: Vec<DirFrame> = match visit(
        root,
        root_metadata,
        None,
        0,
        &mut visited,
        on_progress,
        &mut issues,
    ) {
        Visit::Descend(frame) => vec![*frame],
        Visit::Leaf(node) => return ScanResult { root: node, issues },
        Visit::Skip => {
            return ScanResult {
                root: leaf(root, 0, true),
                issues,
            }
        }
    };

    loop {
        let cancelled = is_cancelled();
        let top = stack
            .last_mut()
            .expect("stack is never empty inside the loop");
        let next = if cancelled { None } else { top.entries.next() };

        let Some(entry) = next else {
            // This directory is done (or the scan was cancelled): fold it
            // into its parent, or return it if it was the root.
            let finished = stack.pop().expect("checked above").finish();
            match stack.last_mut() {
                Some(parent) => {
                    parent.children.push(finished);
                    continue;
                }
                None => {
                    return ScanResult {
                        root: finished,
                        issues,
                    }
                }
            }
        };

        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                issues.push(issue(&top.path, e.to_string()));
                continue;
            }
        };
        let path = entry.path();
        let metadata = match entry.metadata() {
            Ok(m) => m,
            Err(e) => {
                issues.push(issue(&path, e.to_string()));
                continue;
            }
        };
        let (parent_dev, depth) = (top.dev, top.depth + 1);
        match visit(
            &path,
            metadata,
            parent_dev,
            depth,
            &mut visited,
            on_progress,
            &mut issues,
        ) {
            Visit::Leaf(node) => stack.last_mut().expect("non-empty").children.push(node),
            Visit::Descend(frame) => stack.push(*frame),
            Visit::Skip => {}
        }
    }
}

/// Classifies one entry: a leaf to record, a directory to descend into, or
/// something to ignore. Everything that decides *whether* to descend lives
/// here, so the loop in [`scan`] only manages the stack.
fn visit(
    path: &Path,
    metadata: fs::Metadata,
    parent_dev: Option<u64>,
    depth: usize,
    visited: &mut u64,
    on_progress: &dyn Fn(u64),
    issues: &mut Vec<ScanIssue>,
) -> Visit {
    *visited += 1;
    if (*visited).is_multiple_of(PROGRESS_INTERVAL) {
        on_progress(*visited);
    }

    // Symlinks and (on Windows) junctions/mount points are reported as
    // zero-size leaves rather than followed, so a link that points back up
    // the tree can never turn a scan into an infinite loop. `is_symlink()`
    // alone is not enough on Windows: it only matches the SYMLINK reparse
    // tag, not the MOUNT_POINT tag junctions use, and self-referential
    // junctions such as `AppData\Local\Application Data` otherwise send the
    // scanner into unbounded recursion.
    if metadata.is_symlink() || is_reparse_point(&metadata) {
        return Visit::Leaf(leaf(path, 0, false));
    }

    if metadata.is_file() {
        return Visit::Leaf(leaf(path, metadata.len(), false));
    }

    if !metadata.is_dir() {
        return Visit::Skip;
    }

    // Kernel pseudo-filesystems (/proc, /sys, ...) report sizes that don't
    // correspond to anything on disk - /proc/kcore alone claims to be
    // ~128 TB - so a scan of `/` would otherwise be dominated by fake data.
    // Only checked where a directory sits on a different device than its
    // parent, i.e. at a mount point, so the common case costs nothing.
    let dev = device_id(&metadata);
    if depth > 0 && dev.is_some() && dev != parent_dev && is_virtual_filesystem(path) {
        issues.push(issue(
            path,
            "skipped: kernel virtual filesystem (sizes are not real disk usage)".to_string(),
        ));
        return Visit::Leaf(leaf(path, 0, true));
    }

    if depth >= MAX_DEPTH {
        issues.push(issue(
            path,
            format!("skipped: nested deeper than {MAX_DEPTH} levels"),
        ));
        return Visit::Leaf(leaf(path, 0, true));
    }

    match fs::read_dir(path) {
        Ok(entries) => Visit::Descend(Box::new(DirFrame {
            path: path.to_path_buf(),
            dev,
            depth,
            entries,
            children: Vec::new(),
        })),
        Err(e) => {
            issues.push(issue(path, e.to_string()));
            Visit::Leaf(leaf(path, 0, true))
        }
    }
}

fn issue(path: &Path, message: String) -> ScanIssue {
    ScanIssue {
        path: path.to_path_buf(),
        message,
    }
}

fn leaf(path: &Path, size: u64, is_dir: bool) -> Node {
    Node {
        name: display_name(path),
        path: path.to_path_buf(),
        size,
        is_dir,
        children: Vec::new(),
    }
}

#[cfg(unix)]
fn device_id(metadata: &fs::Metadata) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;
    Some(metadata.dev())
}

#[cfg(not(unix))]
fn device_id(_metadata: &fs::Metadata) -> Option<u64> {
    None
}

/// True if `path` is on a Linux kernel pseudo-filesystem, identified by its
/// `statfs` magic number rather than by name, so it also catches e.g. a
/// container's /proc bind-mounted somewhere unusual.
#[cfg(target_os = "linux")]
pub fn is_virtual_filesystem(path: &Path) -> bool {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    const PSEUDO_FS_MAGICS: &[i64] = &[
        0x9fa0,      // proc
        0x6265_6572, // sysfs
        0x1cd1,      // devpts
        0x6462_6720, // debugfs
        0x7472_6163, // tracefs
        0x7363_6673, // securityfs
        0x0027_e0eb, // cgroup
        0x6367_7270, // cgroup2
        0xcafe_4a11, // bpf
        0x6165_676c, // pstore
        0x6265_6570, // configfs
        0x6573_5546, // fusectl
        0x1980_0202, // mqueue
        0x4249_4e4d, // binfmt_misc
        0xde5e_81e4, // efivarfs
        0xf97c_ff8c, // selinuxfs
        0x5a3c_69f0, // apparmorfs
    ];

    let Ok(c_path) = CString::new(path.as_os_str().as_bytes()) else {
        return false;
    };
    // SAFETY: an all-zero `statfs` is a valid value for this plain C struct,
    // `c_path` is a valid NUL-terminated string, and `buf` is a properly
    // sized, writable `statfs` for the call to fill in.
    let mut buf: libc::statfs = unsafe { std::mem::zeroed() };
    let rc = unsafe { libc::statfs(c_path.as_ptr(), &mut buf) };
    if rc != 0 {
        return false;
    }
    #[allow(clippy::unnecessary_cast)]
    let magic = buf.f_type as i64;
    PSEUDO_FS_MAGICS.contains(&magic)
}

#[cfg(not(target_os = "linux"))]
pub fn is_virtual_filesystem(_path: &Path) -> bool {
    false
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

        let result = scan(dir.path(), &|| false, &|_| {});

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
