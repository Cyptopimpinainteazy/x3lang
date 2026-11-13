#!/usr/bin/env python3
"""Walk mainnet invoice_06 link by link: written .x3, compiler, Guard, payment, recorded outcome.

    python3 scripts/hackathon/invoice06_chain.py
    python3 scripts/hackathon/invoice06_chain.py --json

The other offline demos (`x3_wont_compile.py`, `compromised_agent_redteam.py`) use example
programs. This one uses the two `.x3` files from the mainnet run itself and shows that each link
hands the next one exactly what it received:

1. Written policy: the committed `payment-proposed.x3` and `mandate-proposed.x3` hash to the
   program_sha256 values Guard stores on the intent and on the mandate.
2. Compiler: the shipped compiler, run on those files, produces the same 10 policy fields and the
   same 13 mandate terms Guard stores on mainnet. Edited copies of the same payment are refused,
   either at compile time or by the mandate check, each with its own code.
3. Guard: the finalized `agent_accept_quote` receipt returns its checks; all of them passed.
4. Payment: the Solana USDC deposit to the quoted deposit address, and the `intents.near`
   `ft_withdraw` of the quoted amount to the stated recipient.
5. Outcome: the finalized `agent_record_outcome` result as Guard stored it.

It then states where the chain is not cryptographic. Offline: only committed files are read
(the public-RPC observation and outcome verification captured on 2026-11-12/13, and the
offline token fixture). No RPC, 1Click call, key or wallet is used; nothing is signed or sent.
"""
from __future__ import annotations

import argparse
import base64
import hashlib
import importlib.util
import json
import sys
import tempfile
from datetime import datetime, timedelta, timezone
from pathlib import Path
from typing import Any, Dict, List, Optional, Tuple

ROOT = Path(__file__).resolve().parents[2]
X3 = ROOT / "x3-lang"
BUNDLE = ROOT / "evidence" / "near-guard-mainnet-invoice06-2026-11-12"
PAYMENT = BUNDLE / "source-recovery" / "payment-proposed.x3"
MANDATE = BUNDLE / "source-recovery" / "mandate-proposed.x3"
OBSERVATION = BUNDLE / "public-rpc-observation.json"
OUTCOME = BUNDLE / "outcome-recording" / "finalized-outcome-verification.json"
TOKENS = X3 / "tests" / "fixtures" / "near_intents_tokens.json"
TIMEOUT = timedelta(hours=74)  # `timeout 74h` in payment-proposed.x3
SOLANA_USDC_MINT = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v"

#: Edits an agent (or an attacker steering it) might make to the mainnet payment, and where the
#: chain must stop each one: "compile" (compiler code) or "mandate" (failed mandate checks).
EDITS: List[Dict[str, Any]] = [
    {"case": "recipient_attacker", "tried": "pay attacker.near instead of x3lang.near",
     "old": "receiver x3lang.near", "new": "receiver attacker.near",
     "stop": "mandate", "expect": ["mandate_recipient"]},
    {"case": "ten_times_amount", "tried": "send 250 USDC instead of 25",
     "old": "amount 25000000", "new": "amount 250000000",
     "stop": "mandate", "expect": ["mandate_daily", "mandate_per_payment"]},
    {"case": "refund_redirect", "tried": "send refunds to another Solana wallet",
     "old": "receiver 58tbMfadmi8HWzUndLqkX3prBbkiZMEaYMTDYSzhB3cQ",
     "new": "receiver 9xQeWvG816bUx9EPjHmaT23yvVM2ZWbrrpZb9PusVFin",
     "stop": "mandate", "expect": ["mandate_refund_to"]},
    {"case": "loose_slippage", "tried": "allow 5% slippage instead of 0.9%",
     "old": "require slippage <= 0.9%", "new": "require slippage <= 5%",
     "stop": "mandate", "expect": ["mandate_slippage"]},
    {"case": "no_slippage_bound", "tried": "drop the slippage bound",
     "old": "    require slippage <= 0.9%\n", "new": "",
     "stop": "compile", "expect": "X3_NEAR_SLIPPAGE_REQUIRED"},
    {"case": "no_timeout", "tried": "drop the timeout and refund",
     "old": "    timeout 74h refund Solana.USDC\n", "new": "",
     "stop": "compile", "expect": "X3_NEAR_TIMEOUT_REQUIRED"},
    {"case": "no_nonce", "tried": "drop the one-use nonce",
     "old": "    require nonce mainnet_demo_nov11_invoice_06\n", "new": "",
     "stop": "compile", "expect": "X3_NEAR_NONCE"},
    {"case": "second_leg", "tried": "add a second swap leg",
     "old": "min_receive 24750000;\n",
     "new": "min_receive 24750000;\n        bridge NearIntents Near.USDC -> Near.wNEAR;\n",
     "stop": "compile", "expect": "X3_NEAR_ROUTE_SHAPE"},
]

LIMITS = [
    "Guard -> payment: Guard accepted the quote; the owner signed the Solana deposit by hand. "
    "Guard does not hold or move the USDC.",
    "Deposit <-> payout: linked by 1Click status, timing and amount, not by a cryptographic "
    "cross-chain proof (the outcome verification records cross_chain_causality_proven: false).",
    "Payout -> outcome: the outcome is the agent's report. Guard stores the transaction hashes it "
    "is given and checks the amount against the floor; it does not authenticate the hashes.",
    "Compiler -> Guard: Guard checks the stored .x3 hashes and the policy fields; it does not "
    "compile or run .x3. This script shows the compiler's output equals what Guard stores.",
]


def _load_compiler():
    sys.path.insert(0, str(X3))
    spec = importlib.util.spec_from_file_location("x3_near_intents", X3 / "near_intents.py")
    module = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    spec.loader.exec_module(module)
    return module


ni = _load_compiler()


def _json(path: Path) -> Any:
    return json.loads(path.read_text(encoding="utf-8"))


def _tokens() -> List[Dict[str, Any]]:
    data = _json(TOKENS)
    return data.get("tokens", data) if isinstance(data, dict) else data


def _sha256(text: str) -> str:
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


def _check_value(observation: Dict[str, Any], name: str) -> Any:
    """Decode a finalized transaction's SuccessValue recorded in the observation's checks."""
    for check in observation["checks"]:
        if check.get("check") == name:
            if check.get("result") != "pass":
                raise SystemExit(f"{OBSERVATION.name}: {name} is not a passing check")
            encoded = check["detail"]["status"]["SuccessValue"]
            return json.loads(base64.b64decode(encoded))
    raise SystemExit(f"{OBSERVATION.name}: no {name} check")


def _diff(ours: Dict[str, Any], chain: Dict[str, Any]) -> Dict[str, Tuple[Any, Any]]:
    return {k: (ours.get(k), chain[k]) for k in sorted(chain) if ours.get(k) != chain[k]}


def compile_time(chain_policy: Dict[str, Any]) -> datetime:
    """The compile moment implied by the stored deadline (`timeout 74h`)."""
    return datetime.fromtimestamp(chain_policy["deadline_ms"] / 1000, timezone.utc) - TIMEOUT


def compile_payment(text: str, now: datetime, tokens: List[Dict[str, Any]]):
    with tempfile.TemporaryDirectory() as tmp:
        path = Path(tmp) / "payment.x3"
        path.write_text(text, encoding="utf-8")
        _, leg = ni.compile_program(str(path), now=now)
    request = ni.build_quote_request(leg, tokens, dry=True)
    return leg, request, ni.policy_for(leg, request, _sha256(text))


def try_edit(edit: Dict[str, Any], base: str, now: datetime, tokens: List[Dict[str, Any]]) -> Dict[str, Any]:
    if base.count(edit["old"]) != 1:
        raise SystemExit(f"{PAYMENT.name} drifted: edit {edit['case']} no longer applies exactly once")
    text = base.replace(edit["old"], edit["new"])
    row = {"case": edit["case"], "tried": edit["tried"], "expected_stop": edit["stop"]}
    try:
        leg, request, policy = compile_payment(text, now, tokens)
    except ni.NearIntentsError as exc:
        row.update(stopped_at="compile", code=exc.code)
        row["pass"] = edit["stop"] == "compile" and exc.code == edit["expect"]
        return row
    try:
        ni.check_against_mandate(leg, request, policy["program_sha256"], str(MANDATE), tokens=tokens, now=now)
    except ni.NearIntentsError as exc:
        detail = exc.detail if isinstance(exc.detail, dict) else {}
        failed = sorted(c["guard"] for c in detail.get("checks", []) if not c["ok"])
        row.update(stopped_at="mandate", code=exc.code, failed=failed)
        row["pass"] = edit["stop"] == "mandate" and failed == sorted(edit["expect"])
        return row
    row.update(stopped_at=None)
    row["pass"] = False
    return row


def evaluate() -> Dict[str, Any]:
    observation = _json(OBSERVATION)
    outcome_file = _json(OUTCOME)
    tokens = _tokens()
    intent = observation["intent"]["value"]
    mandate = observation["mandate"]["value"]
    payment_text = PAYMENT.read_text(encoding="utf-8")
    mandate_text = MANDATE.read_text(encoding="utf-8")
    now = compile_time(intent["policy"])

    written = {
        "payment_sha256": _sha256(payment_text),
        "mandate_sha256": _sha256(mandate_text),
        "payment_matches_intent": _sha256(payment_text) == intent["policy"]["program_sha256"],
        "mandate_matches_mandate": _sha256(mandate_text) == mandate["terms"]["program_sha256"],
    }

    leg, request, policy = compile_payment(payment_text, now, tokens)
    compiled_mandate = ni.mandates().compile_mandate(str(MANDATE), tokens, now=now)
    policy_diff = _diff(policy, intent["policy"])
    terms_diff = _diff(compiled_mandate["terms"], mandate["terms"])
    file_check = ni.check_against_mandate(leg, request, policy["program_sha256"], str(MANDATE), tokens=tokens, now=now)
    edits = [try_edit(edit, payment_text, now, tokens) for edit in EDITS]
    compiler = {
        "compiled_at_utc": now.isoformat().replace("+00:00", "Z"),
        "policy_fields": len(intent["policy"]),
        "policy_fields_equal": len(intent["policy"]) - len(policy_diff),
        "policy_diff": policy_diff,
        "nonce_equal": leg.get("nonce") == intent["nonce"],
        "owner_agent_equal": (compiled_mandate["owner"], compiled_mandate["agent"]) == (mandate["owner"], mandate["agent"]),
        "mandate_terms": len(mandate["terms"]),
        "mandate_terms_equal": len(mandate["terms"]) - len(terms_diff),
        "mandate_terms_diff": terms_diff,
        "file_mandate_checks_ok": all(c["ok"] for c in file_check["checks"]),
        "file_mandate_checks": len(file_check["checks"]),
        "edits": edits,
        "edits_refused": sum(1 for e in edits if e["pass"]),
    }

    accept_tx = observation["near_transactions"]["agent_accept_quote"]
    guards = _check_value(observation, "agent_accept_quote_final_success")
    agent_keys = {k["public_key"]: k["access_key"]["permission"] for k in observation["agent_access_keys"]["result"]["keys"]}
    signing_permission = agent_keys.get(accept_tx["public_key"])
    guard = {
        "tx": accept_tx["hash"],
        "block": accept_tx["included_block_height"],
        "signer": accept_tx["signer_id"],
        "public_key": accept_tx["public_key"],
        "signed_by_function_call_key": isinstance(signing_permission, dict) and "FunctionCall" in signing_permission
        and signing_permission["FunctionCall"]["receiver_id"] == accept_tx["receiver_id"],
        "checks": len(guards),
        "ok": sum(1 for g in guards if g.get("ok") is True),
        "failed": [g.get("guard") for g in guards if g.get("ok") is not True],
    }

    deposit = observation["solana_deposit"]
    quote = intent["quote"]["facts"]

    def usdc_held(balances: List[Dict[str, Any]], owner: str) -> int:
        return sum(int(b["amount"]) for b in balances if b.get("owner") == owner and b.get("mint") == SOLANA_USDC_MINT)

    payer = quote["refund_to"]
    deposit_address = quote["deposit_address"]
    payout_tx = observation["near_transactions"]["payout"]
    withdrawals = []
    for action in payout_tx["actions"]:
        for signed in action["FunctionCall"]["args"].get("signed", []):
            for item in json.loads(signed["payload"]["message"]).get("intents", []):
                if item.get("intent") == "ft_withdraw":
                    withdrawals.append(item)
    payment = {
        "solana_tx": deposit["signature"],
        "solana_slot": deposit["slot"],
        "solana_err": deposit["err"],
        "solana_signers": deposit["signers"],
        "payer": payer,
        "payer_before": usdc_held(deposit["pre_token_balances"], payer),
        "payer_after": usdc_held(deposit["post_token_balances"], payer),
        "deposit_address": deposit_address,
        "deposit_address_received": usdc_held(deposit["post_token_balances"], deposit_address)
        - usdc_held(deposit["pre_token_balances"], deposit_address),
        "payout_tx": payout_tx["hash"],
        "payout_signer": payout_tx["signer_id"],
        "payout_withdrawals": withdrawals,
        "payout_matches_quote": len(withdrawals) == 1
        and withdrawals[0]["receiver_id"] == quote["recipient"]
        and withdrawals[0]["amount"] == quote["amount_out"],
        "seconds_deposit_to_payout": round(payout_tx["included_block_timestamp_ns"] / 1e9 - deposit["block_time"]),
    }

    recorded = outcome_file["intent_after"]["value"]["outcome"]
    outcome = {
        "tx": outcome_file["transaction_hash"],
        "block": outcome_file["execution_block_height"],
        "status": recorded["status"],
        "amount_out": recorded["amount_out"],
        "floor_met": recorded["floor_met"],
        "tx_hashes": recorded["tx_hashes"],
        "cites_payout": payout_tx["hash"] in recorded["tx_hashes"],
        "cross_chain_causality_proven": outcome_file["cross_chain_causality_proven"],
    }

    links = {
        "written": written["payment_matches_intent"] and written["mandate_matches_mandate"],
        "compiler": not policy_diff and not terms_diff and compiler["nonce_equal"] and compiler["owner_agent_equal"]
        and compiler["file_mandate_checks_ok"] and compiler["edits_refused"] == len(EDITS),
        "guard": guard["checks"] > 0 and not guard["failed"] and guard["signed_by_function_call_key"],
        "payment": deposit["err"] is None and payment["deposit_address_received"] == int(quote["amount_in"])
        and payment["payer_before"] - payment["payer_after"] == int(quote["amount_in"])
        and payment["solana_signers"] == [payer]
        and payment["payout_matches_quote"] and payment["payout_signer"] == "intents.near",
        "outcome": outcome["status"] == "Settled" and outcome["floor_met"] is True
        and outcome["amount_out"] == quote["amount_out"] and outcome["cites_payout"],
    }
    return {"schema": "x3.invoice06_chain.v1", "links": links, "ok": all(links.values()),
            "written": written, "compiler": compiler, "guard": guard, "payment": payment,
            "outcome": outcome, "not_cryptographic": LIMITS}


def _mark(ok: bool) -> str:
    return "PASS" if ok else "FAIL"


def render(r: Dict[str, Any]) -> str:
    w, c, g, p, o = r["written"], r["compiler"], r["guard"], r["payment"], r["outcome"]
    usdc = lambda units: f"{int(units) / 1_000_000:.6f}"  # noqa: E731
    lines = [
        "invoice_06, link by link (offline: committed files only, nothing signed or sent)",
        "",
        f"1  Written policy {'':52}{_mark(r['links']['written'])}",
        f"   payment-proposed.x3  sha256 {w['payment_sha256'][:12]}…  = program hash Guard stores on the intent",
        f"   mandate-proposed.x3  sha256 {w['mandate_sha256'][:12]}…  = program hash Guard stores on the mandate",
        "",
        f"2  Compiler (same compiler the agent path uses, run at {c['compiled_at_utc'][:19]}Z) {_mark(r['links']['compiler'])}",
        f"   payment compiles; its policy equals Guard's stored policy     {c['policy_fields_equal']}/{c['policy_fields']} fields, nonce {'equal' if c['nonce_equal'] else 'DIFFERENT'}",
        f"   mandate compiles; its terms equal Guard's stored mandate      {c['mandate_terms_equal']}/{c['mandate_terms']} terms",
        f"   payment passes its own mandate offline                        {c['file_mandate_checks']}/{c['file_mandate_checks']} checks",
        f"   the same payment, edited: {c['edits_refused']}/{len(c['edits'])} refused where expected",
    ]
    for e in c["edits"]:
        where = (f"compiler refuses: {e['code']}" if e.get("stopped_at") == "compile"
                 else f"mandate refuses: {', '.join(e.get('failed', []))}" if e.get("stopped_at") == "mandate"
                 else "NOT REFUSED")
        lines.append(f"     {'ok ' if e['pass'] else 'BAD'} {e['tried']:42} {where}")
    lines += [
        "",
        f"3  Guard on mainnet {'':50}{_mark(r['links']['guard'])}",
        f"   agent_accept_quote {g['tx'][:8]}… block {g['block']}, signed by {g['signer']}'s "
        + ("FunctionCall key (receiver x3lang.near)" if g["signed_by_function_call_key"] else "NON-FunctionCall key"),
        f"   returned {g['checks']} checks: {g['ok']} ok, {len(g['failed'])} failed",
        "",
        f"4  Payment {'':59}{_mark(r['links']['payment'])}",
        f"   Solana: {p['solana_tx'][:8]}… slot {p['solana_slot']}, signed by the owner wallet {p['payer'][:6]}…: "
        f"{usdc(p['payer_before'])} -> {usdc(p['payer_after'])} USDC",
        f"           quoted deposit address {p['deposit_address'][:6]}… received +{usdc(p['deposit_address_received'])} USDC",
        f"   NEAR:   {p['payout_tx'][:8]}… {p['payout_signer']} ft_withdraw "
        + ", ".join(f"{usdc(x['amount'])} USDC to {x['receiver_id']}" for x in p["payout_withdrawals"])
        + f", {p['seconds_deposit_to_payout']} s after the deposit",
        "",
        f"5  Recorded outcome {'':50}{_mark(r['links']['outcome'])}",
        f"   agent_record_outcome {o['tx'][:8]}… block {o['block']}: {o['status']}, amount_out {o['amount_out']}, "
        f"floor_met {str(o['floor_met']).lower()}, cites the payout: {'yes' if o['cites_payout'] else 'no'}",
        "",
        "Where the chain is not cryptographic:",
    ]
    lines += [f"   - {limit}" for limit in r["not_cryptographic"]]
    lines += ["", f"{sum(r['links'].values())}/5 links hold on the committed evidence."]
    return "\n".join(lines)


def main(argv: Optional[List[str]] = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n", 1)[0])
    parser.add_argument("--json", action="store_true", help="print the full report as JSON")
    args = parser.parse_args(argv)
    report = evaluate()
    print(json.dumps(report, indent=2, sort_keys=True) if args.json else render(report))
    return 0 if report["ok"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
