#!/usr/bin/env python3
"""Exact-head X3Lang local compiler/VM proof. Not a live-chain settlement proof."""
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
INPUTS = (
    "x3-lang/compiler/tests/test_control_flow_e2e.rs",
    "x3-lang/compiler/tests/test_bytecode_version_gate.rs",
    "x3-lang/vm/tests/test_nonce_replay.rs",
    "x3-lang/vm/tests/trading_receipts.rs",
)
TESTS = (
    ("source_to_bytecode_to_vm", "x3-lang-compiler", "test_control_flow_e2e"),
    ("bytecode_version_admission", "x3-lang-compiler", "test_bytecode_version_gate"),
    ("nonce_replay_guard", "x3-lang-vm", "test_nonce_replay"),
    ("signed_receipt_and_economic_replay", "x3-lang-vm", "trading_receipts"),
)


def git(*args: str) -> str:
    return subprocess.check_output(
        ["git", "-C", str(ROOT), *args], text=True, stderr=subprocess.PIPE
    ).strip()


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output-dir", required=True, type=Path,
                        help="report directory outside the checkout or inside a Cargo target directory")
    args = parser.parse_args()
    output = args.output_dir.resolve()

    missing = [path for path in INPUTS if not (ROOT / path).is_file()]
    if missing:
        parser.error(f"required test is missing: {missing[0]}")
    try:
        before = begin(ROOT, output)
        verify_committed_source(ROOT, output_dir=output)
    except InputIntegrityError as exc:
        parser.error(f"source binding refused: {exc}")
    except SourceBindingError as exc:
        parser.error(f"source binding refused: {exc}")
    for path in INPUTS:
        subprocess.run(["git", "-C", str(ROOT), "ls-files", "--error-unmatch", path],
                       stdout=subprocess.DEVNULL, check=True)
    output.mkdir(parents=True, exist_ok=True)
    rev = before["revision"]
    report = {
        "schema": "x3lang-local-execution-proof/v2",
        "status": "failed",
        "repo_revision": rev,
        "repo_tree": before["tree"],
        "source_binding": {
            "before": before,
            "after": None,
            "byte_and_mode_check": "committed checkout checked before and after tests; "
            "toolchain, dependencies and environment not attested",
        },
        "repository": git("config", "--get", "remote.origin.url"),
        "observed_utc": datetime.now(timezone.utc).isoformat(),
        "scope": "local Rust compiler/VM tests only; no EVM, SVM, NEAR or settlement finality proof",
        "inputs_sha256": {p: sha256((ROOT / p).read_bytes()) for p in INPUTS},
        "toolchain": {},
        "tests": [],
    }
    for tool in ("rustc", "cargo"):
        try:
            report["toolchain"][tool] = subprocess.check_output(
                [tool, "--version"], text=True, stderr=subprocess.STDOUT
            ).strip()
        except (OSError, subprocess.CalledProcessError) as exc:
            report["toolchain"][tool] = f"unavailable: {exc}"

    environment = dict(os.environ)
    environment["CARGO_INCREMENTAL"] = "0"
    passed = True
    try:
        for name, package, test in TESTS:
            command = [
                "cargo", "test", "--manifest-path", "x3-lang/Cargo.toml",
                "--locked", "-p", package, "--test", test,
            ]
            print(f"RUN {name}: {' '.join(command)}", flush=True)
            try:
                process = subprocess.run(
                    command, cwd=ROOT, env=environment,
                    stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                    timeout=1800, check=False,
                )
                data = process.stdout
                code = process.returncode
            except subprocess.TimeoutExpired as exc:
                data = (exc.stdout or b"") + b"\nTIMEOUT after 1800 seconds\n"
                code = 124
            except OSError as exc:
                data = str(exc).encode()
                code = 127
            logfile = output / f"{name}.log"
            logfile.write_bytes(data)
            summaries = re.findall(rb"test result: ok\.\s+(\d+) passed;\s+(\d+) failed", data)
            cases = sum(int(successes) for successes, _ in summaries)
            if code == 0 and (not summaries or cases == 0):
                print(f"FAIL {name}: no executed test cases found", flush=True)
                code = 65
            report["tests"].append({
                "name": name, "command": command, "exit_code": code,
                "test_cases_passed": cases,
                "log_sha256": sha256(data),
                "log_file": logfile.name,
            })
            if code:
                passed = False
                print(f"FAIL {name}: exit={code}; see {logfile}", flush=True)
            else:
                print(f"PASS {name}", flush=True)
    finally:
        try:
            verify_committed_source(ROOT, output_dir=output, expected_revision=rev)
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
        report["status"] = "passed" if passed and len(report["tests"]) == len(TESTS) else "failed"
        path = output / "manifest.json"
        path.write_text(json.dumps(report, sort_keys=True, indent=2) + "\n")
        print(f"RESULT {report['status']} revision={rev} report={path}", flush=True)
    return 0 if report["status"] == "passed" else 1


if __name__ == "__main__":
    sys.exit(main())
