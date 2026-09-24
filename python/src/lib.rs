//! Python bindings for `tidytrail-core`, published on PyPI as `tidytrail`.
//!
//! Deliberately thin: every policy decision (what may be deleted, what is
//! audited, how a scan treats links and pseudo-filesystems) lives in the
//! shared, tested Rust core, so the Python API and the desktop app can
//! never disagree about what is safe.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use pyo3::exceptions::{
    PyFileNotFoundError, PyKeyboardInterrupt, PyNotADirectoryError, PyOSError, PyPermissionError,
    PyValueError,
};
use pyo3::prelude::*;
use tidytrail_core as core;

/// One file or directory from a scan. Directories carry the total size of
/// everything beneath them, and their children are sorted largest first.
#[pyclass(frozen, get_all, module = "tidytrail._native")]
struct Node {
    name: String,
    path: String,
    size: u64,
    is_dir: bool,
    category: String,
    children: Vec<Py<Node>>,
}

#[pymethods]
impl Node {
    fn __repr__(&self) -> String {
        format!(
            "Node(path={:?}, size={}, is_dir={}, children={})",
            self.path,
            self.size,
            if self.is_dir { "True" } else { "False" },
            self.children.len()
        )
    }

    fn __len__(&self) -> usize {
        self.children.len()
    }
}

/// A path the scan could not read (permission denied, removed mid-scan, a
/// skipped pseudo-filesystem, ...). Reported instead of failing the scan.
#[pyclass(frozen, get_all, module = "tidytrail._native")]
struct ScanIssue {
    path: String,
    message: String,
}

#[pymethods]
impl ScanIssue {
    fn __repr__(&self) -> String {
        format!(
            "ScanIssue(path={:?}, message={:?})",
            self.path, self.message
        )
    }
}

/// The result of [`scan`]: the root node plus every path that was skipped.
#[pyclass(frozen, get_all, module = "tidytrail._native")]
struct ScanResult {
    root: Py<Node>,
    issues: Vec<Py<ScanIssue>>,
}

#[pymethods]
impl ScanResult {
    fn __repr__(&self) -> String {
        let root = self.root.get();
        format!(
            "ScanResult(root={:?}, size={}, issues={})",
            root.path,
            root.size,
            self.issues.len()
        )
    }
}

fn to_py_node(py: Python<'_>, node: core::Node) -> PyResult<Py<Node>> {
    let category = core::categorize(&node.path, node.is_dir)
        .as_str()
        .to_string();
    let children = node
        .children
        .into_iter()
        .map(|c| to_py_node(py, c))
        .collect::<PyResult<Vec<_>>>()?;
    Py::new(
        py,
        Node {
            name: node.name,
            path: node.path.to_string_lossy().into_owned(),
            size: node.size,
            is_dir: node.is_dir,
            category,
            children,
        },
    )
}

fn absolute_dir(path: &Path) -> PyResult<PathBuf> {
    let absolute = std::path::absolute(path)
        .map_err(|e| PyOSError::new_err(format!("{}: {e}", path.display())))?;
    if !absolute.exists() {
        return Err(PyFileNotFoundError::new_err(format!(
            "{} does not exist",
            absolute.display()
        )));
    }
    if !absolute.is_dir() {
        return Err(PyNotADirectoryError::new_err(format!(
            "{} is not a directory",
            absolute.display()
        )));
    }
    Ok(absolute)
}

/// Recursively scans `path` and returns a size-sorted tree.
///
/// Symlinks and Windows junctions are never followed (they appear as
/// zero-size entries), Linux pseudo-filesystems such as /proc are skipped,
/// and unreadable paths are reported in `issues` instead of raising.
/// Releases the GIL while scanning; Ctrl+C raises KeyboardInterrupt.
#[pyfunction]
fn scan(py: Python<'_>, path: PathBuf) -> PyResult<ScanResult> {
    let root = absolute_dir(&path)?;
    let interrupted = AtomicBool::new(false);

    let result = py.detach(|| {
        core::scan(&root, &|| interrupted.load(Ordering::Relaxed), &|_| {
            // Called every few hundred entries: a cheap moment to let
            // Python deliver a pending Ctrl+C.
            Python::attach(|py| {
                if py.check_signals().is_err() {
                    interrupted.store(true, Ordering::Relaxed);
                }
            });
        })
    });

    if interrupted.load(Ordering::Relaxed) {
        return Err(PyKeyboardInterrupt::new_err("scan interrupted"));
    }

    let issues = result
        .issues
        .into_iter()
        .map(|i| {
            Py::new(
                py,
                ScanIssue {
                    path: i.path.to_string_lossy().into_owned(),
                    message: i.message,
                },
            )
        })
        .collect::<PyResult<Vec<_>>>()?;
    Ok(ScanResult {
        root: to_py_node(py, result.root)?,
        issues,
    })
}

/// What a [`Cleaner::trash`] call did.
#[pyclass(frozen, get_all, module = "tidytrail._native")]
struct TrashReport {
    /// Paths moved to the trash (in a dry run: that would have been).
    trashed: Vec<String>,
    /// `(path, reason)` for every path refused by policy or that failed.
    failed: Vec<(String, String)>,
    dry_run: bool,
    audit_log: String,
}

#[pymethods]
impl TrashReport {
    /// True when every requested path was trashed (or would be).
    #[getter]
    fn ok(&self) -> bool {
        self.failed.is_empty()
    }

    fn __repr__(&self) -> String {
        format!(
            "TrashReport(trashed={}, failed={}, dry_run={})",
            self.trashed.len(),
            self.failed.len(),
            if self.dry_run { "True" } else { "False" }
        )
    }
}

/// Moves files to the OS trash, but only inside `scan_root`, never
/// protected system locations, and always with an audit record.
///
/// Every path is re-validated in Rust: it must resolve (through its real
/// parent directory, so symlinks can't escape) strictly inside `scan_root`,
/// contain no `..`, and not be an OS directory, volume root, or profile
/// root. Every attempt - trashed, refused, failed, or dry run - is appended
/// to `deletions.jsonl` in `audit_log_dir`. If the log can't be written,
/// nothing is deleted.
#[pyclass(subclass, module = "tidytrail._native")]
struct Cleaner {
    guard: core::DeleteGuard,
    audit_log_dir: PathBuf,
}

#[pymethods]
impl Cleaner {
    #[new]
    fn new(scan_root: PathBuf, audit_log_dir: PathBuf) -> PyResult<Self> {
        let root = absolute_dir(&scan_root)?;
        let guard = core::DeleteGuard::new(&root)
            .map_err(|e| PyOSError::new_err(format!("{}: {e}", root.display())))?;
        let audit_log_dir = std::path::absolute(&audit_log_dir)
            .map_err(|e| PyOSError::new_err(format!("{}: {e}", audit_log_dir.display())))?;
        Ok(Cleaner {
            guard,
            audit_log_dir,
        })
    }

    /// The scan root after resolving symlinks.
    #[getter]
    fn scan_root(&self) -> String {
        self.guard.scan_root().to_string_lossy().into_owned()
    }

    /// Full path of the audit log this cleaner writes to.
    #[getter]
    fn audit_log(&self) -> String {
        self.audit_log_dir
            .join(core::AUDIT_LOG_FILE_NAME)
            .to_string_lossy()
            .into_owned()
    }

    /// Returns the resolved path that `trash()` would move, or raises
    /// PermissionError (refused by policy) / FileNotFoundError. Touches
    /// nothing and writes no audit record.
    fn check(&self, path: PathBuf) -> PyResult<String> {
        match self.guard.authorize(&path) {
            Ok(resolved) => Ok(resolved.to_string_lossy().into_owned()),
            Err(core::DeleteRejection::NotFound(e)) => Err(PyFileNotFoundError::new_err(format!(
                "{}: {e}",
                path.display()
            ))),
            Err(rejection) => Err(PyPermissionError::new_err(format!(
                "{}: {rejection}",
                path.display()
            ))),
        }
    }

    /// Moves each path to the OS trash if policy allows it, recording every
    /// attempt in the audit log. Never raises for an individual path; see
    /// `TrashReport.failed`. With `dry_run=True` nothing is moved, but the
    /// would-be outcome is still checked and audited.
    #[pyo3(signature = (paths, *, dry_run = false))]
    fn trash(&self, py: Python<'_>, paths: Vec<PathBuf>, dry_run: bool) -> PyResult<TrashReport> {
        let paths: Vec<String> = paths
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        let mut log = core::AuditLog::open_in(&self.audit_log_dir).map_err(|e| {
            PyOSError::new_err(format!(
                "cannot open audit log in {} (nothing was deleted): {e}",
                self.audit_log_dir.display()
            ))
        })?;
        let audit_log = log.path().to_string_lossy().into_owned();
        let guard = &self.guard;
        let report = py
            .detach(|| core::trash_with_audit(guard, &paths, &mut log, dry_run))
            .map_err(PyValueError::new_err)?;
        Ok(TrashReport {
            trashed: report.trashed,
            failed: report.failed,
            dry_run,
            audit_log,
        })
    }

    fn __repr__(&self) -> String {
        format!("Cleaner(scan_root={:?})", self.scan_root())
    }
}

/// Formats a byte count the same way the desktop app does ("1.5 GB").
#[pyfunction]
fn format_bytes(size: u64) -> String {
    core::format_bytes(size)
}

#[pymodule]
fn _native(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(scan, m)?)?;
    m.add_function(wrap_pyfunction!(format_bytes, m)?)?;
    m.add_class::<Node>()?;
    m.add_class::<ScanIssue>()?;
    m.add_class::<ScanResult>()?;
    m.add_class::<Cleaner>()?;
    m.add_class::<TrashReport>()?;
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    m.add("MAX_DEPTH", core::MAX_DEPTH)?;
    m.add("MAX_PATHS_PER_REQUEST", core::MAX_PATHS_PER_REQUEST)?;
    m.add("AUDIT_LOG_DIR_ENV", core::AUDIT_LOG_DIR_ENV)?;
    m.add("AUDIT_LOG_FILE_NAME", core::AUDIT_LOG_FILE_NAME)?;
    Ok(())
}
