#!/usr/bin/env python3
"""Build a machine-readable hackathon readiness report from recorded test evidence.

    python3 scripts/hackathon/readiness.py --evidence x3-lang/near/demo/judge/evidence.json

This program is a projection, not a second test authority. It reads the evidence file written by
`x3-lang/near/demo/judge/collect_evidence.py`, requires that evidence to describe the current
clean commit, and reports local facts separately from unverified wallet, chain and settlement
claims. It prints JSON to stdout and can also write the same JSON with `--out`.
"""
from __future__ import annotations

import argparse
import json
import subprocess
import sys
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[2]
DEFAULT_EVIDENCE = ROOT / "x3-lang" / "near" / "demo" / "judge" / "evidence.json"

REQUIRED_RUNS = {
    "python": "Policy checker, compiler and adapters (Python)",
    "browser": "Browser checker and console (Node)",
    "scenarios": "Console scenarios match the examples",
    "demo": "Offline five-panel judge demo",
    "contract": "x3-guard contract (Rust)",
}

REQUIRED_CI_CHECKS = [
    "provenance",
    "python-and-browser",
    "rust-vm",
    "near-guard",
    "GitGuardian Security Checks",
]


def _git_head() -> str:
    return subprocess.run(
        ["git", "rev-parse", "HEAD"],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=True,
    ).stdout.strip()


def _run_map(evidence: dict[str, Any]) -> dict[str, dict[str, Any]]:
    return {
        run["name"]: run
        for run in evidence.get("runs", [])
        if isinstance(run, dict) and isinstance(run.get("name"), str)
    }


def _state(run: dict[str, Any] | None) -> str:
    if run is None or run.get("exit_code") is None:
        return "not_run"
    return "passed" if run.get("exit_code") == 0 else "failed"


def build_report(evidence: dict[str, Any], *, head: str) -> dict[str, Any]:
    runs = _run_map(evidence)
    checks: dict[str, dict[str, Any]] = {}
    blockers: list[str] = []
    for key, name in REQUIRED_RUNS.items():
        run = runs.get(name)
        state = _state(run)
        checks[key] = {
            "state": state,
            "evidence_name": name,
            "exit_code": None if run is None else run.get("exit_code"),
            "passed": None if run is None else run.get("passed"),
            "failed": None if run is None else run.get("failed"),
            "skipped": None if run is None else run.get("skipped"),
        }
        if state == "not_run":
            blockers.append(f"{key}_evidence_not_run")
        elif state == "failed":
            blockers.append(f"{key}_evidence_failed")

    evidence_commit = evidence.get("commit")
    if evidence_commit != head:
        blockers.append("evidence_commit_does_not_match_head")
    if evidence.get("worktree_clean") is not True:
        blockers.append("evidence_worktree_was_dirty")

    local_ready = not blockers
    verified = [
        {
            "id": "local_policy",
            "claim": "Both the mandate and payment compile, and the shared local policy checker accepts, "
                     "denies or requires approval as displayed.",
            "classification": "verified_locally",
            "evidence": checks["python"]["evidence_name"],
        },
        {
            "id": "browser_policy_parity",
            "claim": "The browser policy checker reaches Python's decision through the same guard sequence.",
            "classification": "verified_locally",
            "evidence": checks["browser"]["evidence_name"],
        },
        {
            "id": "scenario_freshness",
            "claim": "The checked-in judge scenarios are current with their source examples.",
            "classification": "verified_locally",
            "evidence": checks["scenarios"]["evidence_name"],
        },
        {
            "id": "offline_judge_demo",
            "claim": "The one-command terminal walkthrough produces the three expected policy decisions.",
            "classification": "verified_locally",
            "evidence": checks["demo"]["evidence_name"],
        },
    ]
    if checks["contract"]["state"] == "passed":
        verified.append({
            "id": "contract_tests",
            "claim": "The existing x3-guard Rust tests pass, including mandate and quote enforcement cases.",
            "classification": "verified_locally",
            "evidence": checks["contract"]["evidence_name"],
        })
    unproven = [
        {
            "id": "onchain_authorization",
            "claim": "An owner-approved request was authorized by a deployed x3-guard contract.",
            "missing_evidence": "deployed contract account plus successful register/approve/accept transactions",
        },
        {
            "id": "onchain_refusal",
            "claim": "An unauthorized request was refused on NEAR.",
            "missing_evidence": "failed on-chain transaction and exact X3_MANDATE_DENIED reason",
        },
        {
            "id": "wallet_signed_transaction",
            "claim": "A real wallet signed a quote or transferred funds.",
            "missing_evidence": "wallet-signed transaction and chain observation",
        },
        {
            "id": "live_signed_quote",
            "claim": "A NEAR Intents quote was independently verified from a live response.",
            "missing_evidence": "live quote artifact bound to the submitted program and provider signature",
        },
        {
            "id": "settlement",
            "claim": "A cross-chain payment settled.",
            "missing_evidence": "independently observed source and destination chain transactions",
        },
        {
            "id": "cross_chain_causality",
            "claim": "One chain transaction caused the other.",
            "missing_evidence": "cryptographic or solver-attribution proof of cross-chain causality",
        },
    ]
    return {
        "schema": "x3.hackathon.readiness.v1",
        "repository": "Cyptopimpinainteazy/x3lang",
        "head": head,
        "evidence_commit": evidence_commit,
        "evidence_worktree_clean": evidence.get("worktree_clean"),
        "evidence_recorded_at": evidence.get("recorded_at"),
        "local_verification_ready": local_ready,
        "submission_ready": False,
        "checks": checks,
        "blockers": blockers,
        "verified": verified,
        "unproven": unproven,
        "documented_only": [
            "A public judge-console deployment is not asserted by this report.",
            "The NEAR Guard public page is a signed dry-quote demo, not a completed payment.",
            "Historical X3Lang compiler and VM work predates this hackathon; see PRIOR_WORK.md.",
        ],
        "required_external_ci": {
            "checks": REQUIRED_CI_CHECKS,
            "status": "not_queried_by_this_offline_script",
        },
        "existing_verified_near_integration": {
            "status": "contract_tests_passed" if checks["contract"]["state"] == "passed" else "unverified",
            "scope": "Rust x3-guard mandate and quote tests; no deployment or live transaction is asserted",
            "evidence": checks["contract"]["evidence_name"],
        },
        "public_demo": {
            "offline_command": "python3 scripts/hackathon/demo.py",
            "browser_command": "python3 -m http.server 8000 --directory x3-lang/near/demo",
            "browser_url": "http://localhost:8000/judge.html",
            "external_deployment": "not_asserted",
        },
        "remaining_submission_tasks": [
            "Re-run the recorder on the final clean commit.",
            "Wait for every required GitHub check on that exact commit.",
            "Obtain owner authorization for any deployment or funded transaction.",
            "Record independently verified live transaction evidence if the owner performs one.",
            "Verify the organizer's current submission rules and complete their form.",
        ],
    }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--evidence", default=str(DEFAULT_EVIDENCE), help="collect_evidence.py output")
    parser.add_argument("--out", help="also write the report to this path")
    parser.add_argument("--strict-submission", action="store_true",
                        help="exit 1 unless the live submission evidence is also complete")
    args = parser.parse_args(argv)
    try:
        evidence = json.loads(Path(args.evidence).read_text(encoding="utf-8"))
        report = build_report(evidence, head=_git_head())
    except (OSError, ValueError, KeyError, json.JSONDecodeError, subprocess.CalledProcessError) as exc:
        print(json.dumps({
            "schema": "x3.hackathon.readiness.v1",
            "local_verification_ready": False,
            "submission_ready": False,
            "error": type(exc).__name__,
            "message": str(exc),
        }, indent=2))
        return 2
    text = json.dumps(report, indent=2) + "\n"
    if args.out:
        out = Path(args.out)
        out.parent.mkdir(parents=True, exist_ok=True)
        out.write_text(text, encoding="utf-8")
    print(text, end="")
    if not report["local_verification_ready"]:
        return 1
    return 1 if args.strict_submission and not report["submission_ready"] else 0


if __name__ == "__main__":
    raise SystemExit(main())
