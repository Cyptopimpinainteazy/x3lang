#!/usr/bin/env python3
"""Replay the archived invoice_06 approval requirement and a separate local threshold payment.

    python3 scripts/hackathon/agent_law.py
    python3 scripts/hackathon/agent_law.py --json

A permitted program gets a local X3 receipt. A forbidden program and a Solana
transfer do not. Revocation stops the next program. The court exhibit keeps
the receipt. Nothing is signed or broadcast.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "x3-lang"))

import agent_law  # noqa: E402


def main() -> int:
    parser = argparse.ArgumentParser(description="Demonstrate the local X3 agent-law path")
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args()
    report = agent_law.demonstrate()
    if args.json:
        json.dump(report, sys.stdout, indent=2)
        sys.stdout.write("\n")
    else:
        sys.stdout.write(agent_law.render(report))
    return 0 if report["ok"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
