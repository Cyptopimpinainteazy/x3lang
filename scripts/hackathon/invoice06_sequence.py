#!/usr/bin/env python3
"""Walk one mainnet payment through the five stages judges need to see together.

    python3 scripts/hackathon/invoice06_sequence.py
    python3 scripts/hackathon/invoice06_sequence.py --json

The payment is invoice_06: the written files in
evidence/near-guard-mainnet-invoice06-2026-11-12/source-recovery/. This command
checks, offline:

1. Written .x3. Both files hash to the program_sha256 values in the archived
   finalized Guard reads.
2. Compiler rejection. The payment compiles. Six unsafe edits of that same
   file are refused by the NEAR Intents compiler, each with its own code.
3. Deterministic Guard enforcement. Compiling both files at the instant implied
   by their written 74h window lowers to the terms Guard stored. The contract's
   check_mandate allows that policy and refuses the same payment retargeted at
   attacker.near. The archived agent_accept_quote receipt then shows the 21
   checks Guard actually returned, all passing.
4. Real mainnet payment. The archived public-RPC observation records the
   Solana USDC deposit and the NEAR payout, in order, after acceptance.
5. Recorded outcome. The later finalized readback stores Settled, the payout
   amount, floor_met, and both settlement transaction hashes.

Nothing here signs, deploys, or sends. The archived receipt is not a fresh RPC
read; observe_mainnet_invoice06.py does that. Guard did not compile .x3, did
not hold the Solana USDC, and does not authenticate the outcome's transaction
hashes. floor_met compares the amount the agent supplied with the quote floor.
"""
from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import sys
import tempfile
from datetime import datetime, timedelta, timezone
from pathlib import Path
from typing import Any, Dict, List, Optional

ROOT = Path(__file__).resolve().parents[2]
X3 = ROOT / "x3-lang"
EVIDENCE = ROOT / "evidence" / "near-guard-mainnet-invoice06-2026-11-12"
PAYMENT = EVIDENCE / "source-recovery" / "payment-proposed.x3"
MANDATE = EVIDENCE / "source-recovery" / "mandate-proposed.x3"
OBSERVATION = EVIDENCE / "public-rpc-observation.json"
SOURCE_VERIFICATION = EVIDENCE / "source-recovery" / "guard-source-verification.json"
OUTCOME = EVIDENCE / "outcome-recording" / "finalized-outcome-verification.json"
TOKENS = X3 / "tests" / "fixtures" / "near_intents_tokens.json"

# check_mandate order, then the approval check agent_accept_quote adds, then
# the signed-quote checks. Twenty-one names, matching the invoice_06 receipt.
MANDATE_GUARDS = (
    "mandate_active",
    "mandate_asset",
    "mandate_destination_asset",
    "mandate_recipient",
    "mandate_refund_to",
    "mandate_per_payment",
    "mandate_daily",
    "mandate_slippage",
    "mandate_fees",
    "mandate_deadline",
)
ON_CHAIN_GUARDS = MANDATE_GUARDS + (
    "mandate_approvals",
    "quote_kind",
    "quote_freshness",
    "quote_request_echo",
    "amount_in",
    "min_receive",
    "slippage",
    "fees",
    "refund_fee",
    "deposit_address",
    "deadline",
)

EDITS: List[Dict[str, Any]] = [
    {"case": "no_slippage_bound", "tried": "drop `require slippage`",
     "old": "    require slippage <= 0.9%\n", "new": "",
     "code": "X3_NEAR_SLIPPAGE_REQUIRED"},
    {"case": "no_timeout", "tried": "drop the timeout and refund",
     "old": "    timeout 74h refund Solana.USDC\n", "new": "",
     "code": "X3_NEAR_TIMEOUT_REQUIRED"},
    {"case": "no_nonce", "tried": "drop the nonce",
     "old": "    require nonce mainnet_demo_nov11_invoice_06\n", "new": "",
     "code": "X3_NEAR_NONCE"},
    {"case": "second_leg", "tried": "add a second swap leg",
     "old": "        bridge NearIntents Solana.USDC -> Near.USDC min_receive 24750000;\n",
     "new": ("        bridge NearIntents Solana.USDC -> Near.USDC min_receive 24750000;\n"
             "        bridge NearIntents Near.USDC -> Near.wNEAR;\n"),
     "code": "X3_NEAR_ROUTE_SHAPE"},
    {"case": "lookalike_recipient", "tried": "pay a Cyrillic look-alike of x3lang.near",
     "old": "receiver x3lang.near", "new": "receiver x3lang.ne\u0430r",
     "code": "X3_NEAR_TYPECHECK", "inner": "X3_INVALID_RECEIVER"},
    {"case": "zero_amount", "tried": "pay an amount of 0",
     "old": "amount 25000000", "new": "amount 0",
     "code": "X3_NEAR_TYPECHECK", "inner": "X3_INVALID_AMOUNT"},
]


def _load(name: str, path: Path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


def _compiler():
    sys.path.insert(0, str(X3))
    return _load("x3_near_intents_invoice06", X3 / "near_intents.py")


def _observer():
    return _load(
        "observe_mainnet_invoice06_seq",
        ROOT / "scripts" / "hackathon" / "observe_mainnet_invoice06.py",
    )


def _sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def _from_ms(ms: int) -> datetime:
    return datetime(1970, 1, 1, tzinfo=timezone.utc) + timedelta(milliseconds=ms)


def _read_json(path: Path) -> Dict[str, Any]:
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise SystemExit(f"{path} is not a JSON object")
    return value


def _one_check(observation: Dict[str, Any], name: str) -> Dict[str, Any]:
    rows = [row for row in observation["checks"] if row.get("check") == name]
    if len(rows) != 1:
        raise SystemExit(f"observation has {len(rows)} {name!r} checks")
    return rows[0]


def _compile(ni: Any, path: Path, *, now: Optional[datetime] = None) -> Dict[str, Any]:
    try:
        ni.compile_program(str(path), now=now)
    except ni.NearIntentsError as exc:
        detail = exc.detail if isinstance(exc.detail, list) else []
        inner = sorted({d.get("code") for d in detail if isinstance(d, dict) and d.get("code")})
        return {"compiled": False, "code": exc.code, "inner": inner, "message": str(exc)}
    return {"compiled": True, "code": None, "inner": [], "message": None}


def _window(source: str, clause: str) -> str:
    needle = f"{clause} 74h"
    if source.count(needle) != 1:
        raise SystemExit(f"{clause} 74h does not appear exactly once")
    return "74h"


def _accept_guards(observation: Dict[str, Any]) -> List[Dict[str, Any]]:
    receipts = observation["near_transactions"]["agent_accept_quote"]["receipts"]
    for receipt in receipts:
        value = (receipt.get("status") or {}).get("SuccessValue")
        if isinstance(value, list) and value and isinstance(value[0], dict) and "guard" in value[0]:
            return value
    raise SystemExit("agent_accept_quote receipt has no guard list")


def _policy(ni: Any, leg: Dict[str, Any], tokens: List[Dict[str, Any]], program_sha256: str) -> Dict[str, Any]:
    origin = ni.resolve_asset(tokens, leg["origin"]["chain"], leg["origin"]["asset"])["assetId"]
    destination = ni.resolve_asset(tokens, leg["destination"]["chain"], leg["destination"]["asset"])["assetId"]
    return ni.policy_for(leg, {"originAsset": origin, "destinationAsset": destination}, program_sha256)


def _terms_match(compiled: Dict[str, Any], stored: Dict[str, Any]) -> List[str]:
    diffs = []
    if set(compiled) != set(stored):
        diffs.append(f"keys {sorted(set(compiled) ^ set(stored))}")
    for key in sorted(set(compiled) & set(stored)):
        if str(compiled[key]) != str(stored[key]):
            diffs.append(key)
    return diffs


def written_stage(observation: Dict[str, Any], source_verification: Dict[str, Any]) -> Dict[str, Any]:
    payment_sha = _sha256(PAYMENT)
    mandate_sha = _sha256(MANDATE)
    intent_sha = observation["intent"]["value"]["policy"]["program_sha256"]
    mandate_chain_sha = observation["mandate"]["value"]["terms"]["program_sha256"]
    verified = {row["file"]: row for row in source_verification["files"]}
    ok = (
        payment_sha == intent_sha == verified["payment-proposed.x3"]["sha256"]
        and mandate_sha == mandate_chain_sha == verified["mandate-proposed.x3"]["sha256"]
        and verified["payment-proposed.x3"]["matches_guard"] is True
        and verified["mandate-proposed.x3"]["matches_guard"] is True
        and observation["intent"]["value"]["nonce"] == "mainnet_demo_nov11_invoice_06"
        and observation["mandate"]["value"]["version"] == 4
    )
    return {
        "ok": ok,
        "payment": str(PAYMENT.relative_to(ROOT)),
        "mandate": str(MANDATE.relative_to(ROOT)),
        "payment_sha256": payment_sha,
        "mandate_sha256": mandate_sha,
    }


def compiler_stage(ni: Any, work: Path) -> Dict[str, Any]:
    source = PAYMENT.read_text(encoding="utf-8")
    baseline = _compile(ni, PAYMENT)
    rows = []
    for edit in EDITS:
        if source.count(edit["old"]) != 1:
            raise SystemExit(f"{edit['case']}: anchor text is not unique in the invoice_06 payment")
        edited = work / f"{edit['case']}.x3"
        edited.write_text(source.replace(edit["old"], edit["new"], 1), encoding="utf-8")
        got = _compile(ni, edited)
        passed = (
            not got["compiled"]
            and got["code"] == edit["code"]
            and (edit.get("inner") is None or edit["inner"] in got["inner"])
        )
        rows.append({
            "case": edit["case"],
            "tried": edit["tried"],
            "expected": edit["code"],
            "expected_inner": edit.get("inner"),
            **got,
            "pass": passed,
        })
    return {
        "ok": baseline["compiled"] and all(row["pass"] for row in rows),
        "baseline_compiles": baseline["compiled"],
        "refused": sum(not row["compiled"] for row in rows),
        "total": len(rows),
        "cases": rows,
    }


def guard_stage(ni: Any, observation: Dict[str, Any], work: Path) -> Dict[str, Any]:
    payment = PAYMENT.read_text(encoding="utf-8")
    mandate_source = MANDATE.read_text(encoding="utf-8")
    window = _window(payment, "timeout")
    if _window(mandate_source, "expires") != window:
        raise SystemExit("the payment timeout and the mandate expiry are different windows")
    stored_policy = observation["intent"]["value"]["policy"]
    stored_terms = observation["mandate"]["value"]["terms"]
    deadline_ms = int(stored_policy["deadline_ms"])
    now = _from_ms(deadline_ms - int(ni.parse_duration(window).total_seconds() * 1000))
    tokens = json.loads(TOKENS.read_text(encoding="utf-8"))
    mandates = ni.mandates()
    _, leg = ni.compile_program(str(PAYMENT), now=now)
    compiled = mandates.compile_mandate(str(MANDATE), tokens, now=now)
    policy = _policy(ni, leg, tokens, _sha256(PAYMENT))
    term_diffs = _terms_match(compiled["terms"], stored_terms)
    policy_diffs = _terms_match(policy, stored_policy)
    accept_ms = int(observation["intent"]["value"]["quote"]["accepted_at_ms"])
    mandate = {
        "owner": compiled["owner"],
        "agent": compiled["agent"],
        "terms": compiled["terms"],
        "version": observation["mandate"]["value"]["version"],
        "revoked": False,
        "spends": [],
    }
    allowed = mandates.check_mandate(mandate, policy, accept_ms)
    attacker = work / "attacker-recipient.x3"
    attacker.write_text(
        payment.replace("to Near.USDC receiver x3lang.near", "to Near.USDC receiver attacker.near", 1),
        encoding="utf-8",
    )
    _, bad_leg = ni.compile_program(str(attacker), now=now)
    refused = mandates.check_mandate(mandate, _policy(ni, bad_leg, tokens, _sha256(PAYMENT)), accept_ms)
    refused_names = [row["guard"] for row in refused if not row["ok"]]
    on_chain = _accept_guards(observation)
    on_chain_names = tuple(row["guard"] for row in on_chain)
    on_chain_ok = all(row.get("ok") is True for row in on_chain) and on_chain_names == ON_CHAIN_GUARDS
    local_ok = (
        not term_diffs
        and not policy_diffs
        and all(row["ok"] for row in allowed)
        and [row["guard"] for row in allowed] == list(MANDATE_GUARDS)
        and refused_names == ["mandate_recipient"]
        and bad_leg["recipient"] == "attacker.near"
    )
    return {
        "ok": local_ok and on_chain_ok,
        "lowered_at": now.isoformat(),
        "term_diffs": term_diffs,
        "policy_diffs": policy_diffs,
        "local_allowed": [row["guard"] for row in allowed if row["ok"]],
        "local_refused": refused_names,
        "on_chain_guards": [row["guard"] for row in on_chain],
        "on_chain_all_ok": on_chain_ok,
        "on_chain_count": len(on_chain),
    }


def payment_stage(observation: Dict[str, Any], observer: Any) -> Dict[str, Any]:
    deposit = _one_check(observation, "sol_25_usdc_funder_to_deposit_address")
    payout = _one_check(observation, "payout_logs_name_recipient_and_amount")
    order = _one_check(observation, "approval_before_accept_before_payout")
    signer = _one_check(observation, "agent_call_used_restricted_function_call_key")
    near = observation["near_transactions"]
    solana_sig = observation["solana_deposit"]["signature"]
    payout_hash = near["payout"]["hash"]
    accept_hash = near["agent_accept_quote"]["hash"]
    logs = payout["detail"]
    amount_out_logged = any("24995200" in line and "x3lang.near" in line for line in logs)
    blocks = order["detail"]
    ok = (
        deposit["result"] == "pass"
        and deposit["detail"] == {"deposit_address_delta": 25000000, "funder_delta": -25000000}
        and payout["result"] == "pass"
        and amount_out_logged
        and order["result"] == "pass"
        and blocks == [224148959, 224149400, 224150052]
        and signer["result"] == "pass"
        and signer["detail"]["public_key"] == observer.DELEGATED_KEY
        and solana_sig == observer.SOL_DEPOSIT
        and payout_hash == observer.NEAR_TXS[3][1]
        and accept_hash == observer.NEAR_TXS[2][1]
        and near["agent_accept_quote"]["signer_id"] == observer.AGENT
        and near["payout"]["signer_id"] == "intents.near"
    )
    return {
        "ok": ok,
        "approval": near["owner_approval"]["hash"],
        "accept": accept_hash,
        "solana_deposit": solana_sig,
        "near_payout": payout_hash,
        "amount_in": "25000000",
        "amount_out": "24995200",
        "blocks": blocks,
    }


def outcome_stage(observation: Dict[str, Any], outcome: Dict[str, Any], observer: Any) -> Dict[str, Any]:
    readback = outcome["intent_after"]["value"]
    recorded = readback["outcome"]
    tx = outcome["rpc_transaction"]["transaction"]
    action = tx["actions"][0]["FunctionCall"]
    historical = observation["intent"]["value"]["outcome"]
    hashes = recorded["tx_hashes"]
    ok = (
        historical is None
        and all(value is True for value in outcome["checks"].values())
        and outcome["transaction_hash"] == tx["hash"] == "9sTn9pJ1ZbCVazBm9Wz9oEDh6aWw4FUVMoAJEKCCzuqE"
        and tx["signer_id"] == observer.AGENT
        and tx["public_key"] == observer.DELEGATED_KEY
        and tx["receiver_id"] == observer.GUARD
        and action["method_name"] == "agent_record_outcome"
        and action["deposit"] == "0"
        and len(tx["actions"]) == 1
        and recorded["status"] == "Settled"
        and recorded["amount_out"] == "24995200"
        and recorded["refunded_amount"] is None
        and recorded["floor_met"] is True
        and hashes == [observer.SOL_DEPOSIT, observer.NEAR_TXS[3][1]]
        and readback["policy"]["program_sha256"] == _sha256(PAYMENT)
        and readback["nonce"] == observer.NONCE
    )
    return {
        "ok": ok,
        "transaction": tx["hash"],
        "signer": tx["signer_id"],
        "public_key": tx["public_key"],
        "status": recorded["status"],
        "amount_out": recorded["amount_out"],
        "floor_met": recorded["floor_met"],
        "tx_hashes": hashes,
        "historical_observation_outcome": historical,
    }


def evaluate(work: Path) -> Dict[str, Any]:
    ni = _compiler()
    observer = _observer()
    observation = _read_json(OBSERVATION)
    source_verification = _read_json(SOURCE_VERIFICATION)
    outcome = _read_json(OUTCOME)
    stages = {
        "written": written_stage(observation, source_verification),
        "compiler_rejection": compiler_stage(ni, work),
        "guard": guard_stage(ni, observation, work),
        "payment": payment_stage(observation, observer),
        "outcome": outcome_stage(observation, outcome, observer),
    }
    return {
        "title": "invoice_06 sequence",
        "nonce": "mainnet_demo_nov11_invoice_06",
        "ok": all(stage["ok"] for stage in stages.values()),
        "stages": stages,
        "limits": [
            "Guard stores the .x3 SHA-256. It does not compile or run .x3.",
            "Guard authorized a deposit address. It did not hold or move the Solana USDC.",
            "The agent supplied the outcome transaction hashes. Guard sets floor_met from the supplied amount.",
            "The Solana deposit and the NEAR payout are associated by the 1Click record, not by a cross-chain proof.",
            "The agent account still has the owner's FullAccess recovery key. That key did not sign these calls.",
        ],
    }


def render(report: Dict[str, Any]) -> str:
    stages = report["stages"]
    written = stages["written"]
    compiler = stages["compiler_rejection"]
    guard = stages["guard"]
    payment = stages["payment"]
    outcome = stages["outcome"]
    lines = [
        "invoice_06: one written payment, five stages",
        "",
        "1. Written .x3",
        f"   payment {written['payment_sha256']}  {'matches Guard' if written['ok'] else 'MISMATCH'}",
        f"   mandate {written['mandate_sha256']}",
        "",
        "2. Compiler rejection",
        f"   the written payment {'compiles' if compiler['baseline_compiles'] else 'DOES NOT COMPILE'}",
    ]
    for row in compiler["cases"]:
        code = row["code"] or "compiled"
        if row["inner"]:
            code += f" ({', '.join(row['inner'])})"
        lines.append(f"   {row['tried']:<44} {code:<32} {'PASS' if row['pass'] else 'FAIL'}")
    lines += [
        f"   {compiler['refused']}/{compiler['total']} edits of this file refused at compile time",
        "",
        "3. Deterministic Guard enforcement",
        f"   lowered at {guard['lowered_at']}",
        f"   written files lower to the stored mandate terms: {'yes' if not guard['term_diffs'] else guard['term_diffs']}",
        f"   written payment lowers to the stored policy: {'yes' if not guard['policy_diffs'] else guard['policy_diffs']}",
        f"   local check_mandate allows that policy: {', '.join(guard['local_allowed'])}",
        f"   the same payment to attacker.near is refused by: {', '.join(guard['local_refused']) or 'nothing'}",
        f"   on-chain agent_accept_quote returned {guard['on_chain_count']} checks, all ok: {guard['on_chain_all_ok']}",
        "",
        "4. Real mainnet payment",
        f"   accept  {payment['accept']}",
        f"   Solana  {payment['solana_deposit']}  {payment['amount_in']} USDC base units",
        f"   NEAR    {payment['near_payout']}  {payment['amount_out']} USDC base units",
        f"   blocks  approval, accept, payout = {payment['blocks']}",
        "",
        "5. Recorded outcome",
        f"   {outcome['transaction']}",
        f"   {outcome['signer']}  {outcome['status']}  amount_out {outcome['amount_out']}  floor_met {outcome['floor_met']}",
        "   the earlier public-RPC observation still has outcome null; this is the later readback",
        "",
        "Limits",
    ]
    lines.extend(f"   {limit}" for limit in report["limits"])
    lines.append("")
    lines.append("PASS" if report["ok"] else "FAIL")
    return "\n".join(lines)


def main(argv: Optional[List[str]] = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--json", action="store_true", help="print the report as JSON")
    args = parser.parse_args(argv)
    with tempfile.TemporaryDirectory(prefix="x3-invoice06-sequence-") as tmp:
        report = evaluate(Path(tmp))
    print(json.dumps(report, indent=2, ensure_ascii=True) if args.json else render(report))
    return 0 if report["ok"] else 1


if __name__ == "__main__":
    sys.exit(main())
