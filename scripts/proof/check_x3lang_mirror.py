#!/usr/bin/env python3
"""Fail closed when shared X3Lang files or feature changes diverge across repositories."""
from __future__ import annotations

import argparse
import hashlib
import os
from pathlib import Path
import stat
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]
SHARED = (
    "scripts/proof/checkout_integrity.py",
    "scripts/proof/x3lang_execution_proof.py",
    "scripts/proof/x3lang_runtime_equivalence.py",
    "scripts/proof/check_x3lang_mirror.py",
    "x3-lang/compiler/tests/test_control_flow_e2e.rs",
    "x3-lang/compiler/tests/test_bytecode_version_gate.rs",
    "x3-lang/vm/tests/test_nonce_replay.rs",
    "x3-lang/vm/tests/trading_receipts.rs",
)
SCOPED = ("x3-lang/", "scripts/proof/")


def git(repo: Path, *args: str) -> str:
    return subprocess.check_output(
        ["git", "-C", str(repo), *args], text=True, stderr=subprocess.STDOUT
    ).strip()


def changes(repo: Path, base: str | None) -> set[str]:
    if not base:
        return set()
    # Use branch divergence, not a reviewer's guessed file list.
    upstream = git(repo, "merge-base", base, "HEAD")
    # Disable rename folding so the old path's deletion is checked as well.
    # Include every change type, including regular-file/symlink transitions.
    names = subprocess.check_output(
        ["git", "-C", str(repo), "diff", "--no-renames", "--name-only", "-z",
         upstream, "HEAD", "--", *SCOPED], stderr=subprocess.STDOUT,
    )
    return {os.fsdecode(name) for name in names.split(b"\0") if name}


def identity(path: Path) -> tuple[str, str] | None:
    """Compare Git file modes and stored bytes without following symlinks."""
    try:
        mode = path.lstat().st_mode
    except FileNotFoundError:
        return None
    if stat.S_ISLNK(mode):
        return "120000", hashlib.sha256(os.fsencode(os.readlink(path))).hexdigest()
    if stat.S_ISREG(mode):
        return ("100755" if mode & 0o111 else "100644"), hashlib.sha256(path.read_bytes()).hexdigest()
    raise ValueError("unsupported file type")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--peer-repo", required=True, type=Path)
    parser.add_argument("--base-ref", help="Local base ref, e.g. origin/master")
    parser.add_argument("--peer-base-ref", help="Peer base ref, e.g. origin/master")
    args = parser.parse_args()
    peer = args.peer_repo.resolve()
    if not (peer / ".git").exists():
        parser.error("peer is not a Git checkout")
    try:
        other = Path(git(peer, "rev-parse", "--show-toplevel"))
        current = Path(git(ROOT, "rev-parse", "--show-toplevel"))
        changed = changes(ROOT, args.base_ref) | changes(peer, args.peer_base_ref)
    except subprocess.CalledProcessError as exc:
        parser.error(f"cannot establish comparable Git revisions: {exc}")
    if other == current:
        parser.error("refusing to compare a repository to itself")
    paths = sorted(set(SHARED) | changed)
    failures = []
    for rel in paths:
        try:
            left, right = identity(ROOT / rel), identity(peer / rel)
        except (OSError, ValueError) as exc:
            failures.append((rel, f"cannot compare file: {exc}"))
            continue
        if left is None and right is None and rel not in SHARED:
            print(f"MATCH DELETE {rel}")
            continue
        if left is None or right is None:
            failures.append((rel, "missing on one or both repositories"))
            continue
        if left[0] != right[0]:
            failures.append((rel, f"Git mode mismatch {left[0]} != {right[0]}"))
        elif rel in SHARED and left[0] == "120000":
            failures.append((rel, "required proof input must be a regular file"))
        elif left[1] != right[1]:
            a, b = left[1], right[1]
            failures.append((rel, f"SHA mismatch {a[:12]} != {b[:12]}"))
        else:
            print(f"MATCH {rel} mode={left[0]} sha256={left[1]}")
    if failures:
        for rel, why in failures:
            print(f"DRIFT {rel}: {why}", file=sys.stderr)
        return 1
    print(f"PROOF MIRROR MATCHED {len(paths)} files; {len(changed)} changed language/proof paths checked")
    return 0


if __name__ == "__main__":
    sys.exit(main())
