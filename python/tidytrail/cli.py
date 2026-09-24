"""``tidytrail`` command-line interface.

    tidytrail scan /var/log --top 20
    tidytrail scan /data --dirs --json
    tidytrail trash --root /var/log --dry-run /var/log/app/old.log.1
    tidytrail trash --root /var/log --yes --from-file paths.txt

Exit codes: 0 success, 1 some paths were refused or failed, 2 usage error
or nothing was done, 130 interrupted.
"""

from __future__ import annotations

import argparse
import contextlib
import json
import sys
from typing import List, Optional, Sequence

import tidytrail

EXIT_OK = 0
EXIT_PARTIAL = 1
EXIT_USAGE = 2
EXIT_INTERRUPTED = 130


def _node_dict(node: tidytrail.Node) -> dict:
    return {
        "path": node.path,
        "size": node.size,
        "is_dir": node.is_dir,
        "category": node.category,
    }


def cmd_scan(args: argparse.Namespace) -> int:
    result = tidytrail.scan(args.path)
    top = tidytrail.largest(result.root, args.top, files_only=not args.dirs)

    if args.json:
        json.dump(
            {
                "root": result.root.path,
                "total_size": result.root.size,
                "largest": [_node_dict(n) for n in top],
                "issues": [{"path": i.path, "message": i.message} for i in result.issues],
            },
            sys.stdout,
            indent=2,
        )
        sys.stdout.write("\n")
        return EXIT_OK

    print(f"{result.root.path}: {tidytrail.format_bytes(result.root.size)}")
    kind = "entries" if args.dirs else "files"
    print(f"\nLargest {len(top)} {kind}:")
    width = max((len(tidytrail.format_bytes(n.size)) for n in top), default=0)
    for node in top:
        suffix = "/" if node.is_dir else ""
        print(f"  {tidytrail.format_bytes(node.size):>{width}}  {node.path}{suffix}")
    if result.issues:
        print(f"\n{len(result.issues)} path(s) skipped (use --json to list them).")
    return EXIT_OK


def _read_paths(args: argparse.Namespace) -> List[str]:
    paths = list(args.paths)
    if args.from_file:
        if args.from_file == "-":
            source = contextlib.nullcontext(sys.stdin)
        else:
            source = open(args.from_file, encoding="utf-8")
        with source as f:
            paths.extend(line.rstrip("\r\n") for line in f if line.strip())
    return paths


def _confirm(paths: Sequence[str], root: str) -> bool:
    print(f"About to move {len(paths)} item(s) under {root} to the trash:")
    for p in paths[:20]:
        print(f"  {p}")
    if len(paths) > 20:
        print(f"  ...and {len(paths) - 20} more")
    try:
        answer = input("Type 'yes' to continue: ")
    except EOFError:
        return False
    return answer.strip().lower() == "yes"


def cmd_trash(args: argparse.Namespace) -> int:
    paths = _read_paths(args)
    if not paths:
        print("tidytrail: no paths given", file=sys.stderr)
        return EXIT_USAGE

    cleaner = tidytrail.Cleaner(args.root, args.audit_log_dir)

    if not args.dry_run and not args.yes:
        if not sys.stdin.isatty():
            print(
                "tidytrail: refusing to delete without confirmation; pass --yes when "
                "running non-interactively (or --dry-run to preview)",
                file=sys.stderr,
            )
            return EXIT_USAGE
        if not _confirm(paths, cleaner.scan_root):
            print("Cancelled; nothing was deleted.")
            return EXIT_USAGE

    report = cleaner.trash(paths, dry_run=args.dry_run)

    if args.json:
        json.dump(
            {
                "dry_run": report.dry_run,
                "trashed": report.trashed,
                "failed": [{"path": p, "reason": r} for p, r in report.failed],
                "audit_log": report.audit_log,
            },
            sys.stdout,
            indent=2,
        )
        sys.stdout.write("\n")
    else:
        verb = "Would move" if report.dry_run else "Moved"
        print(f"{verb} {len(report.trashed)} item(s) to the trash.")
        for p in report.trashed:
            print(f"  {p}")
        if report.failed:
            print(f"{len(report.failed)} item(s) refused or failed:")
            for p, reason in report.failed:
                print(f"  {p}: {reason}")
        print(f"Audit log: {report.audit_log}")
        if not report.dry_run and report.trashed:
            print("Note: space is only freed once the trash is emptied.")
    return EXIT_OK if report.ok else EXIT_PARTIAL


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="tidytrail",
        description="Find what is using disk space and clean it up safely, with an audit log.",
    )
    parser.add_argument("--version", action="version", version=f"tidytrail {tidytrail.__version__}")
    sub = parser.add_subparsers(dest="command", required=True)

    p_scan = sub.add_parser("scan", help="show the largest files or folders under a path")
    p_scan.add_argument("path", help="folder to scan")
    p_scan.add_argument("--top", type=int, default=20, help="how many entries to show (default 20)")
    p_scan.add_argument("--dirs", action="store_true", help="include folders, not just files")
    p_scan.add_argument("--json", action="store_true", help="machine-readable output")
    p_scan.set_defaults(func=cmd_scan)

    p_trash = sub.add_parser(
        "trash",
        help="move paths inside --root to the OS trash, with an audit record for each",
    )
    p_trash.add_argument("paths", nargs="*", help="paths to move to the trash")
    p_trash.add_argument(
        "--root",
        required=True,
        help="only paths strictly inside this folder may be trashed",
    )
    p_trash.add_argument(
        "--from-file",
        metavar="FILE",
        help="also read paths, one per line, from FILE ('-' for stdin)",
    )
    p_trash.add_argument("--dry-run", action="store_true", help="check and audit, but move nothing")
    p_trash.add_argument("--yes", action="store_true", help="don't ask for confirmation")
    p_trash.add_argument(
        "--audit-log-dir",
        metavar="DIR",
        help=f"where to write deletions.jsonl (default: ${tidytrail.AUDIT_LOG_DIR_ENV} "
        "or the per-user log folder)",
    )
    p_trash.add_argument("--json", action="store_true", help="machine-readable output")
    p_trash.set_defaults(func=cmd_trash)
    return parser


def main(argv: Optional[Sequence[str]] = None) -> int:
    args = build_parser().parse_args(argv)
    try:
        return args.func(args)
    except KeyboardInterrupt:
        print("\ntidytrail: interrupted", file=sys.stderr)
        return EXIT_INTERRUPTED
    except (OSError, ValueError) as e:
        print(f"tidytrail: {e}", file=sys.stderr)
        return EXIT_USAGE


if __name__ == "__main__":  # pragma: no cover
    sys.exit(main())
