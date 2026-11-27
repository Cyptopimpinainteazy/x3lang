#!/usr/bin/env python3
"""Run the local forcefield over the archived invoice_06 payment.

    python3 scripts/hackathon/forcefield.py
    python3 scripts/hackathon/forcefield.py --json

The checks are local. They do not sign, deploy, or change the Guard account.
The archived FullAccess recovery key stays listed under open.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "x3-lang"))

import forcefield  # noqa: E402


def main() -> int:
    parser = argparse.ArgumentParser(description="Local X3 forcefield checks for invoice_06")
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args()
    report = forcefield.evaluate()
    if args.json:
        json.dump(report, sys.stdout, indent=2, default=str)
        sys.stdout.write("\n")
    else:
        sys.stdout.write(forcefield.render(report))
    return 0 if report["ok"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
