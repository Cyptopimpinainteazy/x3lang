#!/usr/bin/env python3
"""Two-independent-NEAR-RPC *read-only* Guard policy attestation.

This is a quorum of final-state observers, NOT an authenticated validator-light-
client proof and NOT permission to sign, deploy or move any assets. Both RPCs
must pin the SAME canonical finalized checkpoint, Guard code and policy; each
provider must independently confirm the Guard has not changed as of its own
latest finalized head. Missing archive support and divergent observations hard
deny. The caller still checks signed quote terms, owner and program commitments.
"""
from __future__ import annotations

import base64
from concurrent.futures import ThreadPoolExecutor
import hashlib
import json
from urllib.parse import urlsplit
from typing import Any

try:
    from . import base_sepolia_guarded_admission as admission
except ImportError:
    import base_sepolia_guarded_admission as admission

NEAR_RPCS = (
    "https://test.rpc.fastnear.com",
    "https://rpc.testnet.near.org",
)
MAX_FINAL_HEAD_DRIFT = 60
CANONICAL_REVIEWED_TESTNET_GUARD = "x3guardlaunch2026.testnet"
CANONICAL_REVIEWED_TESTNET_GUARD_WASM_SHA256 = (
    "1d7a8b1ee97b7cdc500db1c19b8f3a2bef06e986576ec428f62f8c3ea1b5a145"
)

def validate_guard_identity(account: str, expected_code_sha: str) -> None:
    _must(account.endswith(".testnet") and account != admission.RECEIVER,
          "INVALID_TESTNET_GUARD_ACCOUNT")
    _must(isinstance(expected_code_sha, str) and len(expected_code_sha) == 64
          and all(c in "0123456789abcdef" for c in expected_code_sha),
          "INVALID_GUARD_WASM_SHA")
    _must(
        account == CANONICAL_REVIEWED_TESTNET_GUARD
        and expected_code_sha == CANONICAL_REVIEWED_TESTNET_GUARD_WASM_SHA256,
        "UNAPPROVED_TESTNET_GUARD_ACCOUNT_OR_WASM",
    )



def _must(ok: bool, reason: str) -> None:
    if not ok:
        raise ValueError(reason)


def _policy_payload(row: Any) -> str:
    _must(isinstance(row, dict), "MISSING_GUARD_POLICY")
    return json.dumps(row, sort_keys=True, separators=(",", ":"), ensure_ascii=False, allow_nan=False)


def verify_guard_views(
    views: list[dict[str, Any]], *, expected_code_sha: str, owner: str, nonce: str,
) -> dict[str, Any]:
    """Pure check of explicit RPC observations; synthetic inputs are NOT chain evidence."""
    _must(expected_code_sha == CANONICAL_REVIEWED_TESTNET_GUARD_WASM_SHA256,
          "UNAPPROVED_TESTNET_GUARD_WASM")
    _must(len(views) == 2, "TWO_INDEPENDENT_NEAR_RPCS_REQUIRED")
    hosts = []
    for v in views:
        parsed = urlsplit(v["rpc"])
        _must(
            parsed.scheme == "https" and bool(parsed.hostname)
            and not parsed.username and not parsed.password,
            "GUARD_RPC_MUST_USE_CREDENTIAL_FREE_HTTPS",
        )
        hosts.append(parsed.hostname)
    _must(hosts[0] != hosts[1], "DUPLICATE_NEAR_RPC_PROVIDER")
    heights = [v["head_height"] for v in views]
    _must(all(type(x) is int and x >= 1 for x in heights), "INVALID_NEAR_FINALIZED_HEIGHT")
    _must(max(heights) - min(heights) <= MAX_FINAL_HEAD_DRIFT, "NEAR_RPC_FINALITY_LAG")
    checkpoint_height = min(heights)
    reference = views[0]
    _must(
        type(reference["checkpoint_height"]) is int and reference["checkpoint_height"] == checkpoint_height,
        "GUARD_CHECKPOINT_HEIGHT_INVALID",
    )
    _must(
        isinstance(reference["checkpoint_hash"], str) and len(reference["checkpoint_hash"]) >= 30,
        "GUARD_CHECKPOINT_HASH_INVALID",
    )
    reference_policy = _policy_payload(reference["checkpoint_policy"])
    _must(
        reference["checkpoint_policy"].get("owner") == owner
        and reference["checkpoint_policy"].get("nonce") == nonce
        and reference["checkpoint_policy"].get("revoked") is False,
        "GUARD_OWNER_NONCE_OR_LIFECYCLE_MISMATCH",
    )
    for v in views:
        _must(v["chain_id"] == "testnet" and v["genesis_hash"] == admission.NEAR_GENESIS,
              "GUARD_NETWORK_GENESIS_MISMATCH")
        _must(v["checkpoint_height"] == checkpoint_height and v["checkpoint_hash"] == reference["checkpoint_hash"],
              "NEAR_CANONICAL_FINALIZED_CHECKPOINT_MISMATCH")
        _must(v["checkpoint_code_sha256"] == expected_code_sha and v["head_code_sha256"] == expected_code_sha,
              "GUARD_DEPLOYED_WASM_MISMATCH_OR_CHANGED")
        _must(_policy_payload(v["checkpoint_policy"]) == reference_policy,
              "GUARD_POLICY_QUORUM_DISAGREEMENT")
        _must(_policy_payload(v["head_policy"]) == reference_policy,
              "GUARD_POLICY_CHANGED_AT_FINALIZED_HEAD")
        _must(isinstance(v["head_hash"], str) and len(v["head_hash"]) >= 30,
              "GUARD_FINALIZED_HEAD_HASH_INVALID")
    return {
        "classification": "TWO_INDEPENDENT_NEAR_FINALIZED_RPC_GUARD_OBSERVATIONS_NO_VALIDATOR_PROOF",
        "policy": reference["checkpoint_policy"],
        "checkpoint": {"height": checkpoint_height, "hash": reference["checkpoint_hash"]},
        "provider_count": 2,
        "finalized_head_heights": heights,
        "trustless_validator_finality_proven": False,
        "signing_or_funding_authorized": False,
    }


def _get_code(url: str, account: str, block_hash: str) -> str:
    code = admission.rpc(url, "query", {
        "request_type": "view_code", "block_id": block_hash, "account_id": account,
    })
    _must(isinstance(code, dict) and code.get("block_hash") == block_hash
          and isinstance(code.get("code_base64"), str),
          "GUARD_CODE_RESPONSE_WRONG_BLOCK")
    return hashlib.sha256(base64.b64decode(code["code_base64"], validate=True)).hexdigest()


def _get_policy(url: str, account: str, owner: str, nonce: str, block_hash: str) -> dict:
    args = base64.b64encode(json.dumps(
        {"owner": owner, "nonce": nonce}, separators=(",", ":"),
    ).encode()).decode()
    v = admission.rpc(url, "query", {
        "request_type": "call_function", "block_id": block_hash,
        "account_id": account, "method_name": "get_policy", "args_base64": args,
    })
    _must(isinstance(v, dict) and v.get("block_hash") == block_hash
          and isinstance(v.get("result"), list), "GUARD_POLICY_RESPONSE_WRONG_BLOCK")
    raw = bytes(v["result"]).decode("utf-8")
    row = json.loads(raw)
    _must(isinstance(row, dict), "INVALID_GUARD_POLICY_JSON")
    return row


def _sample_head(url: str) -> dict:
    status = admission.rpc(url, "status", [])
    _must(
        isinstance(status, dict) and status.get("chain_id") == "testnet"
        and status.get("genesis_hash") == admission.NEAR_GENESIS,
        "NEAR_NETWORK_GENESIS_MISMATCH",
    )
    block = admission.rpc(url, "block", {"finality": "final"})
    header = block.get("header") if isinstance(block, dict) else None
    _must(isinstance(header, dict) and type(header.get("height")) is int
          and isinstance(header.get("hash"), str) and len(header["hash"]) >= 30,
          "NO_CANONICAL_NEAR_FINALIZED_HEAD")
    return {"rpc": url, "chain_id": status["chain_id"],
            "genesis_hash": status["genesis_hash"],
            "head_height": header["height"], "head_hash": header["hash"]}


def _checkpoint_observation(
    head: dict, *, account: str, owner: str, nonce: str,
    checkpoint_height: int,
) -> dict:
    url = head["rpc"]
    block = admission.rpc(url, "block", {"block_id": checkpoint_height})
    header = block.get("header") if isinstance(block, dict) else None
    _must(isinstance(header, dict) and header.get("height") == checkpoint_height
          and isinstance(header.get("hash"), str) and len(header["hash"]) >= 30,
          "NEAR_PROVIDER_MISSING_CANONICAL_CHECKPOINT")
    at = header["hash"]
    return {
        **head,
        "checkpoint_height": checkpoint_height, "checkpoint_hash": at,
        "checkpoint_code_sha256": _get_code(url, account, at),
        "checkpoint_policy": _get_policy(url, account, owner, nonce, at),
        "head_code_sha256": _get_code(url, account, head["head_hash"]),
        "head_policy": _get_policy(url, account, owner, nonce, head["head_hash"]),
    }


def observe_guard_policy(
    account: str, owner: str, nonce: str, expected_code_sha: str,
    *, rpcs: tuple[str, str] = NEAR_RPCS,
) -> dict:
    """Make actual read-only queries to both public NEAR testnet RPCs."""
    validate_guard_identity(account, expected_code_sha)
    _must(len(rpcs) == 2, "TWO_INDEPENDENT_NEAR_RPCS_REQUIRED")
    with ThreadPoolExecutor(max_workers=2) as executor:
        heads = list(executor.map(_sample_head, rpcs))
    heights = [h["head_height"] for h in heads]
    _must(max(heights) - min(heights) <= MAX_FINAL_HEAD_DRIFT, "NEAR_RPC_FINALITY_LAG")
    common = min(heights)
    with ThreadPoolExecutor(max_workers=2) as executor:
        views = list(executor.map(
            lambda h: _checkpoint_observation(
                h, account=account, owner=owner, nonce=nonce,
                checkpoint_height=common), heads,
        ))
    return verify_guard_views(views, expected_code_sha=expected_code_sha, owner=owner, nonce=nonce)


def main() -> int:
    """Archive real read-only observations only; does not verify quote freshness."""
    import argparse
    from datetime import datetime, timezone
    from pathlib import Path

    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--guard-account", required=True)
    parser.add_argument("--owner", required=True)
    parser.add_argument("--nonce", required=True)
    parser.add_argument("--wasm-sha256", required=True)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()

    try:
        observation = observe_guard_policy(
            args.guard_account, args.owner, args.nonce, args.wasm_sha256,
        )
        result = {
            "schema": "x3.near_guard_dual_finalized_rpc_observation.v1",
            "captured_utc": datetime.now(timezone.utc).isoformat(),
            "classification": observation["classification"],
            "account": args.guard_account,
            "owner": args.owner,
            "nonce": args.nonce,
            "wasm_sha256": args.wasm_sha256,
            "rpc_urls": list(NEAR_RPCS),
            "observation": observation,
            "quote_freshness_or_maker_signature_verified": False,
            "cryptographic_validator_finality_proven": False,
            "onchain_transaction_or_approval_broadcast": False,
            "vault_deployment_approved": False,
            "caution": (
                "Two independently agreeing public NEAR RPC observers are not "
                "a validator-signature authenticated light client. Historical "
                "approval does not mean the prior maker quote is still live."
            ),
        }
        serialized = (json.dumps(result, sort_keys=True, indent=2) + "\n").encode()
        args.output.parent.mkdir(parents=True, exist_ok=True)
        with args.output.open("xb") as file:
            file.write(serialized)
        print(json.dumps({
            "classification": result["classification"],
            "sha256": hashlib.sha256(serialized).hexdigest(),
            "output": str(args.output),
            "checkpoint": observation["checkpoint"],
            "deployment_approved": False,
        }, sort_keys=True))
        return 0
    except (ValueError, KeyError, TypeError, OSError, admission.AdmissionError) as err:
        print(json.dumps({
            "classification": "HARD_DENIED_NO_GUARD_ATTESTATION",
            "reason": str(err),
            "signed": False,
            "deployed": False,
        }))
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
