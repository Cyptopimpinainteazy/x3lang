#!/usr/bin/env python3
"""Fail-closed source mutations of the Rust NEAR Guard, in disposable copies."""
import argparse
import hashlib
import json
import os
import re
import shutil
import subprocess
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
GUARD = Path("x3-lang/near/x3-guard")
# ID, source path, exact original expression, mutant, focused actual Rust test
MUTANTS = [
    ("signature_bypass", "src/lib.rs",
     "if !env::ed25519_verify(", "if false && !env::ed25519_verify(",
     "tampering_with_any_signed_byte_fails_verification"),
    ("nonce_replay_bypass", "src/lib.rs",
     "!self.intents.contains_key(&k)", "true",
     "a_nonce_registers_once"),
    ("duplicate_approval_bypass", "src/mandate.rs",
     "!record.approvals.contains(&approver)", "true",
     "payments_above_the_threshold_need_n_of_m_approvals"),
    ("expired_approval_bypass", "src/mandate.rs",
     "env::block_timestamp_ms() < m.terms.expires_at_ms", "true",
     "expired_mandate_cannot_record_late_approvals"),
    ("recipient_allowlist_bypass", "src/mandate.rs",
     "t.recipients.contains(&policy.recipient)", "true",
     "every_term_refuses_an_intent_that_breaks_it"),
    ("payment_cap_bypass", "src/mandate.rs",
     "amount <= t.per_payment_cap.0", "true",
     "every_term_refuses_an_intent_that_breaks_it"),
]

def sha(data):
    return hashlib.sha256(data.encode()).hexdigest()

def run_test(crate, name, target_dir, timeout):
    env = dict(os.environ)
    env["CARGO_TARGET_DIR"] = str(target_dir)
    cmd = ["cargo", "test", "--offline", "--locked",
           "--manifest-path", str(crate / "Cargo.toml"), name, "--", "--nocapture"]
    started = time.monotonic()
    try:
        proc = subprocess.run(cmd, text=True, stdout=subprocess.PIPE,
                              stderr=subprocess.STDOUT, env=env, timeout=timeout)
    except subprocess.TimeoutExpired:
        return {"kind": "TIMEOUT", "exit": None, "tail": "timeout",
                "seconds": round(time.monotonic() - started, 2)}
    out = proc.stdout
    passed = bool(re.search(r"test result: ok\. 1 passed;", out))
    failed = bool(re.search(r"test result: FAILED\. 0 passed; 1 failed;", out))
    assertion = ("FAILED" in out and
                 ("panicked at " in out or "test did not panic as expected" in out
                  or "assertion" in out or "expected a panic" in out))
    if proc.returncode == 0 and passed:
        kind = "PASS"
    elif proc.returncode != 0 and failed and assertion:
        kind = "ASSERTION_FAILURE"
    else:
        kind = "INVALID"
    return {"kind": kind, "exit": proc.returncode,
            "seconds": round(time.monotonic() - started, 2),
            "command": cmd, "tail": out[-1800:]}

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", default="/tmp/x3-guard-rust-mutations.json")
    parser.add_argument("--timeout", type=int, default=180)
    parser.add_argument("--target-dir", type=Path)
    parser.add_argument("--only", help="Single mutant ID for reproduction")
    args = parser.parse_args()
    if args.timeout < 1:
        parser.error("--timeout must be positive")
    selected = [m for m in MUTANTS if args.only is None or m[0] == args.only]
    if not selected:
        parser.error(f"unknown mutant: {args.only}")

    source = ROOT / GUARD
    if not (source / "Cargo.lock").is_file():
        parser.error("missing pinned Guard Cargo.lock")
    results = []
    with tempfile.TemporaryDirectory(prefix="x3-guard-mutants-") as tmp:
        scratch = Path(tmp)
        # Preserve Rust include_str! paths relative to the Guard crate.
        crate = scratch / "x3-lang" / "near" / "x3-guard"
        crate.parent.mkdir(parents=True)
        shutil.copytree(source, crate, ignore=shutil.ignore_patterns(
            "target", ".pytest_cache", ".venv", "__pycache__"))
        fixture_dst = scratch / "x3-lang" / "tests" / "fixtures"
        fixture_dst.parent.mkdir(parents=True)
        shutil.copytree(ROOT / "x3-lang" / "tests" / "fixtures", fixture_dst)
        target_dir = args.target_dir or scratch / "target"
        target_dir.mkdir(parents=True, exist_ok=True)

        # Original baseline must pass for EACH specific test before a score
        # can be calculated. No failed baseline is ever called a killed mutant.
        for name in sorted({m[4] for m in selected}):
            baseline = run_test(crate, name, target_dir, args.timeout)
            print(f"BASELINE {name}: {baseline['kind']}", flush=True)
            if baseline["kind"] != "PASS":
                print(baseline.get("tail", ""), flush=True)
                raise SystemExit(f"FAIL CLOSED: baseline did not pass {name}")

        for ident, relative, original, altered, test_name in selected:
            path = crate / relative
            pristine = (source / relative).read_text()
            path.write_text(pristine)
            count = pristine.count(original)
            if count != 1 or original == altered:
                item = {"id": ident, "status": "INVALID",
                        "detail": f"expected 1 exact source match, found {count}"}
            else:
                mutant = pristine.replace(original, altered, 1)
                path.write_text(mutant)
                observed = run_test(crate, test_name, target_dir, args.timeout)
                categories = {"ASSERTION_FAILURE": "KILLED", "PASS": "SURVIVED",
                              "INVALID": "INVALID", "TIMEOUT": "TIMEOUT"}
                item = {"id": ident, "status": categories[observed["kind"]],
                        "test": test_name, "file": relative, "original": original,
                        "mutated": altered, "base_sha256": sha(pristine),
                        "mutant_sha256": sha(mutant), **observed}
            path.write_text(pristine)
            results.append(item)
            print(f"MUTANT {ident}: {item['status']}", flush=True)

    counts = {name: sum(r["status"] == name for r in results)
              for name in ("KILLED", "SURVIVED", "INVALID", "TIMEOUT")}
    counted = counts["KILLED"] + counts["SURVIVED"]
    summary = {"total": len(results), "counts": counts,
               "score": round(100 * counts["KILLED"] / counted, 2) if counted else None,
               "scope": "native Rust Guard unit assertions only",
               "base_commit": subprocess.check_output(
                   ["git", "-C", str(ROOT), "rev-parse", "HEAD"],
                   text=True).strip()}
    report = {"summary": summary, "mutations": results}
    Path(args.output).write_text(json.dumps(report, indent=2))
    print(f"RESULTS {json.dumps(summary)}; report={args.output}", flush=True)
    # Invalid and timeout cannot inflate mutation score or pass the release gate.
    return 0 if counts == {"KILLED": len(results), "SURVIVED": 0,
                           "INVALID": 0, "TIMEOUT": 0} else 1

if __name__ == "__main__":
    raise SystemExit(main())
