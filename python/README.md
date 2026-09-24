# tidytrail

Fast, safe disk-usage scanning and audited cleanup, from Python or the
command line. Built for servers and automation: no GUI, no desktop session
needed.

`tidytrail` is powered by the same Rust core as
[TidyTrail Desktop](https://github.com/sunilgentyala/TidyTrail-Desktop), so
what it considers safe to delete is identical whether you click, script, or
run a cron job.

```bash
pip install tidytrail
```

Prebuilt wheels for Windows, Linux (x86_64, aarch64) and macOS (Intel and
Apple Silicon); Python 3.10+.

## Command line

```bash
# What's using the space?
tidytrail scan /var/log
tidytrail scan /data --dirs --top 10      # include folders
tidytrail scan /data --json               # for scripts / monitoring

# Preview a cleanup: checked and audited, nothing is moved
tidytrail trash --root /var/log --dry-run /var/log/app/old.log.1

# Do it (asks for confirmation when run interactively)
tidytrail trash --root /var/log /var/log/app/old.log.1

# Unattended (cron/CI): --yes is required, paths can come from a file or stdin
find /var/log/app -name '*.log.*' -mtime +30 \
  | tidytrail trash --root /var/log/app --yes --from-file -
```

Exit codes: `0` success, `1` some paths were refused or failed, `2` usage
error or nothing done, `130` interrupted.

## Python

```python
import tidytrail

result = tidytrail.scan("/var/log")           # GIL released; Ctrl+C works
print(tidytrail.format_bytes(result.root.size))
for node in tidytrail.largest(result.root, 10):
    print(tidytrail.format_bytes(node.size), node.path)

cleaner = tidytrail.Cleaner("/var/log/app")   # only paths inside this folder
cleaner.check("/var/log/app/old.log.1")       # PermissionError if refused
report = cleaner.trash(["/var/log/app/old.log.1"], dry_run=True)
print(report.trashed, report.failed, report.audit_log)
```

## Safety model

- **Deletes go to the OS trash** (Recycle Bin / freedesktop Trash), never a
  hard delete. Note that space is only freed when the trash is emptied.
- **Only inside the root you name.** Every path is re-checked in Rust: it
  must resolve, through its real parent directory, strictly inside `--root`
  / `scan_root`, so `..` and symlinked parents can't escape.
- **Protected locations are never deleted**, even inside the root: OS
  directories (`C:\Windows`, `/usr`, `/etc`, `/boot`, `/var/lib`, ...) and
  everything in them, plus volume roots, `Program Files`, profile roots,
  `/home`, `/var`, `/var/log`, `/opt`, `/srv` themselves.
- **Every attempt is audited** (trashed, refused, failed, or dry run) as
  JSON Lines in `deletions.jsonl`: `$TIDYTRAIL_AUDIT_LOG_DIR`, else
  `%LOCALAPPDATA%\tidytrail\logs` (Windows), `~/Library/Logs/tidytrail`
  (macOS), `~/.local/state/tidytrail` (Linux). If it can't be written,
  nothing is deleted. On Unix the log is created owner-readable only.
- **Scans never follow symlinks or junctions**, skip Linux pseudo-filesystems
  like `/proc`, and report unreadable paths instead of failing.

Run it as the least-privileged account that can see the data. See
[SECURITY.md](https://github.com/sunilgentyala/TidyTrail-Desktop/blob/main/SECURITY.md)
for the full threat model and how to report a vulnerability.

## Verifying what you install

Releases are published from GitHub Actions with PyPI Trusted Publishing, so
each file carries a signed [PEP 740](https://peps.python.org/pep-0740/)
attestation linking it to the exact workflow run that built it (shown on the
PyPI file page as "Verified").

## License

MIT
