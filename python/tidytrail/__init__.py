"""Fast, safe disk-usage scanning and audited cleanup.

The heavy lifting (scanning, the delete policy, the audit log) is the same
Rust core that powers the TidyTrail Desktop app, so the rules for what may
be deleted are identical whether you click, script, or use the CLI.

    >>> import tidytrail
    >>> result = tidytrail.scan("/var/log")
    >>> for node in tidytrail.largest(result.root, 5):
    ...     print(tidytrail.format_bytes(node.size), node.path)
    >>> cleaner = tidytrail.Cleaner("/var/log")
    >>> report = cleaner.trash(["/var/log/app/old.log.1"], dry_run=True)
"""

from __future__ import annotations

import heapq
import os
import sys
from pathlib import Path
from typing import Iterator, Optional, Union

from ._native import (
    AUDIT_LOG_DIR_ENV,
    AUDIT_LOG_FILE_NAME,
    MAX_DEPTH,
    MAX_PATHS_PER_REQUEST,
    Node,
    ScanIssue,
    ScanResult,
    TrashReport,
    __version__,
    format_bytes,
    scan,
)
from ._native import Cleaner as _NativeCleaner

__all__ = [
    "AUDIT_LOG_DIR_ENV",
    "AUDIT_LOG_FILE_NAME",
    "MAX_DEPTH",
    "MAX_PATHS_PER_REQUEST",
    "Cleaner",
    "Node",
    "ScanIssue",
    "ScanResult",
    "TrashReport",
    "__version__",
    "default_audit_log_dir",
    "format_bytes",
    "iter_nodes",
    "largest",
    "scan",
]

StrPath = Union[str, "os.PathLike[str]"]


def default_audit_log_dir() -> Path:
    """Where the deletion audit log goes when no directory is given.

    ``$TIDYTRAIL_AUDIT_LOG_DIR`` if set; otherwise the platform's per-user
    log/state folder (``%LOCALAPPDATA%\\tidytrail\\logs`` on Windows,
    ``~/Library/Logs/tidytrail`` on macOS, ``$XDG_STATE_HOME/tidytrail`` or
    ``~/.local/state/tidytrail`` elsewhere).
    """
    override = os.environ.get(AUDIT_LOG_DIR_ENV)
    if override:
        return Path(override)
    if sys.platform == "win32":
        base = os.environ.get("LOCALAPPDATA") or str(Path.home() / "AppData" / "Local")
        return Path(base) / "tidytrail" / "logs"
    if sys.platform == "darwin":
        return Path.home() / "Library" / "Logs" / "tidytrail"
    state = os.environ.get("XDG_STATE_HOME") or str(Path.home() / ".local" / "state")
    return Path(state) / "tidytrail"


class Cleaner(_NativeCleaner):
    """Moves files to the OS trash, only inside ``scan_root``, with an audit log.

    Every path is re-checked by the Rust core: it must resolve strictly
    inside ``scan_root`` (symlinked parents can't escape), contain no
    ``..``, and not be a protected OS location. Every attempt is appended to
    ``deletions.jsonl`` in ``audit_log_dir`` (default:
    :func:`default_audit_log_dir`); if that can't be written, nothing is
    deleted.
    """

    def __new__(cls, scan_root: StrPath, audit_log_dir: Optional[StrPath] = None):
        return super().__new__(
            cls,
            os.fspath(scan_root),
            os.fspath(audit_log_dir) if audit_log_dir is not None else str(default_audit_log_dir()),
        )


def iter_nodes(node: Node) -> Iterator[Node]:
    """Yields ``node`` and everything beneath it, depth-first (no recursion)."""
    stack = [node]
    while stack:
        current = stack.pop()
        yield current
        stack.extend(reversed(current.children))


def largest(node: Node, n: int = 20, *, files_only: bool = True) -> list:
    """The ``n`` biggest entries under ``node``, largest first.

    With ``files_only=False`` directories are included too (sized by
    everything beneath them), which is usually what you want for "which
    folder is eating the disk".
    """
    candidates = (
        x for x in iter_nodes(node) if x is not node and (not files_only or not x.is_dir)
    )
    return heapq.nlargest(n, candidates, key=lambda x: x.size)
