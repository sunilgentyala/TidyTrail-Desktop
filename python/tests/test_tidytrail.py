import json
import os
import subprocess
import sys
from pathlib import Path

import pytest

import tidytrail


@pytest.fixture
def tree(tmp_path: Path) -> dict:
    """scanned/{big.bin 5000B, sub/small.txt 100B, sub/mid.log 700B} plus outside/victim.txt"""
    scanned = tmp_path / "scanned"
    sub = scanned / "sub"
    outside = tmp_path / "outside"
    sub.mkdir(parents=True)
    outside.mkdir()
    (scanned / "big.bin").write_bytes(b"x" * 5000)
    (sub / "small.txt").write_bytes(b"x" * 100)
    (sub / "mid.log").write_bytes(b"x" * 700)
    (outside / "victim.txt").write_bytes(b"keep me")
    return {
        "tmp": tmp_path,
        "scanned": scanned,
        "sub": sub,
        "outside": outside,
        "logs": tmp_path / "audit",
    }


def audit_records(logs: Path) -> list:
    return [json.loads(line) for line in (logs / "deletions.jsonl").read_text().splitlines()]


# --- scanning -------------------------------------------------------------


def test_scan_sums_sizes_and_sorts_children_largest_first(tree):
    result = tidytrail.scan(tree["scanned"])
    assert result.root.size == 5800
    assert [c.name for c in result.root.children] == ["big.bin", "sub"]
    assert result.root.children[1].size == 800
    assert result.issues == []


def test_scan_accepts_str_and_pathlib(tree):
    assert tidytrail.scan(str(tree["scanned"])).root.size == tidytrail.scan(tree["scanned"]).root.size


def test_scan_rejects_missing_and_non_directory_paths(tree):
    with pytest.raises(FileNotFoundError):
        tidytrail.scan(tree["tmp"] / "nope")
    with pytest.raises(NotADirectoryError):
        tidytrail.scan(tree["scanned"] / "big.bin")


def test_largest_files_and_dirs(tree):
    root = tidytrail.scan(tree["scanned"]).root
    assert [n.name for n in tidytrail.largest(root, 2)] == ["big.bin", "mid.log"]
    with_dirs = tidytrail.largest(root, 3, files_only=False)
    assert [n.name for n in with_dirs] == ["big.bin", "sub", "mid.log"]


def test_iter_nodes_visits_everything_once(tree):
    root = tidytrail.scan(tree["scanned"]).root
    names = sorted(n.name for n in tidytrail.iter_nodes(root))
    assert names == sorted(["scanned", "big.bin", "sub", "small.txt", "mid.log"])


def test_node_metadata(tree):
    root = tidytrail.scan(tree["scanned"]).root
    big = root.children[0]
    assert big.is_dir is False and big.category == "other"
    assert root.children[1].is_dir is True
    assert Path(big.path) == tree["scanned"] / "big.bin"
    assert "big.bin" in repr(big)


def test_format_bytes():
    assert tidytrail.format_bytes(0) == "0 B"
    assert tidytrail.format_bytes(1536).endswith("KB")


# --- the delete policy ------------------------------------------------------


def test_check_allows_inside_and_refuses_outside(tree):
    cleaner = tidytrail.Cleaner(tree["scanned"], tree["logs"])
    assert Path(cleaner.check(tree["scanned"] / "big.bin")).name == "big.bin"
    with pytest.raises(PermissionError, match="outside the scanned folder"):
        cleaner.check(tree["outside"] / "victim.txt")
    with pytest.raises(PermissionError, match="scanned folder itself"):
        cleaner.check(tree["scanned"])
    with pytest.raises(PermissionError, match=r"'\.\.'"):
        cleaner.check(tree["scanned"] / ".." / "outside" / "victim.txt")
    with pytest.raises(FileNotFoundError):
        cleaner.check(tree["scanned"] / "missing.txt")


def test_dry_run_moves_nothing_but_is_audited(tree):
    cleaner = tidytrail.Cleaner(tree["scanned"], tree["logs"])
    target = tree["sub"] / "mid.log"
    report = cleaner.trash([target, tree["outside"] / "victim.txt"], dry_run=True)

    assert report.dry_run and not report.ok
    assert report.trashed == [str(target)]
    assert "outside the scanned folder" in report.failed[0][1]
    assert target.exists()
    assert [r["outcome"] for r in audit_records(tree["logs"])] == ["would-trash", "failed"]


def test_trash_moves_allowed_paths_and_never_touches_refused_ones(tree):
    cleaner = tidytrail.Cleaner(tree["scanned"], tree["logs"])
    target = tree["sub"] / "small.txt"
    victim = tree["outside"] / "victim.txt"

    report = cleaner.trash([target, victim])

    assert report.trashed == [str(target)]
    assert not target.exists()
    assert victim.exists()
    assert Path(report.audit_log) == tree["logs"] / "deletions.jsonl"
    assert [r["outcome"] for r in audit_records(tree["logs"])] == ["trashed", "failed"]


def test_os_directory_is_protected_even_when_its_volume_is_scanned(tmp_path):
    if sys.platform == "win32":
        volume = os.environ.get("SystemDrive", "C:") + "\\"
        os_dir = os.environ.get("SystemRoot", r"C:\Windows")
    else:
        volume, os_dir = "/", "/usr"
    cleaner = tidytrail.Cleaner(volume, tmp_path)
    with pytest.raises(PermissionError, match="protected"):
        cleaner.check(os_dir)


def test_default_audit_dir_honors_env(monkeypatch, tmp_path):
    monkeypatch.setenv(tidytrail.AUDIT_LOG_DIR_ENV, str(tmp_path / "x"))
    assert tidytrail.default_audit_log_dir() == tmp_path / "x"


def test_cleaner_uses_env_audit_dir_by_default(tree, monkeypatch):
    monkeypatch.setenv(tidytrail.AUDIT_LOG_DIR_ENV, str(tree["logs"]))
    cleaner = tidytrail.Cleaner(tree["scanned"])
    assert Path(cleaner.audit_log) == tree["logs"] / "deletions.jsonl"


def test_unwritable_audit_log_blocks_deletion(tree):
    # A *file* where the log directory should be: the log can't be created.
    blocker = tree["tmp"] / "not-a-dir"
    blocker.write_text("")
    cleaner = tidytrail.Cleaner(tree["scanned"], blocker)
    target = tree["scanned"] / "big.bin"
    with pytest.raises(OSError, match="nothing was deleted"):
        cleaner.trash([target])
    assert target.exists()


# --- CLI ------------------------------------------------------------------


def run_cli(*args, input=None, env=None):
    return subprocess.run(
        [sys.executable, "-m", "tidytrail", *map(str, args)],
        capture_output=True,
        text=True,
        input=input,
        env={**os.environ, **(env or {})},
    )


def test_cli_version():
    out = run_cli("--version")
    assert out.returncode == 0 and tidytrail.__version__ in out.stdout


def test_cli_scan_json(tree):
    out = run_cli("scan", tree["scanned"], "--json", "--top", "2")
    assert out.returncode == 0, out.stderr
    data = json.loads(out.stdout)
    assert data["total_size"] == 5800
    assert [Path(e["path"]).name for e in data["largest"]] == ["big.bin", "mid.log"]


def test_cli_scan_text(tree):
    out = run_cli("scan", tree["scanned"], "--dirs")
    assert out.returncode == 0, out.stderr
    assert "big.bin" in out.stdout and "sub" in out.stdout


def test_cli_trash_refuses_without_yes_when_not_interactive(tree):
    target = tree["scanned"] / "big.bin"
    out = run_cli("trash", "--root", tree["scanned"], "--audit-log-dir", tree["logs"], target, input="")
    assert out.returncode == 2
    assert "--yes" in out.stderr
    assert target.exists()


def test_cli_trash_dry_run_then_real(tree):
    target = tree["sub"] / "mid.log"
    common = ["trash", "--root", tree["scanned"], "--audit-log-dir", tree["logs"], "--json"]

    dry = run_cli(*common, "--dry-run", target)
    assert dry.returncode == 0, dry.stderr
    assert json.loads(dry.stdout)["dry_run"] is True
    assert target.exists()

    real = run_cli(*common, "--yes", target)
    assert real.returncode == 0, real.stderr
    assert not target.exists()


def test_cli_trash_partial_failure_exit_code(tree):
    out = run_cli(
        "trash", "--root", tree["scanned"], "--audit-log-dir", tree["logs"], "--yes",
        tree["outside"] / "victim.txt",
    )
    assert out.returncode == 1
    assert "outside the scanned folder" in out.stdout
    assert (tree["outside"] / "victim.txt").exists()


def test_cli_trash_reads_paths_from_stdin(tree):
    target = tree["scanned"] / "big.bin"
    out = run_cli(
        "trash", "--root", tree["scanned"], "--audit-log-dir", tree["logs"], "--dry-run",
        "--from-file", "-", input=f"{target}\n\n",
    )
    assert out.returncode == 0, out.stderr
    assert "Would move 1 item" in out.stdout


def test_cli_bad_root_is_a_usage_error(tree):
    out = run_cli("trash", "--root", tree["tmp"] / "missing", "--dry-run", "x")
    assert out.returncode == 2
    assert "does not exist" in out.stderr
