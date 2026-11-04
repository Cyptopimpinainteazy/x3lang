#!/usr/bin/env python3
"""Run the judge console's checks and record what they did in `evidence.json`.

    python3 x3-lang/near/demo/judge/collect_evidence.py                  # Python, Node, scenarios
    python3 x3-lang/near/demo/judge/collect_evidence.py --with-contract  # also x3-guard's cargo tests

Each entry is a command that really ran: its exit code and the pass/fail/skip counts its
own output reported. A check that was not run is recorded as not run, never as passed.
The file names the commit and whether the worktree was clean, since a dirty worktree's
results do not belong to that commit. The console (`../judge.html`) shows it in panel 5.
"""
from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

REPO = Path(__file__).resolve().parents[4]
OUT = Path(__file__).with_name("evidence.json")
TIMEOUT_S = 900


def git(*args: str) -> str:
    return subprocess.run(["git", *args], cwd=REPO, capture_output=True, text=True, check=True).stdout.strip()


def counts_pytest(text: str) -> dict[str, int]:
    tail = text.strip().splitlines()[-1] if text.strip() else ""
    found = {k: int(n) for n, k in re.findall(r"(\d+) (passed|failed|skipped|errors?)", tail)}
    return {"passed": found.get("passed", 0), "failed": found.get("failed", 0) + found.get("error", 0)
            + found.get("errors", 0), "skipped": found.get("skipped", 0)}


def counts_node(text: str) -> dict[str, int]:
    def one(key: str) -> int:
        m = re.search(rf"^ℹ {key} (\d+)$", text, flags=re.M)
        return int(m.group(1)) if m else 0
    return {"passed": one("pass"), "failed": one("fail") + one("cancelled"), "skipped": one("skipped") + one("todo")}


def counts_cargo(text: str) -> dict[str, int]:
    totals = {"passed": 0, "failed": 0, "skipped": 0}
    for p, f, i in re.findall(r"test result: \w+\. (\d+) passed; (\d+) failed; (\d+) ignored", text):
        totals["passed"] += int(p)
        totals["failed"] += int(f)
        totals["skipped"] += int(i)
    return totals


def counts_none(_: str) -> dict[str, Any]:
    return {"passed": None, "failed": None, "skipped": None}


def run(name: str, argv: list[str], counter, *, cwd: Path = REPO, env: dict[str, str] | None = None) -> dict[str, Any]:
    # Show `python3`, not this machine's interpreter path, in a file judges may see.
    shown = " ".join("python3" if arg == sys.executable else arg for arg in argv)
    if cwd != REPO:
        shown = f"(cd {cwd.relative_to(REPO)} && {shown})"
    if env:
        shown = " ".join(f"{k}={v}" for k, v in env.items()) + " " + shown
    try:
        proc = subprocess.run(argv, cwd=cwd, capture_output=True, text=True, timeout=TIMEOUT_S,
                              env={**os.environ, **(env or {})})
    except (OSError, subprocess.TimeoutExpired) as exc:
        return {"name": name, "command": shown, "exit_code": None, **counts_none(""),
                "note": f"could not run: {type(exc).__name__}"}
    output = proc.stdout + proc.stderr
    entry = {"name": name, "command": shown, "exit_code": proc.returncode, **counter(output)}
    if proc.returncode != 0:
        entry["note"] = "\n".join(output.strip().splitlines()[-6:])
    return entry


def not_run(name: str, command: str, note: str) -> dict[str, Any]:
    return {"name": name, "command": command, "exit_code": None, **counts_none(""), "note": note}


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--with-contract", action="store_true",
                        help="also run x3-guard's cargo tests (needs the Rust toolchain and disk space)")
    args = parser.parse_args(argv)

    contract_cmd = "(cd x3-lang/near/x3-guard && cargo test --locked --quiet)"
    runs = [
        run("Policy checker, compiler and adapters (Python)", [sys.executable, "-m", "pytest", "x3-lang/tests", "-q"],
            counts_pytest, env={"PYTHONPATH": "x3-lang"}),
        run("Browser checker and console (Node)", ["node", "--test", *sorted(
            str(p.relative_to(REPO)) for p in (REPO / "x3-lang/near/demo/test").glob("*.test.mjs"))], counts_node),
        run("Console scenarios match the examples", [sys.executable, "x3-lang/near/demo/judge/build_scenarios.py", "--check"],
            counts_none),
    ]
    if args.with_contract and shutil.which("cargo"):
        runs.append(run("x3-guard contract (Rust)", ["cargo", "test", "--locked", "--quiet"], counts_cargo,
                        cwd=REPO / "x3-lang/near/x3-guard"))
    else:
        runs.append(not_run("x3-guard contract (Rust)", contract_cmd,
                            "Not run in this recording. Rerun with --with-contract." if shutil.which("cargo")
                            else "Not run: cargo is not installed."))

    evidence = {
        "schema": "x3.judge-console.evidence.v1",
        "recorded_at": datetime.now(timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z"),
        "commit": git("rev-parse", "HEAD"),
        "worktree_clean": git("status", "--porcelain", "--untracked-files=no") == "",
        "runs": runs,
    }
    OUT.write_text(json.dumps(evidence, indent=2) + "\n", encoding="utf-8")
    failed = [r["name"] for r in runs if r["exit_code"] not in (0, None)]
    print(f"wrote {OUT.relative_to(REPO)}")
    for r in runs:
        status = "not run" if r["exit_code"] is None else f"exit {r['exit_code']}"
        print(f"  {status:8} {r['name']}: passed {r['passed']}, failed {r['failed']}, skipped {r['skipped']}")
    return 1 if failed else 0


if __name__ == "__main__":
    raise SystemExit(main())
