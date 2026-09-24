use std::path::{Component, Path, PathBuf};

/// Decides whether a path the UI asked to delete is actually allowed to be
/// moved to the trash.
///
/// The webview is treated as untrusted: it only ever *requests* a delete,
/// and every requested path is re-checked here on the Rust side before
/// anything on disk is touched. A path is allowed only if it:
///
/// - is absolute and has no `.`/`..` components,
/// - resolves (via its canonicalized parent, so a symlink target is never
///   followed) to somewhere strictly *inside* the folder that was scanned,
///   never the scanned folder itself,
/// - is not a protected system location (an OS directory, a drive/volume
///   root, a user-profile root, ...) or anything inside a protected
///   operating-system tree.
///
/// That keeps a compromised or buggy frontend from trashing arbitrary files,
/// and keeps an operator from accidentally trashing something like
/// `C:\Windows` or `/usr` because a scan of `C:\` or `/` put it on screen.
#[derive(Debug, Clone)]
pub struct DeleteGuard {
    scan_root: PathBuf,
    /// Deleting these paths themselves, *or anything beneath them*, is refused.
    protected_trees: Vec<PathBuf>,
    /// Deleting these exact paths is refused, but their contents may be
    /// cleaned (e.g. files under `/var/log` or `C:\Users\<name>\Downloads`).
    protected_exact: Vec<PathBuf>,
}

/// Why a delete request was refused. Kept as plain strings on the wire,
/// but a real enum here so tests can assert the exact reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeleteRejection {
    NoActiveScan,
    NotAbsolute,
    NonNormalPath,
    NotFound(String),
    OutsideScanRoot,
    IsScanRoot,
    ProtectedLocation(PathBuf),
}

impl std::fmt::Display for DeleteRejection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DeleteRejection::NoActiveScan => write!(f, "no folder has been scanned yet"),
            DeleteRejection::NotAbsolute => write!(f, "path is not absolute"),
            DeleteRejection::NonNormalPath => {
                write!(f, "path contains '.' or '..' components")
            }
            DeleteRejection::NotFound(e) => write!(f, "path could not be resolved: {e}"),
            DeleteRejection::OutsideScanRoot => {
                write!(f, "path is outside the scanned folder")
            }
            DeleteRejection::IsScanRoot => {
                write!(f, "refusing to delete the scanned folder itself")
            }
            DeleteRejection::ProtectedLocation(p) => {
                write!(f, "{} is a protected system location", p.display())
            }
        }
    }
}

impl DeleteGuard {
    /// Builds a guard for a scan of `scan_root`, with the platform's default
    /// protected locations.
    pub fn new(scan_root: &Path) -> std::io::Result<Self> {
        let (trees, exact) = default_protected_paths();
        Self::with_protected(scan_root, trees, exact)
    }

    /// Builds a guard with an explicit protected-path list, so tests can
    /// exercise the policy against temp directories.
    pub fn with_protected(
        scan_root: &Path,
        protected_trees: Vec<PathBuf>,
        protected_exact: Vec<PathBuf>,
    ) -> std::io::Result<Self> {
        let scan_root = normalize(&std::fs::canonicalize(scan_root)?);
        // Protected paths that don't exist on this machine can't be
        // canonicalized; comparing against their literal form is still
        // correct for anything that could later be created there.
        let canon = |p: PathBuf| {
            std::fs::canonicalize(&p)
                .map(|c| normalize(&c))
                .unwrap_or(p)
        };
        Ok(DeleteGuard {
            scan_root,
            protected_trees: protected_trees.into_iter().map(canon).collect(),
            protected_exact: protected_exact.into_iter().map(canon).collect(),
        })
    }

    pub fn scan_root(&self) -> &Path {
        &self.scan_root
    }

    /// Validates `requested` and returns the resolved path to trash. The
    /// returned path has a canonical parent but keeps the entry's own final
    /// component unresolved, so if the entry is itself a symlink or junction
    /// the link is trashed rather than whatever it points at.
    pub fn authorize(&self, requested: &Path) -> Result<PathBuf, DeleteRejection> {
        if !requested.is_absolute() {
            return Err(DeleteRejection::NotAbsolute);
        }
        if requested
            .components()
            .any(|c| matches!(c, Component::CurDir | Component::ParentDir))
        {
            return Err(DeleteRejection::NonNormalPath);
        }

        let name = requested.file_name().ok_or(DeleteRejection::IsScanRoot)?;
        let parent = requested.parent().ok_or(DeleteRejection::IsScanRoot)?;

        // The entry itself must exist (as a link, if it is one).
        std::fs::symlink_metadata(requested)
            .map_err(|e| DeleteRejection::NotFound(e.to_string()))?;
        let parent =
            std::fs::canonicalize(parent).map_err(|e| DeleteRejection::NotFound(e.to_string()))?;
        let resolved = normalize(&parent).join(name);

        if resolved == self.scan_root {
            return Err(DeleteRejection::IsScanRoot);
        }
        if !path_starts_with(&resolved, &self.scan_root) {
            return Err(DeleteRejection::OutsideScanRoot);
        }
        for tree in &self.protected_trees {
            if path_starts_with(&resolved, tree) {
                return Err(DeleteRejection::ProtectedLocation(tree.clone()));
            }
        }
        for exact in &self.protected_exact {
            if path_eq(&resolved, exact) {
                return Err(DeleteRejection::ProtectedLocation(exact.clone()));
            }
        }
        if is_volume_root(&resolved) {
            return Err(DeleteRejection::ProtectedLocation(resolved));
        }
        Ok(resolved)
    }
}

fn is_volume_root(path: &Path) -> bool {
    path.parent().is_none()
}

/// `std::fs::canonicalize` on Windows returns verbatim `\\?\C:\...` paths,
/// which some shell APIs (including the Recycle Bin one the `trash` crate
/// uses) reject. Strip the verbatim prefix back off for ordinary drive
/// paths; leave anything that genuinely needs it (UNC, very long paths)
/// alone.
fn normalize(path: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        let s = path.as_os_str().to_string_lossy();
        if let Some(rest) = s.strip_prefix(r"\\?\") {
            let b = rest.as_bytes();
            if b.len() >= 3
                && b[0].is_ascii_alphabetic()
                && b[1] == b':'
                && b[2] == b'\\'
                && rest.len() < 260
            {
                return PathBuf::from(rest);
            }
        }
    }
    path.to_path_buf()
}

/// Windows paths are case-insensitive; Unix paths are not.
fn path_eq(a: &Path, b: &Path) -> bool {
    #[cfg(windows)]
    {
        a.as_os_str().to_string_lossy().to_lowercase()
            == b.as_os_str().to_string_lossy().to_lowercase()
    }
    #[cfg(not(windows))]
    {
        a == b
    }
}

fn path_starts_with(path: &Path, base: &Path) -> bool {
    #[cfg(windows)]
    {
        let lower = |p: &Path| PathBuf::from(p.as_os_str().to_string_lossy().to_lowercase());
        lower(path).starts_with(lower(base))
    }
    #[cfg(not(windows))]
    {
        path.starts_with(base)
    }
}

#[cfg(windows)]
fn default_protected_paths() -> (Vec<PathBuf>, Vec<PathBuf>) {
    let env = |k: &str, fallback: &str| {
        std::env::var_os(k)
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(fallback))
    };
    let system_drive = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".to_string());
    let on_sys = |rest: &str| PathBuf::from(format!(r"{system_drive}\{rest}"));

    let trees = vec![
        env("SystemRoot", r"C:\Windows"),
        on_sys("System Volume Information"),
        on_sys("$Recycle.Bin"),
        on_sys("Recovery"),
        on_sys("Boot"),
        on_sys("EFI"),
        on_sys("pagefile.sys"),
        on_sys("hiberfil.sys"),
        on_sys("swapfile.sys"),
    ];
    let mut exact = vec![
        env("ProgramFiles", r"C:\Program Files"),
        env("ProgramFiles(x86)", r"C:\Program Files (x86)"),
        env("ProgramData", r"C:\ProgramData"),
        on_sys("Users"),
        on_sys(r"Users\Public"),
        on_sys(r"Users\Default"),
    ];
    if let Some(profile) = std::env::var_os("USERPROFILE") {
        let profile = PathBuf::from(profile);
        for sub in [
            "",
            "AppData",
            r"AppData\Local",
            r"AppData\Roaming",
            "Desktop",
            "Documents",
            "Downloads",
        ] {
            exact.push(if sub.is_empty() {
                profile.clone()
            } else {
                profile.join(sub)
            });
        }
    }
    (trees, exact)
}

#[cfg(not(windows))]
fn default_protected_paths() -> (Vec<PathBuf>, Vec<PathBuf>) {
    let trees = [
        "/bin",
        "/boot",
        "/dev",
        "/etc",
        "/lib",
        "/lib32",
        "/lib64",
        "/libx32",
        "/proc",
        "/run",
        "/sbin",
        "/sys",
        "/usr",
        "/var/lib",
        "/var/spool",
        "/snap",
        "/efi",
        // macOS system trees, in case the core is ever reused there.
        "/System",
        "/Library",
        "/private/etc",
        "/private/var/db",
    ]
    .iter()
    .map(PathBuf::from)
    .collect();

    let mut exact: Vec<PathBuf> = [
        "/home",
        "/root",
        "/var",
        "/var/log",
        "/var/cache",
        "/var/tmp",
        "/opt",
        "/srv",
        "/tmp",
        "/mnt",
        "/media",
        "/Users",
        "/Applications",
    ]
    .iter()
    .map(PathBuf::from)
    .collect();
    if let Some(home) = std::env::var_os("HOME") {
        exact.push(PathBuf::from(home));
    }
    (trees, exact)
}
