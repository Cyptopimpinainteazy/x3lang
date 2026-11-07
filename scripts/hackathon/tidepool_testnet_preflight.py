#!/usr/bin/env python3
"""Read-only discovery for a prospective Base Sepolia -> NEAR testnet X3 trade.

This program never requests live quotes, asks for signatures, calls a faucet,
prepares deposits, or claims that provider metadata is chain-verified.
"""
from __future__ import annotations

import argparse
import base64
import hashlib
import json
import sys
from urllib.request import Request, urlopen

API = "https://www.usetidepool.tech"
RPC = "https://test.rpc.fastnear.com"
DESTINATION = "x3lang.testnet"
VERIFIER = "verifier.tidepool-sandbox.testnet"
GUARD = "x3lang-guard-20261106.testnet"
EXPECTED_GUARD_SHA256 = "ac44fa53acf514d26c7adf883b31b49a125faf5a4c1074928d3e1fe94b36f8ab"
ORIGIN_ASSET = "nep141:base-0x38d81431b9f76d9699af7517040d5221d9dea252.tp-omft.testnet"
DEST_ASSET = "nep141:usdc.tidepool-sandbox.testnet"


def fetch_json(url: str, *, payload: dict | None = None) -> dict | list:
    data = None if payload is None else json.dumps(payload, sort_keys=True).encode()
    req = Request(url, data=data, headers={"Content-Type": "application/json", "User-Agent": "X3Lang-readonly-preflight/1"})
    with urlopen(req, timeout=12) as response:
        if response.status != 200:
            raise RuntimeError(f"unexpected HTTP status {response.status}")
        return json.load(response)


def view_account(account: str) -> dict:
    response = fetch_json(RPC, payload={"jsonrpc": "2.0", "id": "x3-preflight", "method": "query", "params": {"request_type": "view_account", "finality": "final", "account_id": account}})
    if "error" in response or "result" not in response:
        raise RuntimeError(f"NEAR account view failed: {account}")
    return response["result"]


def view_code(account: str) -> dict:
    response = fetch_json(RPC, payload={"jsonrpc": "2.0", "id": "x3-preflight", "method": "query", "params": {"request_type": "view_code", "finality": "final", "account_id": account}})
    if "error" in response or "result" not in response:
        raise RuntimeError(f"NEAR code view failed: {account}")
    result = response["result"]
    return {"sha256": hashlib.sha256(base64.b64decode(result["code_base64"], validate=True)).hexdigest(), "block_height": result["block_height"], "rpc_code_hash": result["hash"]}


def select_pair(tokens: list[dict]) -> tuple[dict, dict]:
    indexed = {t["assetId"]: t for t in tokens if isinstance(t, dict) and "assetId" in t}
    if ORIGIN_ASSET not in indexed or DEST_ASSET not in indexed:
        raise RuntimeError("required Base Sepolia / NEAR testnet USDC pair absent")
    origin, destination = indexed[ORIGIN_ASSET], indexed[DEST_ASSET]
    if (origin.get("blockchain"), destination.get("blockchain")) != ("base", "near"):
        raise RuntimeError("token chain mismatch")
    if any(t.get("decimals") != 6 or t.get("symbol") != "USDC" for t in (origin, destination)):
        raise RuntimeError("token precision/symbol mismatch")
    return origin, destination


def build_report(health: dict, tokens: list[dict], account: dict, guard_code: dict, verifier_code: dict) -> dict:
    if health.get("network") != "testnet" or health.get("verifier") != VERIFIER:
        raise RuntimeError("provider network/verifier mismatch")
    origin, destination = select_pair(tokens)
    if guard_code["sha256"] != EXPECTED_GUARD_SHA256:
        raise RuntimeError("deployed X3 Guard does not match the reviewed release")
    if verifier_code["sha256"] != health.get("verifierWasmSha256"):
        raise RuntimeError("provider advertised verifier hash differs from public NEAR RPC")
    return {
        "classification": "read_only_preflight_not_execution_proof",
        "destination": {"account": DESTINATION, "exists": True, "balance_yocto": account["amount"], "block_height": account["block_height"]},
        "provider": {"name": "Tidepool (community-run, unaffiliated with NEAR/1Click)", "url": API, "network": "testnet", "untrusted_relayer": True},
        "advertised_pair_not_independently_verified": {"origin": origin, "destination": destination},
        "near_rpc_code": {"guard": guard_code, "verifier": verifier_code},
        "authorization_state": "BLOCKED_DIFFERENT_QUOTE_MANAGER_KEY_AND_GUARD_ABI",
        "required_before_live": ["verify Base Sepolia token contract from independent RPC", "verify testnet quote manager key and signature against exact payload", "establish separate testnet Guard authorization without changing mainnet key pin", "owner-controlled Base Sepolia sender and refund address", "owner approval of amount/gas/slippage/refund", "verify finalized source transfer and destination NEAR balance delta"],
        "signed": False, "funds_moved": False, "quote_requested": False, "settlement_proven": False,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--account", default=DESTINATION, choices=[DESTINATION], help="Only the approved project testnet destination is accepted")
    args = parser.parse_args()
    try:
        health = fetch_json(API + "/v0/health")
        tokens = fetch_json(API + "/v0/tokens")
        report = build_report(health, tokens, view_account(args.account), view_code(GUARD), view_code(VERIFIER))
        print(json.dumps(report, sort_keys=True, indent=2))
        return 0
    except (OSError, ValueError, KeyError, TypeError, RuntimeError) as exc:
        print(json.dumps({"classification": "preflight_failed", "reason": str(exc), "signed": False, "funds_moved": False}), file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
