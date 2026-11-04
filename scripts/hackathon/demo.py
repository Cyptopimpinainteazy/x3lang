#!/usr/bin/env python3
"""Print the three-track X3Lang judge demonstration without network or wallet access.

    python3 scripts/hackathon/demo.py
    python3 scripts/hackathon/demo.py --json

The scenario definitions and policy decisions come from the existing judge-console builder,
which evaluates them with `track_control.evaluate`. This entrypoint only formats those facts
for a terminal and adds the compiled NEAR Intents leg. It does not sign, quote, deploy or send.
"""
from __future__ import annotations

import argparse
import importlib.util
import json
import sys
import tempfile
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[2]
X3 = ROOT / "x3-lang"
sys.path.insert(0, str(X3))

import near_intents as ni  # noqa: E402


def _scenario_builder():
    path = X3 / "near" / "demo" / "judge" / "build_scenarios.py"
    spec = importlib.util.spec_from_file_location("x3_judge_build_scenarios", path)
    module = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    spec.loader.exec_module(module)
    return module


BUILDER = _scenario_builder()
DEMO_SCENARIOS = ("authorized", "needs-approval", "wrong-recipient")


def _cross_chain(source: str, tokens: list[dict[str, Any]]) -> dict[str, Any]:
    """Compile one scenario through the existing parser/lowering/request path."""
    with tempfile.TemporaryDirectory() as tmp:
        path = Path(tmp) / "scenario.x3"
        path.write_text(source, encoding="utf-8")
        _, leg = ni.compile_program(str(path), now=BUILDER.NOW)
        request = ni.build_quote_request(leg, tokens, dry=True)
    return {
        "origin_asset": _x3_asset(leg["origin"]),
        "destination_asset": _x3_asset(leg["destination"]),
        "amount": int(leg["amount"]),
        "min_receive": leg.get("min_receive"),
        "recipient": leg["recipient"],
        "refund_to": leg["refund_to"],
        "deadline": leg["deadline"],
        "slippage_bps": leg["slippage_bps"],
        "fee_cap_bps": leg.get("fee_cap_bps"),
        "request_is_dry": request["dry"],
        "authorization": "not_verified",
    }


def _x3_asset(endpoint: dict[str, Any]) -> str:
    chain = str(endpoint["chain"])
    return f"{chain[:1].upper()}{chain[1:]}.{endpoint['asset']}"


def build_report() -> dict[str, Any]:
    data = BUILDER.build()
    selected = []
    for scenario in data["scenarios"]:
        if scenario["id"] not in DEMO_SCENARIOS:
            continue
        selected.append({
            "id": scenario["id"],
            "group": scenario["group"],
            "title": scenario["title"],
            "story": scenario["story"],
            "intent_file": scenario["intent"]["file"],
            "intent_sha256": scenario["intent"]["sha256"],
            "intent_source": scenario["intent"]["source"],
            "edits": scenario["intent"]["edits"],
            "decision": scenario["python"]["decision"],
            "denied_guards": scenario["python"]["denied_guards"],
            "approvals_needed": scenario["python"]["approvals_needed"],
            "checks": scenario["python"]["checks"],
            "cross_chain": _cross_chain(scenario["intent"]["source"], data["tokens"]["records"]),
        })
    return {
        "schema": "x3.hackathon.demo.v1",
        "evidence_level": "local_policy_preview_only",
        "evaluated_at": data["evaluated_at"],
        "tracks": data["tracks"],
        "mandate": data["mandate"],
        "scenarios": selected,
        "evidence_boundaries": {
            "local_policy": "proven for the checked-in scenarios",
            "wallet_authorization": "not_verified",
            "onchain_authorization": "not_verified",
            "signed_quote": "not_requested",
            "observed_transaction": "not_present",
            "finalized_settlement": "not_present",
            "cross_chain_causality": "not_proven",
        },
    }


def _label(decision: str) -> str:
    return {
        "preflight_passed": "Passes the local policy check",
        "approval_required": "Needs approval",
        "denied_by_policy": "Refused",
    }.get(decision, decision)


def render(report: dict[str, Any]) -> str:
    lines: list[str] = [
        "X3LANG THREE-TRACK JUDGE DEMONSTRATION",
        "One owner mandate; one policy authority; three hackathon tracks.",
        "Evidence level: local policy preview only. No network request was made.",
        "",
        "PANEL 1 - THE OWNER'S X3LANG POLICY",
        f"File: {report['mandate']['file']}",
        f"SHA-256: {report['mandate']['sha256']}",
        *[f"  {line}" for line in report["mandate"]["source"].splitlines()],
        "",
        "PANEL 2 - AGENT REQUESTS",
        "These are checked-in agent programs; this offline demo does not call an AI provider.",
    ]
    for scenario in report["scenarios"]:
        lines.extend([
            f"[{scenario['group']}] {scenario['title']} ({scenario['id']})",
            f"  {scenario['story']}",
            f"  Program: {scenario['intent_file']}",
            f"  SHA-256: {scenario['intent_sha256']}",
            *[f"  {line}" for line in scenario["intent_source"].splitlines()],
            "",
        ])
    lines.extend([
        "PANEL 3 - SECURITY DECISION",
        "  Authorized         -> " + _label(report["scenarios"][0]["decision"]),
        "  Above threshold    -> " + _label(report["scenarios"][1]["decision"]),
        "  Wrong recipient    -> " + _label(report["scenarios"][2]["decision"]),
    ])
    for scenario in report["scenarios"]:
        failed = [check["guard"] for check in scenario["checks"] if not check["ok"]]
        lines.append(
            f"  {scenario['id']}: decision={scenario['decision']}, "
            f"failed_guards={failed or 'none'}, approvals={scenario['approvals_needed']}"
        )
    lines.extend([
        "",
        "PANEL 4 - CROSS-CHAIN EXECUTION PATH",
    ])
    for scenario in report["scenarios"]:
        leg = scenario["cross_chain"]
        lines.append(
            f"  {scenario['id']}: {leg['amount']} base units "
            f"{leg['origin_asset']} -> {leg['destination_asset']}; "
            f"min_receive={leg['min_receive']}; recipient={leg['recipient']}; "
            f"refund_to={leg['refund_to']}"
        )
    lines.extend([
        "  1Click may later provide the route; this demo uses the archived token fixture only.",
        "  1Click is not contacted. A dry quote would still not be a transaction or settlement.",
        "",
        "PANEL 5 - VERIFICATION AND LIMITS",
        "  Proven now: deterministic local policy checks over the checked-in X3 sources.",
        "  Browser parity: run node --test x3-lang/near/demo/test/*.test.mjs",
        "  Browser console: python3 -m http.server 8000 --directory x3-lang/near/demo",
        "                   then open http://localhost:8000/judge.html",
        "  Full local evidence: python3 scripts/hackathon/verify.sh",
        "  No wallet signature, on-chain authorization, signed quote or settlement is claimed.",
        "",
        "Tracks: " + ", ".join(report["tracks"]),
    ])
    return "\n".join(lines) + "\n"


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--json", action="store_true", help="emit the complete machine-readable report")
    args = parser.parse_args(argv)
    report = build_report()
    expected = {
        "authorized": ("preflight_passed", []),
        "needs-approval": ("approval_required", []),
        "wrong-recipient": ("denied_by_policy", ["mandate_recipient"]),
    }
    ok = all(
        (scenario["decision"], scenario["denied_guards"]) == expected[scenario["id"]]
        for scenario in report["scenarios"]
    )
    if args.json:
        print(json.dumps(report, indent=2, sort_keys=True))
    else:
        print(render(report), end="")
    return 0 if ok else 1


if __name__ == "__main__":
    raise SystemExit(main())
