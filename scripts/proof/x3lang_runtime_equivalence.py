#!/usr/bin/env python3
"""Fail-closed exact-head proof of local X3 canonical-runtime invariants.

Requires the full xxxstar workspace. Not a live chain or cross-chain proof.
"""
from __future__ import annotations
import argparse
import hashlib
import json
import re
import os
from pathlib import Path
import subprocess
import sys
from datetime import datetime, timezone

from checkout_integrity import InputIntegrityError, begin, finish
from source_binding import SourceBindingError, verify_committed_source

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
        ["git", "-C", str(ROOT), *args], text=True, stderr=subprocess.PIPE
    ).strip()

def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output-dir", required=True, type=Path,
                        help="report directory outside the checkout")
    args = parser.parse_args()
    output = args.output_dir.resolve()

    files = (MANIFEST,) + tuple(
        f"crates/x3-integration/tests/{test}.rs" for _, test, _ in SUITES
    )
    missing = [f for f in files if not (ROOT / f).is_file()]
    if missing:
        parser.error(
            f"canonical runtime workspace absent: {missing}. "
            "Run this suite in xxxstar; standalone x3lang only mirrors the runner."
        )
    try:
        before = begin(ROOT, output)
        verify_committed_source(ROOT, output_dir=output)
    except InputIntegrityError as exc:
        parser.error(f"source binding refused: {exc}")
    except SourceBindingError as exc:
        parser.error(f"source binding refused: {exc}")
    for relative in files:
        subprocess.run(
            ["git", "-C", str(ROOT), "ls-files", "--error-unmatch", relative],
            stdout=subprocess.DEVNULL, check=True,
        )
    output.mkdir(parents=True, exist_ok=True)
    report = {
        "schema": "x3lang-local-runtime-equivalence/v2",
        "status": "failed",
        "scope": (
            "local X3 runtime, three execution paths, gas, opcode, rollback, "
            "signed receipt replay; NOT EVM/SVM/NEAR settlement or chain finality"
        ),
        "repo_revision": before["revision"],
        "repo_tree": before["tree"],
        "source_binding": {
            "before": before,
            "after": None,
            "byte_and_mode_check": "committed checkout checked before and after tests; "
            "toolchain, dependencies and environment not attested",
        },
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
            summaries = re.findall(rb"test result: ok\.\s+(\d+) passed;\s+(\d+) failed", data)
            cases = sum(int(successes) for successes, _ in summaries)
            if code == 0 and (not summaries or cases == 0):
                code = 65  # Empty suite is never evidence of equivalence.
            report["tests"].append({
                "name": name, "command": command, "exit_code": code,
                "test_cases_passed": cases,
                "log_file": logfile.name, "log_sha256": digest(data),
            })
            print(("PASS" if code == 0 else "FAIL"), name, "exit", code, "cases", cases, flush=True)
            if code:
                passed = False
    finally:
        try:
            verify_committed_source(
                ROOT, output_dir=output, expected_revision=before["revision"]
            )
            report["source_binding"]["after"] = finish(ROOT, before)
        except (InputIntegrityError, SourceBindingError) as exc:
            passed = False
            message = str(exc)
            if "differs from HEAD" in message or "untracked input" in message:
                message = f"uncommitted proof checkout inputs: {message}"
            if isinstance(exc, SourceBindingError):
                report["source_binding_error"] = str(exc)
                if message.startswith("HEAD changed"):
                    message = message.replace("HEAD changed", "HEAD/tree changed", 1)
            report["input_error"] = message
            print(f"FAIL source integrity: {message}", file=sys.stderr, flush=True)
        report["status"] = (
            "passed" if passed and len(report["tests"]) == len(SUITES) else "failed"
        )
        target = output / "manifest.json"
        target.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
        print("RESULT", report["status"], "revision", report["repo_revision"], flush=True)
    return 0 if report["status"] == "passed" else 1

if __name__ == "__main__":
    sys.exit(main())
