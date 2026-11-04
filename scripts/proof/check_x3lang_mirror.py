#!/usr/bin/env python3
"""Fail-closed comparison of X3Lang proof sources across two local Git checkouts."""
from __future__ import annotations

import argparse
import hashlib
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]
SHARED = (
    "scripts/proof/x3lang_execution_proof.py",
    "scripts/proof/check_x3lang_mirror.py",
    "x3-lang/compiler/tests/test_control_flow_e2e.rs",
    "x3-lang/compiler/tests/test_bytecode_version_gate.rs",
    "x3-lang/vm/tests/test_nonce_replay.rs",
    "x3-lang/vm/tests/trading_receipts.rs",
)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--peer-repo", required=True, type=Path)
    args = parser.parse_args()
    peer = args.peer_repo.resolve()
    if not (peer / ".git").exists() and not (peer / ".git").is_file():
        parser.error("peer is not a Git checkout")
    try:
        other = subprocess.check_output(
            ["git", "-C", str(peer), "rev-parse", "--show-toplevel"],
            text=True,
        ).strip()
    except subprocess.CalledProcessError:
        parser.error("cannot resolve peer repository")
    if Path(other) == ROOT:
        parser.error("refusing to compare repository to itself")

    mismatches = []
    for rel in SHARED:
        left, right = ROOT / rel, peer / rel
        if not left.is_file() or not right.is_file():
            mismatches.append((rel, "missing file"))
            continue
        a = hashlib.sha256(left.read_bytes()).hexdigest()
        b = hashlib.sha256(right.read_bytes()).hexdigest()
        if a != b:
            mismatches.append((rel, f"SHA mismatch {a[:12]} != {b[:12]}"))
        else:
            print(f"MATCH {rel} sha256={a}")
    if mismatches:
        for rel, why in mismatches:
            print(f"DRIFT {rel}: {why}", file=sys.stderr)
        return 1
    print(f"PROOF MIRROR MATCHED {len(SHARED)} files")
    return 0


if __name__ == "__main__":
    sys.exit(main())
