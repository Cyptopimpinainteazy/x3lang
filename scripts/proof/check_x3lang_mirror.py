#!/usr/bin/env python3
"""Fail closed when shared X3Lang files or feature changes diverge across repositories."""
from __future__ import annotations

import argparse
import hashlib
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]
SHARED = (
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
    names = git(repo, "diff", "--name-only", "--diff-filter=ACDMR",
                upstream, "HEAD", "--", *SCOPED)
    return set(names.splitlines()) if names else set()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--peer-repo", required=True, type=Path)
    parser.add_argument("--base-ref", help="Local base ref, e.g. origin/main")
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
        left, right = ROOT / rel, peer / rel
        if not left.exists() and not right.exists() and rel not in SHARED:
            print(f"MATCH DELETE {rel}")
            continue
        if not left.is_file() or not right.is_file():
            failures.append((rel, "missing on one or both repositories"))
            continue
        a = hashlib.sha256(left.read_bytes()).hexdigest()
        b = hashlib.sha256(right.read_bytes()).hexdigest()
        if a != b:
            failures.append((rel, f"SHA mismatch {a[:12]} != {b[:12]}"))
        else:
            print(f"MATCH {rel} sha256={a}")
    if failures:
        for rel, why in failures:
            print(f"DRIFT {rel}: {why}", file=sys.stderr)
        return 1
    print(f"PROOF MIRROR MATCHED {len(paths)} files; {len(changed)} changed language/proof paths checked")
    return 0


if __name__ == "__main__":
    sys.exit(main())
