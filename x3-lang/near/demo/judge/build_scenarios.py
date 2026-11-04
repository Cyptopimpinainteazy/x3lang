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

# (id, group, title, story, source file, [(old, new)], assumed Guard state, contract tests)
SCENARIOS: list[dict[str, Any]] = [
    {"id": "authorized", "group": "A", "title": "Authorized payment",
     "story": "The agent pays 25 USDC to a listed recipient, inside every limit.",
     "source": PAYMENT, "edits": []},
    {"id": "needs-approval", "group": "A", "title": "Payment that needs approval",
     "story": "40 USDC is under the 50 USDC cap but over the 30 USDC approval threshold.",
     "source": PAYMENT, "edits": [("amount 25000000", "amount 40000000"),
                                  ("min_receive 24800000", "min_receive 39680000")],
     "contract_tests": [f"{GUARD_TESTS}/mandate_tests.rs::payments_above_the_threshold_need_n_of_m_approvals"]},
    {"id": "over-limit", "group": "B", "title": "Exceeds the spending limit",
     "story": "60 USDC in one payment, over the owner's 50 USDC cap.",
     "source": PAYMENT, "edits": [("amount 25000000", "amount 60000000"),
                                  ("min_receive 24800000", "min_receive 59520000")]},
    {"id": "wrong-recipient", "group": "B", "title": "Unauthorized recipient",
     "story": "The agent tries to pay attacker.near, who is not on the owner's list.",
     "source": DENIED, "edits": []},
    {"id": "refund-redirect", "group": "B", "title": "Redirected refund",
     "story": "The agent points a failed swap's refund at an address the owner did not pin.",
     "source": PAYMENT, "edits": [("timeout 10m refund Solana.USDC",
                                   "timeout 10m refund Solana.USDC to So11111111111111111111111111111111111111112")]},
    {"id": "fee-too-high", "group": "B", "title": "Fee cap above the owner's limit",
     "story": "The agent accepts up to 150 bps in fees; the owner allows 100.",
     "source": PAYMENT, "edits": [("require fees <= 100 bps", "require fees <= 150 bps")]},
    {"id": "daily-cap", "group": "B", "title": "Over the rolling 24-hour cap",
     "story": "Assuming the agent already spent 180 USDC an hour ago, 25 more breaks the 200 USDC day.",
     "source": PAYMENT, "edits": [],
     "assumed_state": {"spends": [{"amount": "180000000", "at_ms": NOW_MS - HOUR_MS}]},
     "contract_tests": [f"{GUARD_TESTS}/mandate_tests.rs::the_daily_cap_is_a_rolling_window_checked_when_a_quote_is_accepted"]},
    {"id": "revoked", "group": "B", "title": "After the owner revoked the mandate",
     "story": "Assuming the owner revoked the mandate, the same authorized payment is refused.",
     "source": PAYMENT, "edits": [], "assumed_state": {"revoked": True},
     "contract_tests": [f"{GUARD_TESTS}/mandate_tests.rs::revoking_stops_the_agent_at_once_and_replacing_voids_old_intents"]},
    {"id": "nonce-reuse", "group": "B", "title": "Reused nonce",
     "story": "The agent registers agent_payment_001 a second time to open another deposit address.",
     "source": PAYMENT, "edits": [], "contract_only": True,
     "contract_note": "A nonce is state on NEAR. The local checker cannot see which nonces were used, "
                      "so it passes this payment; x3-guard panics with \"already registered\" on the "
                      "second registration, and a policy accepts one quote.",
     "contract_tests": [f"{GUARD_TESTS}/tests.rs::a_nonce_registers_once",
                        f"{GUARD_TESTS}/tests.rs::a_policy_accepts_one_quote"]},
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
        "schema": "x3.judge-console.scenarios.v1",
        "generated_by": "x3-lang/near/demo/judge/build_scenarios.py",
        "evaluated_at": NOW.isoformat().replace("+00:00", "Z"),
        "evidence_level": "local_policy_preview_only",
        "tracks": list(tc.TRACKS),
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
                "intent": {"file": f"x3-lang/{spec['source']}", "source": source, "sha256": sha256(source),
                           "edits": [{"from": old, "to": new} for old, new in spec["edits"]]},
                "assumed_guard_state": spec.get("assumed_state"),
                "contract_only": spec.get("contract_only", False),
                "contract_note": spec.get("contract_note"),
                "contract_tests": spec.get("contract_tests", []),
                "python": {key: report[key] for key in ("decision", "denied_guards", "approvals_needed")}
                | {"checks": [{"guard": c["guard"], "ok": c["ok"]} for c in report["checks"]]},
            })
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
