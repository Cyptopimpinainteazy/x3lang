#!/usr/bin/env python3
"""Fail-closed source mutations of the x3-mpc-guard authority check, in disposable copies.

Adapted from guard_rust_mutations.py. Each mutant removes one check; the named
unit test must then fail with an assertion. Native unit tests only, no chain."""
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
GUARD = Path("x3-lang/near/x3-mpc-guard")
# ID, source path, exact original expression, mutant, focused actual Rust test
MUTANTS = [
    ("recipient_allowlist_bypass", "src/authority.rs",
     "&& t.recipients.contains(&req.recipient),", "&& true,",
     "a_recipient_off_the_allowlist_is_refused"),
    ("per_payment_cap_bypass", "src/authority.rs",
     "amount > 0 && amount <= t.per_payment_cap.0,", "amount > 0,",
     "per_payment_and_rolling_caps_are_enforced_at_request_time"),
    ("rolling_cap_bypass", "src/authority.rs",
     "spent_after.is_some_and(|total| total <= t.daily_cap.0)", "spent_after.is_some()",
     "per_payment_and_rolling_caps_are_enforced_at_request_time"),
    ("revocation_bypass", "src/authority.rs",
     "!m.revoked && now_ms < t.expires_at_ms,", "now_ms < t.expires_at_ms,",
     "a_revoked_agent_cannot_propose_or_request"),
    ("fee_cap_bypass", "src/authority.rs",
     "req.max_fee_per_gas.0 > 0 && req.max_fee_per_gas.0 <= t.max_fee_per_gas.0,",
     "req.max_fee_per_gas.0 > 0,",
     "fee_escalation_is_refused_field_by_field"),
    ("chain_id_bypass", "src/authority.rs",
     "req.chain_id == t.chain_id,", "true,",
     "a_wrong_chain_id_is_refused"),
    ("key_index_bypass", "src/authority.rs",
     "req.key_index == t.key_index,", "true,",
     "another_derived_account_is_refused"),
    ("program_hash_bypass", "src/authority.rs",
     "req.program_sha256 == t.program_sha256,", "true,",
     "wrong_asset_or_program_is_refused"),
    ("intent_expiry_bypass", "src/authority.rs",
     "now_ms < req.expires_at_ms\n", "true\n",
     "an_expired_intent_cannot_be_approved_or_signed"),
    ("approval_count_bypass", "src/lib.rs",
     "ok: have >= needed,", "ok: true,",
     "approvals_must_be_distinct_listed_and_bound_to_the_intent_hash"),
    ("approval_hash_binding_bypass", "src/lib.rs",
     "intent_hash == rec.intent_hash,", "true,",
     "approvals_must_be_distinct_listed_and_bound_to_the_intent_hash"),
    ("mandate_version_bypass", "src/lib.rs",
     '"mandate_version",\n            intent.mandate_version == m.version,',
     '"mandate_version",\n            true,',
     "replacing_the_mandate_voids_intents_and_approvals_made_under_the_old_one"),
    ("in_flight_bypass", "src/lib.rs",
     "ok: account.is_some_and(|a| a.in_flight.is_none()),", "ok: true,",
     "a_second_request_while_one_is_in_flight_is_refused"),
    ("evm_nonce_replay_bypass", "src/lib.rs",
     "ok: rec.spend_counted || intent.evm_nonce == next,", "ok: true,",
     "an_evm_nonce_is_signed_once"),
    ("intent_nonce_replay_bypass", "src/lib.rs",
     "!self.intents.contains_key(&intent_key(&owner, &request.intent_nonce))", "true",
     "an_intent_nonce_is_used_once"),
    ("intent_status_bypass", "src/lib.rs",
     "matches!(rec.status, SigningStatus::Proposed | SigningStatus::SignFailed),", "true,",
     "an_intent_nonce_is_used_once"),
    ("canonical_payload_bypass", "src/lib.rs",
     ".is_ok_and(|(bytes, hash)| *bytes == intent.unsigned_tx && *hash == intent.signing_hash)",
     ".is_ok()",
     "a_tampered_stored_payload_is_refused_before_signing"),
    ("signature_key_bypass", "src/lib.rs",
     "if recovered != expected {", "if false {",
     "a_signature_by_another_key_is_not_accepted"),
    ("unverified_account_bypass", "src/lib.rs",
     "ok: account.status == AccountStatus::Verified,", "ok: true,",
     "an_account_the_signer_disagrees_with_can_never_sign"),
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
    parser.add_argument("--output", default="/tmp/x3-mpc-guard-rust-mutations.json")
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
        parser.error("missing pinned x3-mpc-guard Cargo.lock")
    results = []
    with tempfile.TemporaryDirectory(prefix="x3-mpc-guard-mutants-") as tmp:
        scratch = Path(tmp)
        # Preserve Rust include_str! paths relative to the Guard crate.
        crate = scratch / "x3-lang" / "near" / "x3-mpc-guard"
        crate.parent.mkdir(parents=True)
        shutil.copytree(source, crate, ignore=shutil.ignore_patterns(
            "target", ".pytest_cache", ".venv", "__pycache__"))
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
               "scope": "native Rust x3-mpc-guard unit assertions only",
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
