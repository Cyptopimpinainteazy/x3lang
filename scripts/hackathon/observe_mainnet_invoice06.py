#!/usr/bin/env python3
"""Read-only public-RPC observation of the invoice_06 mainnet settlement.

    python3 scripts/hackathon/observe_mainnet_invoice06.py --out observation.json

Reads NEAR mainnet and Solana mainnet through public JSON-RPC. Signs, sends and stores nothing
on any chain, and needs no key or token. It records what the chains return for the
transactions named in the owner's settlement record and checks the claims that can be read
directly; every claim it cannot read is reported as not checked, never as passed.

Exit 0 only when every check that ran passed. A transport failure is a failed check.
"""
from __future__ import annotations

import argparse
import base64
import json
import sys
import time
import urllib.request
from typing import Any, Dict, List, Optional

NEAR_RPCS = ("https://free.rpc.fastnear.com", "https://rpc.mainnet.near.org",
             "https://archival-rpc.mainnet.fastnear.com")
SOLANA_RPCS = ("https://api.mainnet-beta.solana.com",)

GUARD = "x3lang.near"
AGENT = "x3-agent.x3lang.near"
NONCE = "mainnet_demo_nov11_invoice_06"
NEAR_USDC = "17208628f84f5d6ad33f0da3bbbeb27ffcb398eac501a31bd6ad2011e36133a1"
SOL_USDC_MINT = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v"
#: The restricted key and the retained owner key on the agent account, from PR #207's evidence.
DELEGATED_KEY = "ed25519:J3NiU8J63HAfz2MTDu1zQVhFRYS5Pio21h1kNtJdP11x"
OWNER_RECOVERY_KEY = "ed25519:8teaXGYFELKQN5QA7gQqeA4B7URrcMQqaczZ5D68c23u"
DEPLOYED_CODE_HASH = "BPdpJAw8R4SDh6Wi7dPcXTs5T7ncJDVKCLgzuqZFb15T"

NEAR_TXS = [
    ("owner_approval", "GFdQLrzT5tnPPdZwKLtmwpNvSwrxVHDvVg2C3WX5fsK6", GUARD),
    ("agent_storage", "Ax9yNKUicGoyR3fcStygFQypUQkmfSKEFYDmRxbK9ZA", GUARD),
    ("agent_accept_quote", "Eg75uehwU4YJ86ZZPEPKiBqnjkr6PxwPmV23HFAgu153", AGENT),
    ("payout", "8jfbACLSxrzi3fnajfAVGc4Fgo6XLCWA1mpiJWXPw43P", "intents.near"),
    ("deployment", "BVXX3hJEmThWVZNpUknZv9HCpGezisiN8SQKsZgA3faF", GUARD),
]
SOL_DEPOSIT = "4zhGYyWNabYQ5gixTUzoVHVbgFKWAsQssSpCv3B1J1LjCRisEJW6wm3zuS2GVuDotAT93S8wm1qezHwsKX2RkXo5"
SOL_FUNDER = "58tbMfadmi8HWzUndLqkX3prBbkiZMEaYMTDYSzhB3cQ"
SOL_DEPOSIT_ADDRESS = "C6978ATtM33Acw79Dh3y6K5syfzJtwcvo6t6b62UdkN"


def rpc(urls, method: str, params: Any) -> Dict[str, Any]:
    errors = []
    for url in urls:
        body = json.dumps({"jsonrpc": "2.0", "id": "x3", "method": method, "params": params}).encode()
        req = urllib.request.Request(url, data=body, headers={"content-type": "application/json",
                                                              "user-agent": "x3-readonly-observer"})
        for attempt in range(3):
            try:
                with urllib.request.urlopen(req, timeout=30) as resp:
                    out = json.load(resp)
                if "error" in out:
                    errors.append({"url": url, "error": out["error"]})
                    break
                return {"url": url, "result": out["result"]}
            except Exception as exc:  # transport failure: retry, then next endpoint
                errors.append({"url": url, "attempt": attempt, "error": repr(exc)})
                time.sleep(2 * (attempt + 1))
    return {"errors": errors}


def decode_args(b64: Optional[str]) -> Any:
    if not b64:
        return None
    raw = base64.b64decode(b64)
    try:
        return json.loads(raw)
    except ValueError:
        return {"raw_base64": b64}


def near_view(method: str, args: Dict[str, Any], account: str = GUARD) -> Dict[str, Any]:
    got = rpc(NEAR_RPCS, "query", {"request_type": "call_function", "finality": "final", "account_id": account,
                                   "method_name": method,
                                   "args_base64": base64.b64encode(json.dumps(args).encode()).decode()})
    if "result" in got:
        got["value"] = json.loads(bytes(got["result"].pop("result")).decode() or "null")
    return got


def near_tx(name: str, tx_hash: str, sender: str) -> Dict[str, Any]:
    got = rpc(NEAR_RPCS, "EXPERIMENTAL_tx_status",
              {"tx_hash": tx_hash, "sender_account_id": sender, "wait_until": "FINAL"})
    if "result" not in got:
        return {"name": name, "hash": tx_hash, **got}
    r = got["result"]
    tx = r["transaction"]
    actions = []
    for a in tx.get("actions", []):
        if isinstance(a, dict) and "FunctionCall" in a:
            fc = a["FunctionCall"]
            actions.append({"FunctionCall": {"method_name": fc["method_name"], "args": decode_args(fc.get("args")),
                                             "gas": fc.get("gas"), "deposit": fc.get("deposit")}})
        else:
            actions.append(a)
    receipts = []
    for o in r.get("receipts_outcome", []):
        oc = o["outcome"]
        status = oc["status"]
        if isinstance(status, dict) and "SuccessValue" in status:
            sv = status["SuccessValue"]
            status = {"SuccessValue": decode_args(sv) if sv else ""}
        receipts.append({"id": o["id"], "block_hash": o["block_hash"], "executor": oc["executor_id"],
                         "logs": oc["logs"], "status": status})
    blk = rpc(NEAR_RPCS, "block", {"block_id": r["transaction_outcome"]["block_hash"]})
    header = (blk.get("result") or {}).get("header", {})
    return {"name": name, "hash": tx_hash, "rpc": got["url"], "final_execution_status": r.get("final_execution_status"),
            "status": r.get("status"), "signer_id": tx["signer_id"], "public_key": tx["public_key"],
            "receiver_id": tx["receiver_id"], "nonce": tx["nonce"], "actions": actions,
            "included_block_hash": r["transaction_outcome"]["block_hash"], "included_block_height": header.get("height"),
            "included_block_timestamp_ns": header.get("timestamp"), "receipts": receipts}


def sol_tx(sig: str) -> Dict[str, Any]:
    got = rpc(SOLANA_RPCS, "getTransaction",
              [sig, {"encoding": "jsonParsed", "commitment": "finalized", "maxSupportedTransactionVersion": 0}])
    if "result" not in got or got["result"] is None:
        return {"signature": sig, **got, "found": False}
    r = got["result"]
    keys = r["transaction"]["message"]["accountKeys"]
    signers = [k["pubkey"] for k in keys if k.get("signer")]

    def bal(entries):
        return [{"owner": e.get("owner"), "mint": e["mint"], "amount": e["uiTokenAmount"]["amount"]}
                for e in entries or []]
    return {"signature": sig, "rpc": got["url"], "found": True, "slot": r["slot"], "block_time": r["blockTime"],
            "err": r["meta"]["err"], "signers": signers, "fee_lamports": r["meta"]["fee"],
            "pre_token_balances": bal(r["meta"].get("preTokenBalances")),
            "post_token_balances": bal(r["meta"].get("postTokenBalances")),
            "log_messages": r["meta"].get("logMessages")}


def token_delta(sol: Dict[str, Any], owner: str) -> Optional[int]:
    pre = sum(int(b["amount"]) for b in sol.get("pre_token_balances", []) if b["owner"] == owner and b["mint"] == SOL_USDC_MINT)
    post = sum(int(b["amount"]) for b in sol.get("post_token_balances", []) if b["owner"] == owner and b["mint"] == SOL_USDC_MINT)
    seen = any(b["owner"] == owner for b in sol.get("post_token_balances", []) + sol.get("pre_token_balances", []))
    return post - pre if seen else None


def main(argv: Optional[List[str]] = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--out", default="observation.json")
    args = ap.parse_args(argv)

    txs = {name: near_tx(name, h, s) for name, h, s in NEAR_TXS}
    sol = sol_tx(SOL_DEPOSIT)
    guard_account = rpc(NEAR_RPCS, "query", {"request_type": "view_account", "finality": "final", "account_id": GUARD})
    agent_keys = rpc(NEAR_RPCS, "query", {"request_type": "view_access_key_list", "finality": "final", "account_id": AGENT})
    intent = near_view("get_intent", {"owner": GUARD, "nonce": NONCE})
    mandate = near_view("get_mandate", {"owner": GUARD, "agent": AGENT})
    manager = near_view("get_manager_public_key", {})
    balance = near_view("ft_balance_of", {"account_id": GUARD}, account=NEAR_USDC)

    checks: List[Dict[str, Any]] = []

    def check(name: str, ok: Optional[bool], detail: Any) -> None:
        checks.append({"check": name, "result": "not_checked" if ok is None else ("pass" if ok else "FAIL"),
                       "detail": detail})

    def success(t):
        st = t.get("status")
        return isinstance(st, dict) and "SuccessValue" in st and not any(
            isinstance(r["status"], dict) and "Failure" in r["status"] for r in t.get("receipts", []))

    for name, t in txs.items():
        check(f"{name}_final_success", success(t) if "signer_id" in t else False,
              {"status": t.get("status"), "final": t.get("final_execution_status"), "errors": t.get("errors")})
    acc = txs["agent_accept_quote"]
    method = (((acc.get("actions") or [{}])[0]).get("FunctionCall") or {}).get("method_name")
    check("agent_call_signer_is_agent", acc.get("signer_id") == AGENT if "signer_id" in acc else False, acc.get("signer_id"))
    check("agent_call_method", method == "agent_accept_quote" if method else False, method)
    key = acc.get("public_key")
    check("agent_call_used_restricted_function_call_key", key == DELEGATED_KEY if key else False,
          {"public_key": key, "restricted_key": DELEGATED_KEY, "owner_full_access_key": OWNER_RECOVERY_KEY})
    acc_args = (((acc.get("actions") or [{}])[0]).get("FunctionCall") or {}).get("args") or {}
    check("agent_call_nonce", acc_args.get("nonce") == NONCE if acc_args else False, acc_args.get("nonce"))
    appr = txs["owner_approval"]
    appr_args = (((appr.get("actions") or [{}])[0]).get("FunctionCall") or {}).get("args") or {}
    check("approval_nonce", appr_args.get("nonce") == NONCE if appr_args else False, appr_args.get("nonce"))
    hs = [txs[n].get("included_block_height") for n in ("owner_approval", "agent_accept_quote", "payout")]
    check("approval_before_accept_before_payout", (hs[0] < hs[1] < hs[2]) if all(hs) else False, hs)
    check("guard_code_hash_is_reviewed_deployment",
          (guard_account.get("result") or {}).get("code_hash") == DEPLOYED_CODE_HASH if "result" in guard_account else False,
          (guard_account.get("result") or {}).get("code_hash"))
    iv = intent.get("value")
    check("intent_registered_by_agent", (iv or {}).get("agent") == AGENT if iv is not None else False,
          {k: (iv or {}).get(k) for k in ("agent", "mandate_version", "approvals")} if iv else intent.get("errors"))
    check("intent_has_accepted_quote", bool((iv or {}).get("quote")) if iv is not None else False,
          bool((iv or {}).get("quote")))
    check("sol_deposit_finalized_no_error", (sol.get("found") and sol.get("err") is None) or False,
          {"found": sol.get("found"), "err": sol.get("err"), "slot": sol.get("slot")})
    check("sol_signer_is_funder", (SOL_FUNDER in sol.get("signers", [])) if sol.get("found") else False, sol.get("signers"))
    d_funder, d_deposit = token_delta(sol, SOL_FUNDER), token_delta(sol, SOL_DEPOSIT_ADDRESS)
    check("sol_25_usdc_funder_to_deposit_address",
          (d_funder == -25_000_000 and d_deposit == 25_000_000) if sol.get("found") else False,
          {"funder_delta": d_funder, "deposit_address_delta": d_deposit})
    payout_logs = " ".join(" ".join(r["logs"]) for r in txs["payout"].get("receipts", []))
    check("payout_logs_name_recipient_and_amount",
          (GUARD in payout_logs and "24995200" in payout_logs) if payout_logs else False,
          [l for r in txs["payout"].get("receipts", []) for l in r["logs"] if "ft_transfer" in l or "24995200" in l][:6])
    check("controller_of_funding_wallet", None, "not readable on-chain; owner must state who controls " + SOL_FUNDER)

    report = {"schema": "x3.mainnet.invoice06.public-rpc-observation.v1", "observed_at_unix": int(time.time()),
              "scope": "read-only public RPC; no signing, no keys, no state changes",
              "near_transactions": txs, "solana_deposit": sol, "guard_account": guard_account,
              "agent_access_keys": agent_keys, "intent": intent, "mandate": mandate,
              "manager_public_key": manager, "guard_near_usdc_balance": balance, "checks": checks}
    with open(args.out, "w", encoding="utf-8") as f:
        json.dump(report, f, indent=2, sort_keys=True)
        f.write("\n")
    for c in checks:
        print(f"{c['result']:<12} {c['check']}: {json.dumps(c['detail'])[:220]}")
    failed = [c for c in checks if c["result"] == "FAIL"]
    print(f"\n{len(checks) - len(failed)}/{len(checks)} checks not failed; {len(failed)} failed")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
