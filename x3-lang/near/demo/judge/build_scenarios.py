#!/usr/bin/env python3
"""Write `scenarios.json` for the judge console (`../judge.html`).

Every scenario is the owner's mandate (`examples/near_mandate_agent.x3`) and one agent
payment: a checked-in example, or that example with one stated line changed. Each one is
evaluated here by `track_control.evaluate`, the Python policy authority, and the console
re-evaluates it in the browser with `x3guard.mjs` and shows whether the two agree.

    python3 x3-lang/near/demo/judge/build_scenarios.py           # rewrite scenarios.json
    python3 x3-lang/near/demo/judge/build_scenarios.py --check   # fail if it is stale

Nothing here signs, calls 1Click or NEAR RPC, or reads chain state. The token list is the
archived fixture, and the time is fixed so the output is reproducible.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import sys
import tempfile
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

X3 = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(X3))

import track_control as tc  # noqa: E402

OUT = Path(__file__).with_name("scenarios.json")
MANDATE = "examples/near_mandate_agent.x3"
PAYMENT = "examples/near_intents_agent_payment.x3"
DENIED = "examples/near_intents_agent_denied.x3"
TOKENS = "tests/fixtures/near_intents_tokens.json"
NOW = datetime(2026, 11, 3, 3, 0, 0, tzinfo=timezone.utc)
NOW_MS = int(NOW.timestamp() * 1000)
HOUR_MS = 3_600_000

GUARD_TESTS = "near/x3-guard/src"

DEMO_STEPS = [
    {"number": 1, "title": "Read the owner's policy",
     "body": "The mandate is the source of truth: assets, recipients, caps, approval and refund rules."},
    {"number": 2, "title": "Inspect the agent's request",
     "body": "Every scenario starts from a checked-in X3 program and shows the exact edited lines."},
    {"number": 3, "title": "Follow the security decision",
     "body": "Python and the browser use the same guard order and must reach the same verdict."},
    {"number": 4, "title": "Separate policy from settlement",
     "body": "A local pass is not a wallet approval, on-chain authorization or completed payment."},
    {"number": 5, "title": "Check the evidence",
     "body": "The final panel records which tests ran and names every claim that remains unproven."},
]

TRACK_GUIDE = [
    {"track": "AI Security & Defense",
     "role": "Deny substitutions, overspend, fee evasion, replay and revoked authority."},
    {"track": "Agent Economy",
     "role": "Allow bounded payments and distinguish approval-required from authorized."},
    {"track": "Unstoppable DeFi",
     "role": "Lower authorized intent into one inspectable NEAR Intents route without claiming settlement."},
]

# (id, group, title, story, source file, [(old, new)], assumed Guard state, contract tests)
SCENARIOS: list[dict[str, Any]] = [
    {"id": "authorized", "group": "A", "title": "Authorized payment",
     "story": "The agent pays 25 USDC to a listed recipient, inside every limit.",
     "takeaway": "All mandate checks pass; the payment is still only locally eligible.",
     "source": PAYMENT, "edits": []},
    {"id": "needs-approval", "group": "A", "title": "Payment that needs approval",
     "story": "40 USDC is under the 50 USDC cap but over the 30 USDC approval threshold.",
     "takeaway": "Every limit passes, but the listed owner must approve before acceptance.",
     "source": PAYMENT, "edits": [("amount 25000000", "amount 40000000"),
                                  ("min_receive 24800000", "min_receive 39680000"),
                                  ("// 25 USDC is under the mandate's 30 USDC approval threshold, so no approval is needed.",
                                   "// 40 USDC is below the 50 USDC cap but above the 30 USDC approval threshold.")],
     "contract_tests": [f"{GUARD_TESTS}/mandate_tests.rs::payments_above_the_threshold_need_n_of_m_approvals"]},
    {"id": "over-limit", "group": "B", "title": "Exceeds the spending limit",
     "story": "60 USDC in one payment, over the owner's 50 USDC cap.",
     "takeaway": "The amount is safe to encode but outside the owner's authority.",
     "source": PAYMENT, "edits": [("amount 25000000", "amount 60000000"),
                                  ("min_receive 24800000", "min_receive 59520000"),
                                  ("// 25 USDC is under the mandate's 30 USDC approval threshold, so no approval is needed.",
                                   "// 60 USDC exceeds the owner's 50 USDC per-payment cap.")]},
    {"id": "wrong-recipient", "group": "B", "title": "Unauthorized recipient",
     "story": "The agent tries to pay attacker.near, who is not on the owner's list.",
     "takeaway": "Recipient substitution stops at mandate_recipient, before any quote.",
     "source": DENIED, "edits": []},
    {"id": "prompt-injection-recipient", "group": "B",
     "title": "Prompt-injected payment",
     "story": "A prompt-injected malicious instruction convinces the AI agent to pay attacker.near, but the owner never authorized that recipient.",
     "takeaway": "The model can generate any request; the compiled mandate stops recipient substitution.",
     "source": DENIED, "edits": []},
    {"id": "refund-redirect", "group": "B", "title": "Redirected refund",
     "story": "The agent points a failed swap's refund at an address the owner did not pin.",
     "takeaway": "The owner's refund address cannot be redirected by the agent.",
     "source": PAYMENT, "edits": [("timeout 10m refund Solana.USDC",
                                   "timeout 10m refund Solana.USDC to So11111111111111111111111111111111111111112")]},
    {"id": "fee-too-high", "group": "B", "title": "Fee cap above the owner's limit",
     "story": "The agent accepts up to 150 bps in fees; the owner allows 100.",
     "takeaway": "A higher fee cap is refused even if the model presents it as normal.",
     "source": PAYMENT, "edits": [("require fees <= 100 bps", "require fees <= 150 bps")]},
    {"id": "daily-cap", "group": "B", "title": "Over the rolling 24-hour cap",
     "story": "Assuming the agent already spent 180 USDC an hour ago, 25 more breaks the 200 USDC day.",
     "takeaway": "Rolling spend is stateful; the console labels its assumed state explicitly.",
     "source": PAYMENT, "edits": [],
     "assumed_state": {"spends": [{"amount": "180000000", "at_ms": NOW_MS - HOUR_MS}]},
     "contract_tests": [f"{GUARD_TESTS}/mandate_tests.rs::the_daily_cap_is_a_rolling_window_checked_when_a_quote_is_accepted"]},
    {"id": "revoked", "group": "B", "title": "After the owner revoked the mandate",
     "story": "Assuming the owner revoked the mandate, the same authorized payment is refused.",
     "takeaway": "Revocation invalidates the same request without changing its contents.",
     "source": PAYMENT, "edits": [], "assumed_state": {"revoked": True},
     "contract_tests": [f"{GUARD_TESTS}/mandate_tests.rs::revoking_stops_the_agent_at_once_and_replacing_voids_old_intents"]},
    {"id": "nonce-reuse", "group": "B", "title": "Reused nonce",
     "story": "The agent registers agent_payment_001 a second time to open another deposit address.",
     "takeaway": "The local checker cannot see used nonces; the contract test is the evidence.",
     "source": PAYMENT, "edits": [], "contract_only": True,
     "contract_note": "A nonce is state on NEAR. The local checker cannot see which nonces were used, "
                      "so it passes this payment; x3-guard panics with \"already registered\" on the "
                      "second registration, and a policy accepts one quote.",
     "contract_tests": [f"{GUARD_TESTS}/tests.rs::a_nonce_registers_once",
                        f"{GUARD_TESTS}/tests.rs::a_policy_accepts_one_quote"]},
    {"id": "wrong-owner", "group": "B", "title": "Wrong mandate owner",
     "story": "The agent claims to act for another owner with a different mandate.",
     "takeaway": "Owner mismatch is refused before any payment term is evaluated.",
     "source": PAYMENT,
     "edits": [("    under mandate x3-owner.near", "    under mandate other-owner.near")]},
    {"id": "slippage-too-high", "group": "B", "title": "Slippage above the owner's limit",
     "story": "The agent asks for 1.5% slippage while the owner permits at most 1%.",
     "takeaway": "Execution freedom is capped independently of the payment amount.",
     "source": PAYMENT,
     "edits": [("require slippage <= 0.5%", "require slippage <= 1.5%")]},
]


def sha256(text: str) -> str:
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


def apply_edits(source: str, edits: list[tuple[str, str]], name: str) -> str:
    for old, new in edits:
        if source.count(old) != 1:
            raise SystemExit(f"{name}: expected exactly one {old!r} in the source")
        source = source.replace(old, new)
    return source


def build() -> dict[str, Any]:
    mandate_source = (X3 / MANDATE).read_text(encoding="utf-8")
    tokens = json.loads((X3 / TOKENS).read_text(encoding="utf-8"))
    out: dict[str, Any] = {
        "schema": "x3.judge-console.scenarios.v2",
        "generated_by": "x3-lang/near/demo/judge/build_scenarios.py",
        "evaluated_at": NOW.isoformat().replace("+00:00", "Z"),
        "evidence_level": "local_policy_preview_only",
        "tracks": list(tc.TRACKS),
        "track_guide": TRACK_GUIDE,
        "demo_steps": DEMO_STEPS,
        "groups": {
            "A": "Authorized agent payment",
            "B": "Forbidden actions",
        },
        "mandate": {"file": f"x3-lang/{MANDATE}", "source": mandate_source, "sha256": sha256(mandate_source)},
        "tokens": {"file": f"x3-lang/{TOKENS}", "records": tokens,
                   "note": "Archived 1Click token list. Not a live response."},
        "scenarios": [],
    }
    with tempfile.TemporaryDirectory() as tmp:
        for spec in SCENARIOS:
            base = (X3 / spec["source"]).read_text(encoding="utf-8")
            source = apply_edits(base, spec["edits"], spec["id"])
            path = Path(tmp) / f"{spec['id']}.x3"
            path.write_text(source, encoding="utf-8")
            report = tc.evaluate(str(X3 / MANDATE), str(path), tokens, now=NOW,
                                 assumed_state=spec.get("assumed_state"))
            out["scenarios"].append({
                "id": spec["id"], "group": spec["group"], "title": spec["title"], "story": spec["story"],
                "takeaway": spec["takeaway"],
                "evidence_state": "contract_test_required" if spec.get("contract_only") else "local_python_and_browser",
                "intent": {"file": f"x3-lang/{spec['source']}", "source": source, "sha256": sha256(source),
                           "edits": [{"from": old, "to": new} for old, new in spec["edits"]]},
                "assumed_guard_state": spec.get("assumed_state"),
                "contract_only": spec.get("contract_only", False),
                "contract_note": spec.get("contract_note"),
                "contract_tests": spec.get("contract_tests", []),
                "python": {key: report[key] for key in ("decision", "denied_guards", "approvals_needed")}
                | {"checks": [{"guard": c["guard"], "ok": c["ok"]} for c in report["checks"]]},
            })
    out["summary"] = {
        "total": len(out["scenarios"]),
        "allowed": sum(s["python"]["decision"] == "preflight_passed" and not s["contract_only"]
                       for s in out["scenarios"]),
        "approval_required": sum(s["python"]["decision"] == "approval_required" for s in out["scenarios"]),
        "denied": sum(s["python"]["decision"] == "denied_by_policy" for s in out["scenarios"]),
        "contract_only": sum(s["contract_only"] for s in out["scenarios"]),
    }
    return out


def render(data: dict[str, Any]) -> str:
    return json.dumps(data, indent=2, sort_keys=False) + "\n"


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--check", action="store_true", help="exit 1 if scenarios.json is out of date")
    args = parser.parse_args(argv)
    text = render(build())
    if args.check:
        if not OUT.exists() or OUT.read_text(encoding="utf-8") != text:
            print(f"{OUT} is stale: run {Path(__file__).name}", file=sys.stderr)
            return 1
        return 0
    OUT.write_text(text, encoding="utf-8")
    print(f"wrote {OUT}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
