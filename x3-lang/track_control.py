#!/usr/bin/env python3
"""Shared X3Lang policy preflight for AI Security, Agent Economy and DeFi.

This is a local, deterministic policy preview. It cannot authorize a wallet,
prove an on-chain Guard decision, verify a quote or establish settlement.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import sys
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

import near_intents as ni

TRACKS = ("AI Security & Defense", "Agent Economy", "Unstoppable DeFi")


def evaluate(mandate_path: str, intent_path: str, tokens: list[dict[str, Any]], *,
             now: datetime | None = None) -> dict[str, Any]:
    """Compile an X3 mandate and intent, then apply the contract-parity checks locally.

    No HTTP calls, nonce consumption, signing or deposits. `preflight_passed`
    means the *local* policy check passed, not that an agent may spend funds.
    """
    now = now or datetime.now(timezone.utc)
    if now.tzinfo is None or now.utcoffset() is None:
        raise ValueError("now must be timezone-aware")
    if not isinstance(tokens, list):
        raise ValueError("tokens must be a list of 1Click token records")

    mandate = ni.mandates().compile_mandate(mandate_path, tokens, now=now)
    _, leg = ni.compile_program(intent_path, now=now)
    intent_sha = hashlib.sha256(Path(intent_path).read_bytes()).hexdigest()
    result: dict[str, Any] = {
        "schema": "x3.track-control.local.v1",
        "tracks": list(TRACKS),
        "mandate_sha256": mandate["terms"]["program_sha256"],
        "intent_sha256": intent_sha,
        "authorization": "not_verified",
        "wallet_enforced": False,
        "quote_verified": False,
        "settlement_proven": False,
        "evidence_level": "local_policy_preview_only",
        "checks": [],
        "approvals_needed": 0,
    }
    if leg.get("mandate_owner") != mandate["owner"]:
        result.update(decision="denied_by_policy", denied_guards=["mandate_owner"])
        return result

    request = ni.build_quote_request(leg, tokens, dry=True)
    policy = ni.policy_for(leg, request, intent_sha)
    preview_state = {**mandate, "version": None, "revoked": False, "spends": []}
    checks = ni.mandates().check_mandate(preview_state, policy, int(now.timestamp() * 1000))
    denied = [check["guard"] for check in checks if not check["ok"]]
    approvals = ni.mandates().approvals_needed(preview_state, int(policy["amount"]))
    result.update(checks=checks, denied_guards=denied, approvals_needed=approvals)
    if denied:
        result["decision"] = "denied_by_policy"
    elif approvals:
        result["decision"] = "approval_required"
    else:
        result["decision"] = "preflight_passed"
    return result


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Local, non-authorizing X3Lang cross-track policy preview")
    parser.add_argument("--mandate", required=True, help="Owner mandate .x3")
    parser.add_argument("--intent", required=True, help="Agent payment intent .x3")
    parser.add_argument("--tokens", required=True, help="Local 1Click token snapshot JSON; not live evidence")
    parser.add_argument("--at", help="UTC ISO-8601 timestamp for reproducible evaluation")
    args = parser.parse_args(argv)
    try:
        now = ni.parse_timestamp(args.at) if args.at else None
        tokens = json.loads(Path(args.tokens).read_text(encoding="utf-8"))
        report = evaluate(args.mandate, args.intent, tokens, now=now)
    except (OSError, ValueError, TypeError, KeyError, json.JSONDecodeError, ni.NearIntentsError) as exc:
        message = exc.code if isinstance(exc, ni.NearIntentsError) else type(exc).__name__
        print(json.dumps({"schema": "x3.track-control.local.v1", "decision": "invalid_input",
                          "error": message, "authorization": "not_verified"}), file=sys.stdout)
        return 2
    print(json.dumps(report, sort_keys=True, indent=2))
    return 0 if report["decision"] == "preflight_passed" else 6


if __name__ == "__main__":
    raise SystemExit(main())
