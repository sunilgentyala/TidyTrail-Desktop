# Security Policy

## Reporting a vulnerability

Please report security issues privately through GitHub's
[private vulnerability reporting](https://github.com/sunilgentyala/TidyTrail-Desktop/security/advisories/new)
rather than a public issue. You should get an acknowledgement within 5
business days. Please include the version, OS, and steps to reproduce.

## Supported versions

Only the latest release receives security fixes while the project is
pre-1.0.

## Security model

TidyTrail Desktop reads directory metadata and moves user-selected items to
the OS trash. It makes **no network connections**, runs no external
programs, and loads no remote content.

| Boundary | Control |
|---|---|
| Webview → Rust backend (IPC) | The webview is treated as untrusted. Every path in a delete request is re-validated in Rust (`core/src/delete_guard.rs`): it must be absolute, contain no `.`/`..`, resolve (via its canonicalized parent, so symlinks aren't followed) strictly inside the folder that was last scanned, and not be a protected OS location. A delete with no completed scan is refused. |
| Protected locations | The OS directory and boot/system trees (`C:\Windows`, `/usr`, `/etc`, `/boot`, `/var/lib`, ...) and everything inside them can never be trashed. Volume roots, `Program Files`, `ProgramData`, user-profile roots, `/home`, `/var`, `/var/log`, `/opt`, `/srv` and similar can't be trashed themselves (their contents can). |
| File/folder names on screen | Names come from disk and are attacker-controlled on shared systems. They are only ever rendered with `textContent`/canvas text, never `innerHTML`. |
| Content Security Policy | `default-src 'self'; script-src 'self'; style-src 'self'; object-src 'none'; base-uri 'none'; form-action 'none'; frame-src 'none'`, plus Tauri's `freezePrototype`. |
| Tauri capabilities | Least privilege: `core:default`, `dialog:allow-open`, `dialog:allow-message`. No shell, opener, filesystem, or HTTP plugins. |
| Untrusted directory trees | The scanner is iterative (no recursion), never follows symlinks or Windows junctions, caps result depth at 256 levels, and skips Linux kernel pseudo-filesystems (`/proc`, `/sys`, ...). |
| Accountability | Every delete attempt, allowed or refused, is appended to a JSON Lines audit log (`deletions.jsonl`) with time, OS user, scan root, requested and resolved path, and outcome. If the log can't be opened, nothing is deleted. |

### Known limitations

- **Time-of-check/time-of-use.** Validation happens immediately before each
  move, but a local user who can write inside the scanned folder could in
  principle swap a parent directory for a link in that tiny window. The
  final path component is never followed, and escapes through a symlinked
  parent are rejected, but don't run the tool as root/Administrator over
  directories that untrusted users can write to.
- **Binaries are not yet code-signed.** Verify downloads as described below,
  or build from source.
- **Hard links** are counted once per link, so sizes can over-report on
  trees with many hard links.

## Verifying a release download

Every release asset has a SHA-256 checksum (`SHA256SUMS-Windows.txt`,
`SHA256SUMS-Linux.txt`) and a signed SLSA build-provenance attestation
proving it was built by this repository's release workflow:

```bash
# Linux / Git Bash
sha256sum --ignore-missing -c SHA256SUMS-Linux.txt
gh attestation verify TidyTrail-Desktop_0.2.0_amd64.deb --repo sunilgentyala/TidyTrail-Desktop
```

```powershell
# Windows PowerShell: compare against the line in SHA256SUMS-Windows.txt
Get-FileHash .\TidyTrail-Desktop-v0.2.0-portable-windows-x64.zip -Algorithm SHA256
gh attestation verify .\TidyTrail-Desktop-v0.2.0-portable-windows-x64.zip --repo sunilgentyala/TidyTrail-Desktop
```

## Development process

- Every push and PR runs `cargo fmt --check`, `cargo clippy -D warnings`, the
  full test suite on Windows and Linux (including IPC-level tests of the
  delete policy), `cargo deny` (RustSec vulnerabilities, licenses, sources;
  policy in `deny.toml`), and CodeQL (Rust, JavaScript, Actions).
- All GitHub Actions are pinned to full commit SHAs, workflows default to a
  read-only token, and Dependabot keeps actions and crates current.
