#!/usr/bin/env python3
"""Produce sanitized, read-only BNB -> NEAR readiness evidence for the judge page.

Never broadcasts, signs, consumes a nonce, or creates a live deposit address.
"""
import argparse
import datetime as dt
import json
import subprocess
import sys
from pathlib import Path

SCHEMA = "x3.judge.bnb-readiness.v1"


def extract_result(stdout):
    try:
        doc = json.loads(stdout)
    except (TypeError, ValueError):
        return {"kind": "unreadable_response"}
    if doc.get("status") == "planned":
        req = doc.get("quote_request", {})
        leg = doc.get("leg", {})
        return {
            "kind": "planned",
            "source_chain": leg.get("origin", {}).get("chain"),
            "source_asset": leg.get("origin", {}).get("asset"),
            "destination_chain": leg.get("destination", {}).get("chain"),
            "destination_asset": leg.get("destination", {}).get("asset"),
            "amount_base_units": str(req.get("amount", "")),
            "dry": req.get("dry") is True,
        }
    errors = doc.get("errors") or []
    if errors:
        codes = [str(e.get("code", "")) for e in errors if isinstance(e, dict)]
        minimums = [
            str((e.get("detail") or {}).get("providerMinimumUsd"))
            for e in errors if isinstance(e, dict) and (e.get("detail") or {}).get("providerMinimumUsd") is not None
        ]
        return {"kind": "rejected", "error_codes": codes, "provider_minimum_usd": minimums[0] if minimums else None}
    if doc.get("status"):
        return {"kind": str(doc["status"]), "funded_transfer_proven": False}
    return {"kind": "unexpected_response"}


def run_read_only(connector, intent, action, timeout=30):
    script = connector / "x3-lang" / "near_intents.py"
    command = [sys.executable, str(script), action, str(intent)]
    try:
        result = subprocess.run(command, text=True, capture_output=True, timeout=timeout, check=False, cwd=connector)
        return {"exit_code": result.returncode, **extract_result(result.stdout or result.stderr)}
    except (OSError, subprocess.TimeoutExpired):
        return {"kind": "execution_unavailable", "exit_code": None}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--connector", required=True, type=Path)
    parser.add_argument("--intent", type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    connector = args.connector.resolve()
    intent = (args.intent or connector / "x3-lang/examples/bsc_bnb_to_near_wnear_dry.x3").resolve()
    if not intent.is_file():
        parser.error("The read-only example program does not exist")
    if not (connector / "x3-lang/near_intents.py").is_file():
        parser.error("The BNB connector script does not exist")
    program = intent.read_text(encoding="utf-8")
    # This evidence generator is deliberately restricted to the published dry example.
    # Do not accidentally pass a real funded wallet intent into a public demo artifact.
    if "DRY QUOTE REFERENCE ONLY" not in program:
        parser.error("Only the known non-funded dry example is permitted")
    plan = run_read_only(connector, intent, "plan")
    quote = run_read_only(connector, intent, "quote")  # No --live, no deposit address.
    report = {
        "schema": SCHEMA,
        "recorded_at": dt.datetime.now(dt.timezone.utc).isoformat(),
        "mode": "read_only_dry_quote",
        "connector_revision": subprocess.run(
            ["git", "-C", str(connector), "rev-parse", "HEAD"],
            text=True, capture_output=True, check=False).stdout.strip(),
        "connector_worktree_clean": subprocess.run(
            ["git", "-C", str(connector), "status", "--porcelain"],
            text=True, capture_output=True, check=False).stdout.strip() == "",
        "plan": plan,
        "quote": quote,
        "wallet_ownership_proven": False,
        "near_recipient_owner_verified": False,
        "live_guard_authorization_proven": False,
        "live_settlement_proven": False,
        "transaction_hashes": [],
        "warning": "This program uses deliberately unusable placeholder wallet addresses. Never fund this quote.",
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"report": str(args.output), "plan": plan["kind"], "quote": quote["kind"],
                      "provider_minimum_usd": quote.get("provider_minimum_usd")}, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
