#!/usr/bin/env python3
"""Read-only, two-independent-provider Base Sepolia test-USDC integrity evidence.

Does NOT verify Guard policy, sign transactions, authorize transfers or deploy.
Requires historical canonical block agreement and latest-finalized code pins.
"""
from __future__ import annotations

import argparse
from concurrent.futures import ThreadPoolExecutor
from datetime import datetime, timezone
import hashlib
import json
import re
from pathlib import Path
from urllib.parse import urlsplit
from urllib.request import Request, urlopen

CHAIN = 84532
TOKEN = "0x38d81431B9f76d9699AF7517040D5221D9dEA252"
CODE_SHA256 = "db77a21a647b5973bac5728bf9f6977b56933006bf2f0730a14d109d33072c83"
HIST_HEIGHT = 55_557_819
HIST_HASH = "0x1a95c4b60f2bc909969f52d24fd73727b237427a1ba60763214eebd03ea86157"
IMPL = "0x360894a13ba1a3210667c828492db98dca3e2076cc3735a920a3ca505d382bbc"
BEACON = "0xa3f0ad74e5423aebfd80d3ef4346578335a9a72aeaee59ff6cb3582b35133d50"
ZERO = "0x" + "0" * 64
DEFAULT_RPCS = (
    "https://base-sepolia.gateway.tenderly.co",
    "https://base-sepolia-rpc.publicnode.com",
)


def strict_rpc(url: str, method: str, params: list):
    parsed = urlsplit(url)
    if parsed.scheme != "https" or not parsed.hostname or parsed.username or parsed.password:
        raise ValueError("RPC endpoint must use credential-free HTTPS")
    request = Request(
        url,
        data=json.dumps({"jsonrpc": "2.0", "id": "x3-vault-readonly", "method": method, "params": params}).encode(),
        headers={"Content-Type": "application/json", "User-Agent": "X3Shield-Token-Audit/1"},
    )
    with urlopen(request, timeout=15) as handle:
        response = json.load(handle)
    if response.get("error") is not None or response.get("result") is None:
        raise ValueError(f"RPC failed closed for {method} at {parsed.hostname}")
    return response["result"]


def checked_block(url: str, number: int | str) -> dict:
    block = strict_rpc(url, "eth_getBlockByNumber", [hex(number) if isinstance(number, int) else number, False])
    if not isinstance(block, dict) or not block.get("hash") or not block.get("number"):
        raise ValueError("RPC returned incomplete canonical block")
    if not re.fullmatch(r"0x[0-9a-fA-F]{64}", block["hash"]):
        raise ValueError("RPC returned malformed block hash")
    try:
        returned_number = int(block["number"], 16)
    except (ValueError, TypeError) as exc:
        raise ValueError("RPC returned malformed block height") from exc
    if isinstance(number, int) and returned_number != number:
        raise ValueError("RPC returned different block height than requested")
    return block


def code_sha256(url: str, height: int) -> str:
    code = strict_rpc(url, "eth_getCode", [TOKEN, hex(height)])
    if not isinstance(code, str) or not code.startswith("0x") or len(code) <= 2:
        raise ValueError("missing historical token code")
    return hashlib.sha256(bytes.fromhex(code[2:])).hexdigest()


def token_slot(url: str, slot: str, height: int) -> str:
    value = strict_rpc(url, "eth_getStorageAt", [TOKEN, slot, hex(height)])
    if not isinstance(value, str) or not value.startswith("0x"):
        raise ValueError("missing proxy slot observation")
    return "0x" + value[2:].lower().zfill(64)


def capture_endpoint(url: str) -> dict:
    chain = int(strict_rpc(url, "eth_chainId", []), 16)
    historic = checked_block(url, HIST_HEIGHT)
    finalized = checked_block(url, "finalized")
    latest = int(finalized["number"], 16)
    if latest < HIST_HEIGHT:
        raise ValueError("RPC finalized head behind historical checkpoint")
    return {
        "rpc": url,
        "chain_id": chain,
        "historical_height": HIST_HEIGHT,
        "historical_hash": historic["hash"].lower(),
        "historical_code_sha256": code_sha256(url, HIST_HEIGHT),
        "finalized_height": latest,
        "finalized_head_hash": finalized["hash"].lower(),
        "finalized_code_sha256": code_sha256(url, latest),
        "impl_slot_historical": token_slot(url, IMPL, HIST_HEIGHT),
        "impl_slot_finalized": token_slot(url, IMPL, latest),
        "beacon_slot_historical": token_slot(url, BEACON, HIST_HEIGHT),
        "beacon_slot_finalized": token_slot(url, BEACON, latest),
    }


def attest_pair(a: dict, b: dict, common_a: str, common_b: str) -> dict:
    """Pure fail-closed verification; test with synthetic observations only."""
    endpoints = [urlsplit(x["rpc"]) for x in (a, b)]
    if any(x.scheme != "https" or not x.hostname or x.username or x.password for x in endpoints):
        raise ValueError("untrusted provider URL")
    if endpoints[0].hostname == endpoints[1].hostname:
        raise ValueError("independent providers required")
    for x in (a, b):
        if x["chain_id"] != CHAIN:
            raise ValueError("not Base Sepolia")
        if x["historical_height"] != HIST_HEIGHT or x["historical_hash"] != HIST_HASH:
            raise ValueError("historical canonical block disagreement")
        if x["finalized_height"] < HIST_HEIGHT:
            raise ValueError("provider not finalized past historical checkpoint")
        if x["historical_code_sha256"] != CODE_SHA256 or x["finalized_code_sha256"] != CODE_SHA256:
            raise ValueError("deployed test-USDC code changed or mismatched")
        for slot in (
            "impl_slot_historical",
            "impl_slot_finalized",
            "beacon_slot_historical",
            "beacon_slot_finalized",
        ):
            if x[slot] != ZERO:
                raise ValueError("standard EIP-1967 proxy storage slot not empty")
    if not (isinstance(common_a, str) and re.fullmatch(r"0x[0-9a-fA-F]{64}", common_a)):
        raise ValueError("invalid canonical checkpoint hash")
    if not (isinstance(common_b, str) and re.fullmatch(r"0x[0-9a-fA-F]{64}", common_b)):
        raise ValueError("invalid second canonical checkpoint hash")
    if common_a.lower() != common_b.lower():
        raise ValueError("independent finalized checkpoint hash mismatch")
    return {
        "common_finalized_height": min(a["finalized_height"], b["finalized_height"]),
        "common_finalized_hash": common_a.lower(),
        "standard_eip1967_slots_zero": True,
        "token_bytecode_matches_pinned_sha256": True,
        "provider_chains_confirmed": CHAIN,
    }


def capture(rpcs: tuple[str, str]) -> dict:
    if len(rpcs) != 2:
        raise ValueError("exactly two RPC providers required")
    # Two independent requests; never fall back to one RPC.
    with ThreadPoolExecutor(max_workers=2) as pool:
        a, b = list(pool.map(capture_endpoint, rpcs))
    common = min(a["finalized_height"], b["finalized_height"])
    with ThreadPoolExecutor(max_workers=2) as pool:
        block_a, block_b = list(pool.map(lambda url: checked_block(url, common), rpcs))
    summary = attest_pair(a, b, block_a["hash"], block_b["hash"])
    return {
        "schema": "x3.base_sepolia_token_readonly_attestation.v1",
        "classification": "PUBLIC_CHAIN_READ_ONLY_TWO_RPC_TOKEN_BYTECODE_AND_STANDARD_PROXY_SLOT_EVIDENCE",
        "observed_at_utc": datetime.now(timezone.utc).isoformat(),
        "testnet_chain_id": CHAIN,
        "contract_address": TOKEN,
        "token_code_sha256": CODE_SHA256,
        "historical_checkpoint_height": HIST_HEIGHT,
        "historical_checkpoint_hash": HIST_HASH,
        "providers": [a, b],
        "quorum": summary,
        "deployment_approved": False,
        "claims_excluded": [
            "does not authorize a testnet deployment or funds movement",
            "standard EIP-1967 zero slots do not rule out custom proxy/upgrade patterns",
            "token blacklist and privileged issuer policy remain unverified",
            "does not establish owner key custody, quote authenticity or atomic settlement",
            "not an EVM deployed-vault receipt or a human security audit",
        ],
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rpc", action="append", dest="rpcs")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    rpcs = tuple(args.rpcs or DEFAULT_RPCS)
    try:
        result = capture(rpcs)
        args.output.parent.mkdir(parents=True, exist_ok=True)
        raw = (json.dumps(result, sort_keys=True, indent=2) + "\n").encode()
        with args.output.open("xb") as out:
            out.write(raw)
        print(json.dumps({
            "result": "READ_ONLY_TWO_PROVIDER_QUORUM_PASSED",
            "output": str(args.output),
            "sha256": hashlib.sha256(raw).hexdigest(),
            "latest_common_finalized_height": result["quorum"]["common_finalized_height"],
            "deployment_approved": False,
        }, sort_keys=True))
        return 0
    except (ValueError, OSError, KeyError, TypeError) as err:
        print(json.dumps({"result": "HARD_DENIED_NO_ATTESTATION", "reason": str(err)}))
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
