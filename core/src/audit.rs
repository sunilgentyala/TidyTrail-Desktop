use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

use crate::{move_to_trash, DeleteGuard};

/// File name of the deletion audit log inside its folder.
pub const AUDIT_LOG_FILE_NAME: &str = "deletions.jsonl";

/// Overrides where the deletion audit log is written, e.g. a folder a log
/// shipper already collects from on a managed server. Honored by both the
/// desktop app and the Python package / CLI.
pub const AUDIT_LOG_DIR_ENV: &str = "TIDYTRAIL_AUDIT_LOG_DIR";

/// Upper bound on paths per trash request, so a runaway or malicious caller
/// can't hand the backend an unbounded list.
pub const MAX_PATHS_PER_REQUEST: usize = 10_000;

/// An append-only JSON Lines log of every delete attempt - allowed, refused,
/// failed, or only simulated - so an operator can always answer "what did
/// this tool remove, when, and as whom".
pub struct AuditLog {
    file: File,
    path: PathBuf,
    user: String,
}

#[derive(Serialize)]
struct AuditRecord<'a> {
    unix_time: u64,
    user: &'a str,
    scan_root: &'a str,
    requested: &'a str,
    resolved: Option<&'a str>,
    outcome: &'a str,
    detail: Option<&'a str>,
}

impl AuditLog {
    /// Opens (creating if needed) `dir/deletions.jsonl` for appending. On
    /// Unix a newly created log is readable by its owner only, since it
    /// lists paths from wherever the tool was pointed.
    pub fn open_in(dir: &Path) -> io::Result<Self> {
        std::fs::create_dir_all(dir)?;
        let path = dir.join(AUDIT_LOG_FILE_NAME);
        let mut options = OpenOptions::new();
        options.create(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options.open(&path)?;
        Ok(AuditLog {
            file,
            path,
            user: current_user(),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn record(
        &mut self,
        scan_root: &str,
        requested: &str,
        resolved: Option<&str>,
        outcome: &str,
        detail: Option<&str>,
    ) -> io::Result<()> {
        let record = AuditRecord {
            unix_time: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
            user: &self.user,
            scan_root,
            requested,
            resolved,
            outcome,
            detail,
        };
        let line = serde_json::to_string(&record).map_err(io::Error::other)?;
        writeln!(self.file, "{line}")?;
        self.file.flush()
    }
}

fn current_user() -> String {
    std::env::var("USERNAME")
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_else(|_| "unknown".to_string())
}

/// What happened to each requested path.
#[derive(Debug, Default, Clone, Serialize)]
pub struct TrashReport {
    /// Paths moved to the trash (or, in a dry run, that would have been).
    pub trashed: Vec<String>,
    /// Paths refused by the policy or that failed to move, with the reason.
    pub failed: Vec<(String, String)>,
}

/// Authorizes every path against `guard`, moves the allowed ones to the OS
/// trash (unless `dry_run`), and writes one audit record per path before
/// moving on to the next. This is the single implementation behind both the
/// desktop app's delete button and the Python `Cleaner.trash()` / CLI.
///
/// Fails closed: if an audit record can't be written, the remaining paths
/// are not touched and are reported as failed.
pub fn trash_with_audit(
    guard: &DeleteGuard,
    paths: &[String],
    log: &mut AuditLog,
    dry_run: bool,
) -> Result<TrashReport, String> {
    if paths.len() > MAX_PATHS_PER_REQUEST {
        return Err(format!(
            "refusing to trash more than {MAX_PATHS_PER_REQUEST} items in one request"
        ));
    }
    let scan_root = guard.scan_root().to_string_lossy().into_owned();
    let mut report = TrashReport::default();

    for (index, requested) in paths.iter().enumerate() {
        let (resolved, result) = match guard.authorize(Path::new(requested)) {
            Ok(resolved) => {
                let result = if dry_run {
                    Ok(())
                } else {
                    move_to_trash(&resolved)
                };
                (Some(resolved), result)
            }
            Err(rejection) => (None, Err(format!("refused: {rejection}"))),
        };
        let resolved_str = resolved.as_ref().map(|p| p.to_string_lossy().into_owned());
        let outcome = match (&result, dry_run) {
            (Ok(()), true) => "would-trash",
            (Ok(()), false) => "trashed",
            (Err(_), _) => "failed",
        };

        if let Err(e) = log.record(
            &scan_root,
            requested,
            resolved_str.as_deref(),
            outcome,
            result.as_ref().err().map(String::as_str),
        ) {
            // The move (if any) already happened, so report it truthfully,
            // then stop: nothing else may be deleted without an audit trail.
            match result {
                Ok(()) => report.trashed.push(requested.clone()),
                Err(message) => report.failed.push((requested.clone(), message)),
            }
            for rest in &paths[index + 1..] {
                report.failed.push((
                    rest.clone(),
                    format!("not attempted: audit log write failed: {e}"),
                ));
            }
            return Ok(report);
        }

        match result {
            Ok(()) => report.trashed.push(requested.clone()),
            Err(message) => report.failed.push((requested.clone(), message)),
        }
    }
    Ok(report)
}
