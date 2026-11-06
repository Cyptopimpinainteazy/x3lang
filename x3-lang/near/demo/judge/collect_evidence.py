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
import platform
import re
import shutil
import subprocess
import sys
import time
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
        # Node 20+ test runners may print either `ℹ pass 42` or TAP
        # `# pass 42`, depending on version and reporter settings.
        m = re.search(rf"^(?:ℹ|#)\s+{key}\s+(\d+)\s*$", text, flags=re.M)
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


SAFE_ENV = {"PYTHONPATH", "TMPDIR"}


def version(argv: list[str]) -> str:
    try:
        proc = subprocess.run(argv, cwd=REPO, capture_output=True, text=True, timeout=10, check=False)
    except (OSError, subprocess.TimeoutExpired):
        return "unavailable"
    text = (proc.stdout or proc.stderr).strip().splitlines()
    return text[0].strip() if text else "unavailable"


def status_for(exit_code: int | None) -> str:
    if exit_code is None:
        return "not_run"
    return "passed" if exit_code == 0 else "failed"


def summarize_counts(entry: dict[str, Any]) -> str:
    if entry["status"] != "passed" and entry.get("note"):
        return entry["note"].splitlines()[-1]
    if entry["passed"] is None:
        return "completed" if entry["status"] == "passed" else entry["status"].replace("_", " ")
    parts = [f"{entry['passed']} passed"]
    if entry["failed"]:
        parts.append(f"{entry['failed']} failed")
    if entry["skipped"]:
        parts.append(f"{entry['skipped']} skipped")
    return ", ".join(parts)


def run(name: str, argv: list[str], counter, *, test_id: str, kind: str, cwd: Path = REPO,
        env: dict[str, str] | None = None) -> dict[str, Any]:
    # Show `python3`, not this machine's interpreter path, in a file judges may see.
    shown = " ".join("python3" if arg == sys.executable else arg for arg in argv)
    if cwd != REPO:
        shown = f"(cd {cwd.relative_to(REPO)} && {shown})"
    if env:
        shown = " ".join(f"{k}={v if k in SAFE_ENV else '<redacted>'}" for k, v in env.items()) + " " + shown
    started = time.monotonic()
    try:
        proc = subprocess.run(argv, cwd=cwd, capture_output=True, text=True, timeout=TIMEOUT_S,
                              env={**os.environ, **(env or {})})
    except (OSError, subprocess.TimeoutExpired) as exc:
        duration_ms = round((time.monotonic() - started) * 1000)
        entry = {"id": test_id, "name": name, "kind": kind, "required": True, "command": shown,
                 "exit_code": None, **counts_none(""), "duration_ms": duration_ms,
                 "status": "not_run", "note": f"could not run: {type(exc).__name__}"}
        entry["summary"] = summarize_counts(entry)
        return entry
    output = proc.stdout + proc.stderr
    entry = {"id": test_id, "name": name, "kind": kind, "required": True, "command": shown,
             "exit_code": proc.returncode, **counter(output),
             "duration_ms": round((time.monotonic() - started) * 1000),
             "status": status_for(proc.returncode)}
    if proc.returncode != 0:
        entry["note"] = "\n".join(output.strip().splitlines()[-6:])
    entry["summary"] = summarize_counts(entry)
    return entry


def not_run(name: str, command: str, note: str, *, test_id: str, kind: str) -> dict[str, Any]:
    entry = {"id": test_id, "name": name, "kind": kind, "required": True, "command": command,
             "exit_code": None, **counts_none(""), "duration_ms": None,
             "status": "not_run", "note": note}
    entry["summary"] = summarize_counts(entry)
    return entry


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--with-contract", action="store_true",
                        help="also run x3-guard's cargo tests (needs the Rust toolchain and disk space)")
    parser.add_argument("--out", help="write evidence here instead of judge/evidence.json")
    args = parser.parse_args(argv)

    contract_cmd = "(cd x3-lang/near/x3-guard && cargo test --locked --quiet)"
    runs = [
        run("Policy checker, compiler and adapters (Python)", [sys.executable, "-m", "pytest", "x3-lang/tests", "-q"],
            counts_pytest, test_id="python", kind="test", env={"PYTHONPATH": "x3-lang"}),
        run("Browser checker and console (Node)", ["node", "--test", *sorted(
            str(p.relative_to(REPO)) for p in (REPO / "x3-lang/near/demo/test").glob("*.test.mjs"))],
            counts_node, test_id="browser", kind="test"),
        run("Console scenarios match the examples", [sys.executable, "x3-lang/near/demo/judge/build_scenarios.py", "--check"],
            counts_none, test_id="scenarios", kind="integrity"),
        run("Offline five-panel judge demo", [sys.executable, "scripts/hackathon/demo.py"],
            counts_none, test_id="demo", kind="demo"),
    ]
    if args.with_contract and shutil.which("cargo"):
        runs.append(run("x3-guard contract (Rust)", ["cargo", "test", "--locked", "--quiet"], counts_cargo,
                        test_id="contract", kind="test", cwd=REPO / "x3-lang/near/x3-guard"))
    else:
        runs.append(not_run("x3-guard contract (Rust)", contract_cmd,
                            "Not run in this recording. Rerun with --with-contract." if shutil.which("cargo")
                            else "Not run: cargo is not installed.", test_id="contract", kind="test"))

    required = [r for r in runs if r["required"]]
    passed = [r for r in required if r["status"] == "passed"]
    failed = [r for r in required if r["status"] == "failed"]
    not_run_runs = [r for r in required if r["status"] == "not_run"]
    evidence = {
        "schema": "x3.judge-console.evidence.v2",
        "recorded_at": datetime.now(timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z"),
        "commit": git("rev-parse", "HEAD"),
        "branch": git("branch", "--show-current"),
        "worktree_clean": git("status", "--porcelain", "--untracked-files=no") == "",
        "environment": {
            "python": platform.python_version(),
            "node": version(["node", "--version"]),
            "cargo": version(["cargo", "--version"]),
            "platform": platform.platform(),
        },
        "summary": {
            "required": len(required),
            "passed": len(passed),
            "failed": len(failed),
            "not_run": len(not_run_runs),
            "all_required_passed": not failed and not not_run_runs,
        },
        "runs": runs,
        "claim_boundaries": [
            {"claim": "Deterministic local policy verdicts", "proved_by": ["python", "browser", "scenarios"]},
            {"claim": "x3-guard contract behavior", "proved_by": ["contract"]},
            {"claim": "Wallet authorization or on-chain transaction", "proved_by": []},
            {"claim": "Signed live quote, deposit or settlement", "proved_by": []},
            {"claim": "Cross-chain causal attribution", "proved_by": []},
        ],
    }
    out = Path(args.out).resolve() if args.out else OUT
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(evidence, indent=2) + "\n", encoding="utf-8")
    failed = [r["name"] for r in runs if r["exit_code"] not in (0, None)]
    shown_out = out.relative_to(REPO) if out.is_relative_to(REPO) else out
    print(f"wrote {shown_out}")
    for r in runs:
        print(f"  {r['status']:8} {r['name']}: {r['summary']}")
    return 1 if failed else 0


if __name__ == "__main__":
    raise SystemExit(main())
