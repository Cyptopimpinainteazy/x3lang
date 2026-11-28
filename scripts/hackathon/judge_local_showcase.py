#!/usr/bin/env python3
"""Fail-closed local judge showcase over existing X3 security demonstrations.

No blockchain, wallet, RPC or production side effects. Mainnet payment proof
is referenced, never rerun or inferred from these local demonstrations.
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]
SCRIPTS = ROOT / "scripts" / "hackathon"
GROK_CASES = frozenset({"permission_object_lifts_limits", "morse_encoded_transfer", "full_incident_chain"})
ACCOUNTING_CHECKS = frozenset({
    "original_payment_settles", "alternate_txid_cannot_trigger_second_withdrawal",
    "exactly_one_economic_debit", "changed_txid_linked_to_same_intent",
    "missing_reserves_freeze", "authorized_control_allowed",
})


def verify_report(name: str, result: dict) -> dict:
    """Return summary only for exact expected local outputs; refuse changed contracts."""
    if name == "grok":
        checks = (result.get("ok") is True and result.get("passed") == 3
                  and result.get("total") == 3
                  and result.get("control", {}).get("allowed") is True
                  and all(row.get("pass") is True for row in result.get("cases", []))
                  and len(result.get("cases", [])) == 3
                  and {row.get("case") for row in result["cases"]} == GROK_CASES)
        summary = "3/3 malicious local proposals refused; local control allowed"
    elif name == "agent_law":
        checks = (result.get("replay_blocked") is True
                  and result.get("retired_after_policy_violation") is True
                  and result.get("retirement_survives_restart") is True
                  and result.get("compliant_plan", {}).get("kind") == "local-compiled-plan-no-payment")
        summary = "signed local agent lifecycle; retirement survives restart"
    elif name == "mtgox":
        checks = (result.get("ok") is True and result.get("passed") == 6
                  and result.get("total") == 6
                  and set(result.get("checks", {})) == ACCOUNTING_CHECKS
                  and all(value is True for value in result["checks"].values()))
        summary = "6/6 synthetic accounting assertions"
    else:
        raise ValueError("unrecognized showcase stage")
    if not checks:
        raise ValueError(f"{name} evidence did not satisfy exact expected conditions")
    return {"stage": name, "status": "PASS", "summary": summary, "level": "LOCAL"}


def run_stage(name: str, script: str, args: list[str]) -> dict:
    completed = subprocess.run(
        [sys.executable, str(SCRIPTS / script), *args],
        cwd=ROOT,
        check=False,
        capture_output=True,
        text=True,
        timeout=120,
    )
    if completed.returncode != 0:
        raise RuntimeError(f"{name}: demonstration failed (exit {completed.returncode}): {completed.stderr[-900:]}")
    try:
        payload = json.loads(completed.stdout)
    except json.JSONDecodeError as exc:
        raise RuntimeError(f"{name}: expected JSON evidence") from exc
    return verify_report(name, payload)


def showcase() -> dict:
    steps = [
        ("grok", "replay_grok_wallet_drain.py", ["--json"]),
        ("agent_law", "agent_law_local_demo.py", []),
        ("mtgox", "replay_mtgox_accounting.py", ["--json"]),
    ]
    stages = [run_stage(*step) for step in steps]
    return {
        "ok": True,
        "classification": "LOCAL demonstration only; NOT mainnet hostile-agent defense",
        "stages": stages,
        "mainnet_proof": "PROOF_CARD.md (independent archived owner-assisted 25 USDC Solana-to-NEAR payment)",
        "limitations": [
            "The Grok scenario uses a scripted compliant model and off-chain checks.",
            "The Agent Law demonstration uses scripted jury keys and a local SQLite runtime.",
            "The Mt. Gox-inspired accounting model is synthetic and not deployed Guard.",
            "The Guard is noncustodial; owner/funding FullAccess keys can bypass it.",
            "No production agent, wallet, or chain is touched by this launcher.",
        ],
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--json", action="store_true")
    parser.add_argument("--out", type=Path, help="Save a summary report outside the source tree")
    args = parser.parse_args()
    try:
        report = showcase()
    except (RuntimeError, ValueError, subprocess.TimeoutExpired, OSError) as exc:
        print(f"FAIL: {exc}", file=sys.stderr)
        return 1
    if args.out:
        args.out.parent.mkdir(parents=True, exist_ok=True)
        args.out.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    if args.json:
        print(json.dumps(report, indent=2))
    else:
        for stage in report["stages"]:
            print(f"{stage['status']} [{stage['level']}] {stage['stage']}: {stage['summary']}")
        print("MAINNET proof is separate: PROOF_CARD.md; no on-chain operation was executed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
