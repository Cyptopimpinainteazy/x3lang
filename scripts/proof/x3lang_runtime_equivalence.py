#!/usr/bin/env python3
"""Fail-closed exact-head proof of local X3 canonical-runtime invariants.

Requires the full xxxstar workspace. Not a live chain or cross-chain proof.
"""
from __future__ import annotations
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
from datetime import datetime, timezone

ROOT = Path(__file__).resolve().parents[2]
SUITES = (
    ("source_three_engine_agreement", "source_language", "std compile"),
    ("runtime_chain_engine", "chain_engine", "std"),
    ("opcode_differential", "opcode_differential", "std"),
    ("gas_enforcement", "gas_accounting", "std"),
    ("transactional_rollback", "x3lang_trading_rollback", "std compile"),
    ("signed_receipt_replay", "x3lang_trading_receipt_replay", "std compile"),
)
MANIFEST = "crates/x3-integration/Cargo.toml"

def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()

def git(*args: str) -> str:
    return subprocess.check_output(
        ["git", "-C", str(ROOT), *args], text=True, stderr=subprocess.STDOUT
    ).strip()

def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output-dir", required=True, type=Path)
    args = parser.parse_args()
    output = args.output_dir.resolve()
    output.mkdir(parents=True, exist_ok=True)
    files = (MANIFEST,) + tuple(
        f"crates/x3-integration/tests/{test}.rs" for _, test, _ in SUITES
    )
    missing = [f for f in files if not (ROOT / f).is_file()]
    if missing:
        parser.error(
            f"canonical runtime workspace absent: {missing}. "
            "Run this suite in xxxstar; standalone x3lang only mirrors the runner."
        )
    for relative in files:
        subprocess.run(
            ["git", "-C", str(ROOT), "ls-files", "--error-unmatch", relative],
            stdout=subprocess.DEVNULL, check=True,
        )
        if subprocess.run(
            ["git", "-C", str(ROOT), "diff", "--quiet", "HEAD", "--", relative]
        ).returncode:
            parser.error(f"uncommitted runtime test input: {relative}")
    report = {
        "schema": "x3lang-local-runtime-equivalence/v1",
        "status": "failed",
        "scope": (
            "local X3 runtime, three execution paths, gas, opcode, rollback, "
            "signed receipt replay; NOT EVM/SVM/NEAR settlement or chain finality"
        ),
        "repo_revision": git("rev-parse", "HEAD"),
        "observed_utc": datetime.now(timezone.utc).isoformat(),
        "inputs_sha256": {f: digest((ROOT / f).read_bytes()) for f in files},
        "tests": [],
    }
    env = dict(os.environ, CARGO_INCREMENTAL="0")
    passed = True
    try:
        for name, test, features in SUITES:
            command = [
                "cargo", "test", "--locked", "-p", "x3-x3-integration",
                "--test", test, "--features", features, "--", "--nocapture",
            ]
            print("RUN", name, flush=True)
            try:
                execution = subprocess.run(
                    command, cwd=ROOT, env=env, stdout=subprocess.PIPE,
                    stderr=subprocess.STDOUT, timeout=3600, check=False,
                )
                data, code = execution.stdout, execution.returncode
            except subprocess.TimeoutExpired as exc:
                data = (exc.stdout or b"") + b"\nTIMEOUT after 3600s\n"
                code = 124
            except OSError as exc:
                data, code = str(exc).encode(), 127
            logfile = output / (name + ".log")
            logfile.write_bytes(data)
            report["tests"].append({
                "name": name, "command": command, "exit_code": code,
                "log_file": logfile.name, "log_sha256": digest(data),
            })
            print(("PASS" if code == 0 else "FAIL"), name, "exit", code, flush=True)
            if code:
                passed = False
    finally:
        report["status"] = (
            "passed" if passed and len(report["tests"]) == len(SUITES) else "failed"
        )
        target = output / "manifest.json"
        target.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
        print("RESULT", report["status"], "revision", report["repo_revision"], flush=True)
    return 0 if report["status"] == "passed" else 1

if __name__ == "__main__":
    sys.exit(main())
