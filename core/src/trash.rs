use std::path::Path;

/// Moves `path` to the OS trash/recycle bin rather than deleting it
/// outright, so a bad click in the treemap is recoverable the same way
/// TidyTrail's mobile app keeps a restorable trash instead of hard-deleting.
///
/// Backed by the `trash` crate: Recycle Bin on Windows, Trash on macOS, and
/// the freedesktop.org trash spec on Linux.
pub fn move_to_trash(path: &Path) -> Result<(), String> {
    trash::delete(path).map_err(|e| e.to_string())
}

/// Moves every path in `paths` to the OS trash, continuing past individual
/// failures (e.g. a file that was deleted or locked by another process
/// between scan and delete) and returning one error per path that failed.
pub fn move_all_to_trash<'a, I: IntoIterator<Item = &'a Path>>(
    paths: I,
) -> Vec<(std::path::PathBuf, String)> {
    let mut failures = Vec::new();
    for path in paths {
        if let Err(e) = move_to_trash(path) {
            failures.push((path.to_path_buf(), e));
        }
    }
    failures
}
