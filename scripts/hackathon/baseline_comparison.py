#!/usr/bin/env python3
"""Same signed 1Click quotes, three checks: only the X3 Guard check tells them apart.

    python3 scripts/hackathon/baseline_comparison.py
    python3 scripts/hackathon/baseline_comparison.py --json

Uses the two genuine 1Click manager-signed quotes that NEAR testnet Guard
`x3lang-guard-20261106.testnet` saw on 2026-11-07, from the archived evidence in
`evidence/near-guard-testnet/`:

  allowed    live quote, recipient defuse.near, against a policy for defuse.near
             -> accept_quote 6B6k5kre... succeeded
  forbidden  dry quote, recipient wrap.near, against a policy for
             x3lang-guard-20261106.testnet -> accept_quote 3Q7qTZqi... failed

Before comparing anything it proves each archived quote is the one sent on-chain: the SHA-256 of
the reconstructed `signed_payload` and of the signature must equal the hashes recorded for that
transaction's arguments. Then, for each quote, it asks:

  signature check     does the quote verify under the pinned 1Click manager key?
                      (real ed25519 verification, as any careful client would do)
  function-call key   would a NEAR function-call access key scoped to the call's receiver
                      contract and method allow the transaction? A key decides by
                      receiver_id and method_names, so the archived receiver and method are compared.
  X3 Guard terms      do the signed quote's request terms equal the registered policy's
                      (origin/destination asset, recipient, refund address, slippage, amount)
                      and is it a live quote? The policy is decoded from Guard's own
                      `register_intent` return value in the archive.

It passes only if both baselines accept both quotes, the X3 term check refuses the forbidden
quote for its recipient and allows the other, and that matches the on-chain outcome of each
`accept_quote`. A third, local-only row edits the forbidden quote's recipient without re-signing,
to show why the signature check is still needed.

Offline: reads committed files only. No RPC, key, transaction or deployment.
"""
from __future__ import annotations

import argparse
import base64
import copy
import hashlib
import importlib.util
import json
import sys
from pathlib import Path
from typing import Any, Dict, List, Optional

ROOT = Path(__file__).resolve().parents[2]
X3 = ROOT / "x3-lang"
EVIDENCE = ROOT / "evidence" / "near-guard-testnet"
GUARD = "x3lang-guard-20261106.testnet"

#: Signed-request field -> registered-policy field; the fields Guard's `quote_request_echo`
#: compares (x3-guard `accept_verified`), plus the amount.
ECHO_FIELDS = (
    ("originAsset", "origin_asset"),
    ("destinationAsset", "destination_asset"),
    ("recipient", "recipient"),
    ("refundTo", "refund_to"),
    ("slippageTolerance", "slippage_bps"),
    ("amount", "amount"),
)

CASES = (
    {"case": "allowed", "quote_file": "quote-live-official.json", "accept": "live_accept",
     "register_nonce": "evidence-allowed-live-02", "expect_allowed": True},
    {"case": "forbidden", "quote_file": "quote-dry-official.json", "accept": "prohibited_accept",
     "register_nonce": "evidence-prohibited-01", "expect_allowed": False},
)


def _load_adapter():
    sys.path.insert(0, str(X3))
    spec = importlib.util.spec_from_file_location("x3_near_intents", X3 / "near_intents.py")
    module = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    spec.loader.exec_module(module)
    return module


ni = _load_adapter()


class ArchiveMismatch(SystemExit):
    pass


def _sha(text: str) -> str:
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


def _status(tx: Dict[str, Any]) -> Dict[str, Any]:
    status = tx.get("status")
    return json.loads(status) if isinstance(status, str) else (status or {})


def _response(archived: Dict[str, Any]) -> Dict[str, Any]:
    return {k: archived[k] for k in ("quote", "quoteRequest", "signature", "timestamp") if k in archived}


def load_archive(evidence: Path = EVIDENCE) -> Dict[str, Any]:
    txs = json.loads((evidence / "transactions.json").read_text(encoding="utf-8"))["transactions"]
    by_label = {tx["label"]: tx for tx in txs}
    policies = {}
    for tx in txs:
        if tx.get("method_names") == ["register_intent"]:
            value = _status(tx).get("SuccessValue")
            if value:
                record = json.loads(base64.b64decode(value))
                policies[record["nonce"]] = record["policy"]
    views = json.loads((evidence / "check-quote-views.json").read_text(encoding="utf-8"))["views"]
    return {"txs": by_label, "policies": policies, "views": {v["nonce"]: v for v in views}}


def bind_to_chain(response: Dict[str, Any], tx: Dict[str, Any]) -> Dict[str, Any]:
    """Prove the archived quote is the one this transaction sent."""
    args = tx["sanitized_args"][0]["sanitized_args"]
    payload_ok = _sha(ni.signed_quote_payload(response)) == args["signed_payload"]["sha256"]
    signature_ok = _sha(response["signature"]) == args["signature"]["sha256"]
    if not (payload_ok and signature_ok):
        raise ArchiveMismatch(f"{tx['label']}: archived quote does not match the on-chain call arguments")
    return {"tx_hash": tx["tx_hash"], "nonce": args["nonce"], "block_height": tx.get("block_height")}


def signature_check(response: Dict[str, Any]) -> Dict[str, Any]:
    ok, reason = ni.check_oneclick_quote_signature(response)
    return {"allows": ok, "detail": reason}


def function_call_key(tx: Dict[str, Any]) -> Dict[str, Any]:
    scope = {"receiver_id": tx["receiver_id"], "method_names": list(tx["method_names"])}
    return {"allows": tx["receiver_id"] == GUARD and tx["method_names"] == ["accept_quote"], "scope": scope}


def x3_terms(response: Dict[str, Any], policy: Dict[str, Any]) -> Dict[str, Any]:
    request = response["quoteRequest"]
    mismatched = [sent for sent, held in ECHO_FIELDS if str(request.get(sent)) != str(policy.get(held))]
    live = request.get("dry") is False
    signature_ok = ni.check_oneclick_quote_signature(response)[0]
    reasons = ([] if signature_ok else ["quote_signature"]) + \
              (["quote_request_echo: " + ", ".join(mismatched)] if mismatched else []) + \
              ([] if live else ["quote_kind: dry quote cannot authorize a deposit"])
    return {"allows": not reasons, "mismatched": mismatched, "reasons": reasons}


def evaluate(evidence: Path = EVIDENCE) -> Dict[str, Any]:
    archive = load_archive(evidence)
    rows = []
    fc_scopes = []
    for case in CASES:
        archived = json.loads((evidence / case["quote_file"]).read_text(encoding="utf-8"))
        response = _response(archived)
        tx = archive["txs"][case["accept"]]
        binding = bind_to_chain(response, tx)
        policy = archive["policies"][case["register_nonce"]]
        if binding["nonce"] != case["register_nonce"]:
            raise ArchiveMismatch(f"{case['case']}: accept nonce {binding['nonce']} is not {case['register_nonce']}")
        fc = function_call_key(tx)
        fc_scopes.append(fc["scope"])
        x3 = x3_terms(response, policy)
        onchain_allowed = "SuccessValue" in _status(tx)
        view = archive["views"].get(case["register_nonce"])
        view_failed = [g["guard"] for g in (view or {}).get("guards", []) if not g["ok"]]
        rows.append({
            "case": case["case"],
            "quote_recipient": response["quoteRequest"]["recipient"],
            "policy_recipient": policy["recipient"],
            "onchain": binding,
            "signature_check": signature_check(response),
            "function_call_key": fc,
            "x3_guard_terms": x3,
            "onchain_accept_allowed": onchain_allowed,
            "archived_view_failed_guards": view_failed,
            "pass": (x3["allows"] == case["expect_allowed"] == onchain_allowed),
        })

    forbidden = next(r for r in rows if r["case"] == "forbidden")
    tampered_resp = _response(json.loads((evidence / "quote-dry-official.json").read_text(encoding="utf-8")))
    tampered_resp = copy.deepcopy(tampered_resp)
    tampered_resp["quoteRequest"]["recipient"] = forbidden["policy_recipient"]
    tampered = {
        "case": "tampered (local only)",
        "quote_recipient": tampered_resp["quoteRequest"]["recipient"],
        "policy_recipient": forbidden["policy_recipient"],
        "signature_check": signature_check(tampered_resp),
        "x3_guard_terms": x3_terms(tampered_resp, archive["policies"]["evidence-prohibited-01"]),
    }
    tampered["pass"] = (not tampered["signature_check"]["allows"]
                        and "quote_signature" in tampered["x3_guard_terms"]["reasons"])

    baselines_blind = all(r["signature_check"]["allows"] and r["function_call_key"]["allows"] for r in rows)
    same_scope = fc_scopes[0] == fc_scopes[1]
    recipient_reason = "recipient" in forbidden["x3_guard_terms"]["mismatched"] \
        and "quote_request_echo" in forbidden["archived_view_failed_guards"]
    ok = baselines_blind and same_scope and recipient_reason and all(r["pass"] for r in rows) and tampered["pass"]
    return {
        "title": "Same signed 1Click quotes, three checks",
        "scope": "archived NEAR testnet evidence, offline; no RPC, key, transaction or deployment",
        "guard": GUARD,
        "baselines_accept_both": baselines_blind,
        "function_call_scope_identical": same_scope,
        "ok": ok,
        "cases": rows,
        "tampered": tampered,
    }


def _mark(allows: bool) -> str:
    return "allows" if allows else "REFUSES"


def render(report: Dict[str, Any]) -> str:
    out = [report["title"], f"Scope: {report['scope']}.", ""]
    for r in report["cases"]:
        out.append(f"{r['case'].upper()} quote: recipient {r['quote_recipient']}, policy recipient "
                   f"{r['policy_recipient']}; on-chain accept_quote {r['onchain']['tx_hash'][:10]}... "
                   f"{'succeeded' if r['onchain_accept_allowed'] else 'failed'} (payload and signature hashes match the archived call)")
        fc = r["function_call_key"]["scope"]
        out.append(f"  signature check      {_mark(r['signature_check']['allows']):<8} {r['signature_check']['detail']}")
        out.append(f"  function-call key    {_mark(r['function_call_key']['allows']):<8} scope {fc['receiver_id']} / {', '.join(fc['method_names'])}")
        reasons = "; ".join(r["x3_guard_terms"]["reasons"]) or "all terms equal the registered policy"
        out.append(f"  X3 Guard terms       {_mark(r['x3_guard_terms']['allows']):<8} {reasons}")
        if r["archived_view_failed_guards"]:
            out.append(f"  archived Guard view  failed: {', '.join(r['archived_view_failed_guards'])}")
        out.append("")
    t = report["tampered"]
    out += [
        f"TAMPERED quote (local only): forbidden quote with recipient edited to {t['quote_recipient']}, not re-signed",
        f"  signature check      {_mark(t['signature_check']['allows']):<8} {t['signature_check']['detail']}",
        f"  X3 Guard terms       {_mark(t['x3_guard_terms']['allows']):<8} {'; '.join(t['x3_guard_terms']['reasons'])}",
        "",
        "Both baselines allow both real quotes: the signature is valid on each, and both accept_quote calls",
        f"have the same receiver and method ({'identical' if report['function_call_scope_identical'] else 'DIFFERENT'} key scope).",
        "Only the X3 term check refuses the forbidden one, matching the on-chain outcome. It has two",
        "independent reasons: the recipient differs from the policy, and it is a dry quote. A live quote",
        "with only the recipient wrong is the pending testnet proof (#172), not shown here.",
        f"Result: {'PASS' if report['ok'] else 'FAIL'}",
    ]
    return "\n".join(out)


def main(argv: Optional[List[str]] = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--json", action="store_true", help="print the report as JSON")
    args = parser.parse_args(argv)
    report = evaluate()
    print(json.dumps(report, indent=2) if args.json else render(report))
    return 0 if report["ok"] else 1


if __name__ == "__main__":
    sys.exit(main())
