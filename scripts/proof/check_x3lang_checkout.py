#!/usr/bin/env python3
"""Admit exact committed X3Lang source; do not assert cross-repository parity."""
import argparse
import json
from pathlib import Path
import subprocess
import sys

from check_x3lang_mirror import SHARED, identity
from source_binding import SourceBindingError, verify_committed_source

ROOT = Path(__file__).resolve().parents[2]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--expected-head", required=True)
    parser.add_argument("--output-dir", required=True, type=Path)
    args = parser.parse_args()
    try:
        revision = verify_committed_source(
            ROOT, output_dir=args.output_dir, expected_revision=args.expected_head,
        )
        for relative in SHARED:
            entry = identity(ROOT, relative)
            if entry is None or entry[0] not in ("100644", "100755"):
                raise SourceBindingError(f"required proof input must be a committed regular file: {relative}")
        # Detect a revision/input change while inspecting the required proof files.
        verify_committed_source(ROOT, output_dir=args.output_dir, expected_revision=revision)
    except (SourceBindingError, OSError, ValueError, subprocess.CalledProcessError) as error:
        print(f"SOURCE ADMISSION REFUSED: {error}", file=sys.stderr)
        return 2
    args.output_dir.mkdir(parents=True, exist_ok=True)
    report = {
        "head": revision,
        "scope": "complete committed checkout bytes, modes, index and required proof inputs",
        "cross_repository_parity": "not_checked",
        "execution_proven": False,
        "onchain_proven": False,
        "required_proof_inputs": list(SHARED),
    }
    (args.output_dir / "checkout.json").write_text(json.dumps(report, indent=2) + "\n")
    print(f"SOURCE ADMISSION PASSED {revision}; cross-repository parity NOT CHECKED")
    return 0


if __name__ == "__main__":
    sys.exit(main())
