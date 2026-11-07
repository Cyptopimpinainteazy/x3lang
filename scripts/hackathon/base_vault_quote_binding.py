#!/usr/bin/env python3
"""Read-only Base vault binding for a genuine Tidepool signed testnet quote.

Strictly validates maker Ed25519 signed terms, the real X3Lang frontend output,
a Guard policy observation, and *reviewer-supplied* proposed vault immutables.
An input Guard row is NOT proof it came from a blockchain; the CLI retrieves
the row from final NEAR RPC and pins the Guard code hash separately.

This script deliberately CANNOT sign, broadcast, deploy, approve tokens,
submit an agent payment or create an executable EVM transaction.
"""
from __future__ import annotations

import argparse
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import sys
from typing import Any

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "x3-lang"))
sys.path.insert(0, str(Path(__file__).resolve().parent))

import base_sepolia_guarded_admission as admission
from base_sepolia_x3_intent import compile_testnet_intent

AdmissionError = admission.AdmissionError

OWNER_TERMS_FIELDS = frozenset({
    "source_owner", "agent_signer", "origin_chain_id", "token_contract",
    "token_code_sha256", "destination_account", "refund_to",
    "deposit_address", "amount_base_units", "max_per_payment",
    "max_total", "approved_quote_sha256", "approved_program_sha256",
})


def _address(value: Any, field: str) -> str:
    admission.reject(
        isinstance(value, str) and len(value) == 42
        and value[:2] == "0x"
        and all(c in "0123456789abcdefABCDEF" for c in value[2:]),
        f"{field}_INVALID_EVM_ADDRESS",
    )
    return value.lower()


def _exact_address(actual: Any, expected: str, field: str) -> None:
    admission.reject(
        _address(actual, field) == _address(expected, field),
        f"{field}_MISMATCH",
    )


def _hex_hash(value: Any, field: str) -> str:
    admission.reject(
        isinstance(value, str) and len(value) == 64
        and all(c in "0123456789abcdef" for c in value),
        f"{field}_INVALID_SHA256",
    )
    return value


def make_unsigned_binding(
    response: dict[str, Any],
    guard_policy: dict[str, Any],
    owner_terms: dict[str, Any],
    *,
    now: datetime,
) -> dict[str, Any]:
    """Offline pure comparison; caller-supplied Guard policy is NOT chain proof."""
    admission.reject(
        isinstance(now, datetime) and now.utcoffset() is not None,
        "OFFSET_AWARE_NOW_REQUIRED",
    )
    admission.reject(
        isinstance(owner_terms, dict) and frozenset(owner_terms) == OWNER_TERMS_FIELDS,
        "EXACT_REVIEWER_TERMS_REQUIRED",
    )
    source = compile_testnet_intent()  # Real .x3 source parse + typecheck
    admission.reject(
        source["classification"] == "x3lang_python_frontend_testnet_policy_not_vm_bytecode",
        "NOT_THE_APPROVED_X3_FRONTEND_PATH",
    )
    quote = admission.check_quote(response, now)  # Genuine pinned Ed25519 maker key
    approval_quote_hash = _hex_hash(owner_terms["approved_quote_sha256"], "APPROVED_QUOTE")
    approval_program_hash = _hex_hash(owner_terms["approved_program_sha256"], "APPROVED_PROGRAM")
    admission.reject(
        approval_quote_hash == quote["quote_sha256"],
        "EXPLICIT_QUOTE_HASH_APPROVAL_MISMATCH",
    )
    admission.reject(
        approval_program_hash == source["program_sha256"],
        "EXPLICIT_X3_PROGRAM_HASH_APPROVAL_MISMATCH",
    )

    # A locally supplied Guard row is checked *for consistency only*. The CLI
    # obtains it by read-only finalized NEAR RPC, where code identity is pinned.
    admission.check_guard_policy(
        quote, guard_policy, owner=admission.RECEIVER,
        nonce=source["nonce"], now=now,
        program_sha=source["program_sha256"],
    )
    admission.enforce_compiled_x3_policy(source, quote, guard_policy, source["nonce"])

    admission.reject(
        type(owner_terms["origin_chain_id"]) is int
        and owner_terms["origin_chain_id"] == admission.CHAIN_ID,
        "BASE_SEPOLIA_CHAIN_ID_MISMATCH",
    )
    admission.reject(
        type(owner_terms["amount_base_units"]) is int
        and owner_terms["amount_base_units"] == admission.AMOUNT,
        "OWNER_SOURCE_AMOUNT_MISMATCH",
    )
    admission.reject(
        type(owner_terms["max_per_payment"]) is int
        and owner_terms["max_per_payment"] == admission.AMOUNT
        and type(owner_terms["max_total"]) is int
        and owner_terms["max_total"] == admission.AMOUNT,
        "SINGLE_QUOTE_VAULT_BUDGET_MISMATCH",
    )
    admission.reject(
        owner_terms["token_code_sha256"] == admission.TOKEN_CODE_SHA256,
        "PINNED_REAL_TOKEN_CODE_SHA256_MISMATCH",
    )
    admission.reject(
        owner_terms["destination_account"] == source["destination_account"]
        == admission.RECEIVER,
        "DESTINATION_NEAR_ACCOUNT_MISMATCH",
    )
    _exact_address(owner_terms["source_owner"], source["source_account"], "SOURCE_OWNER")
    _exact_address(owner_terms["token_contract"], admission.TOKEN, "BASE_TOKEN")
    _exact_address(owner_terms["refund_to"], source["refund_to"], "OWNER_REFUND")
    _exact_address(owner_terms["deposit_address"], quote["deposit_address"], "MAKER_DEPOSIT")
    agent = _address(owner_terms["agent_signer"], "AGENT_SIGNER")
    owner = _address(owner_terms["source_owner"], "SOURCE_OWNER")
    target = _address(owner_terms["deposit_address"], "MAKER_DEPOSIT")
    token = _address(owner_terms["token_contract"], "BASE_TOKEN")
    admission.reject(
        agent not in (owner, target, token, "0x" + "0" * 40),
        "AGENT_SIGNER_MUST_BE_SEPARATE_FROM_OWNER_TOKEN_AND_DEPOSIT",
    )
    admission.reject(
        target not in (owner, token, "0x" + "0" * 40),
        "SIGNED_QUOTE_DEPOSIT_IS_NOT_INDEPENDENT",
    )

    deadline_unix = int(quote["deadline"].timestamp())
    admission.reject(deadline_unix > int(now.timestamp()), "QUOTE_NOT_LIVE")
    nonce_bytes = hashlib.sha256(
        b"X3_SHIELD_SINGLE_QUOTE_BASE_VAULT_NONCE_V1\x00"
        + bytes.fromhex(quote["quote_sha256"])
        + bytes.fromhex(source["program_sha256"])
        + source["nonce"].encode("utf-8")
    ).hexdigest()

    return {
        "classification": "UNSIGNED_VAULT_BINDING_PREVIEW_NOT_AUTHORIZED",
        "route": "Base Sepolia test ERC20 -> Tidepool community testnet -> NEAR testnet",
        "verified_historical_maker_signature": True,
        "maker_key": admission.MAKER_KEY,
        "maker_signature_scope": "strict_testnet_signed_payload_SHA256_Ed25519",
        "quote_sha256": quote["quote_sha256"],
        "program_sha256": source["program_sha256"],
        "x3_policy_classification": source["classification"],
        "owner_nonce": source["nonce"],
        "guard_policy_checked_for_terms_only": True,
        "onchain_guard_observation_verified": False,
        "owner_signature_verified": False,
        "refund_to_owner_evm": owner,
        "refund_not_enforced_by_base_vault": True,
        "destination_near_account": admission.RECEIVER,
        "minimum_near_token_receive_units": quote["minimum_out"],
        "origin_chain_id": admission.CHAIN_ID,
        "pinned_base_token_code_sha256": admission.TOKEN_CODE_SHA256,
        "constructor_owner_is_deployer_not_constructor_argument": owner,
        "constructor_arguments": {
            "asset": token,
            "agent": agent,
            "recipient": target,
            "quoteHash": "0x" + quote["quote_sha256"],
            "programHash": "0x" + source["program_sha256"],
            "maxPerPayment": admission.AMOUNT,
            "maxTotal": admission.AMOUNT,
            "expiresAt": deadline_unix,
        },
        # No vault exists at this point. Cannot sign valid EIP712 without
        # an independently checked deployed vault address and chain id.
        "agent_order_template": {
            "nonce": "0x" + nonce_bytes,
            "recipient": target,
            "amount": admission.AMOUNT,
            "deadline": deadline_unix,
            "verifyingContract": "UNDEPLOYED_NO_EIP712_DIGEST",
        },
        "required_owner_reviews": [
            "Owner must independently authenticate and approve proposed deployment",
            "Vault's actual deployed immutable recipient/quoteHash/programHash/token/agent/limits must be queried before any signature",
            "Read-only maker signature and Guard approval MUST be refreshed immediately before deployment AND signing",
            "Token code hash, signer's isolation, refundTo ownership and testnet chain identity must be independently verified",
            "No vault transfer receipt, Base finality or NEAR payout has been observed",
        ],
        "signed": False,
        "deployed": False,
        "broadcast": False,
        "settlement_proven": False,
    }


def main() -> int:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--quote-file", required=True, type=Path)
    p.add_argument("--reviewer-terms-file", required=True, type=Path)
    p.add_argument("--guard-account", required=True)
    p.add_argument("--guard-wasm-sha256", required=True)
    args = p.parse_args()
    try:
        raw = json.loads(args.quote_file.read_text())
        admission.reject(
            isinstance(raw, dict)
            and raw.get("classification") == "LIVE_QUOTE_UNFUNDED_UNAUTHORIZED_NOT_SETTLED"
            and isinstance(raw.get("response"), dict),
            "LIVE_SIGNED_QUOTE_FILE_REQUIRED",
        )
        terms = json.loads(args.reviewer_terms_file.read_text())
        now = datetime.now(timezone.utc)
        # Validate maker signature/freshness and exact reviewer inputs BEFORE
        # network calls; never turn a stale signed quote into a live authority.
        # Since an on-chain Guard row is not yet fetched, this first pass is
        # a signature check only and is not exported as an authorization.
        admission.check_quote(raw["response"], now)
        _hex_hash(args.guard_wasm_sha256, "GUARD_WASM")
        row = admission.get_near_policy(
            args.guard_account, admission.RECEIVER,
            compile_testnet_intent()["nonce"], args.guard_wasm_sha256,
        )
        admission.reject(isinstance(row, dict), "ONCHAIN_GUARD_POLICY_MISSING")
        report = make_unsigned_binding(raw["response"], row, terms, now=datetime.now(timezone.utc))
        # Two-provider read-only Base wallet/token-chain observations. This
        # is NOT an on-chain vault code or EIP712 verifier observation.
        base = admission.check_base_state()
        admission.reject(base.get("chain_id") == admission.CHAIN_ID, "BASE_STATE_CHAIN_MISMATCH")
        # Deadline and final Guard policy might change during slow RPC calls.
        final_now = datetime.now(timezone.utc)
        row = admission.get_near_policy(
            args.guard_account, admission.RECEIVER,
            compile_testnet_intent()["nonce"], args.guard_wasm_sha256,
        )
        report = make_unsigned_binding(raw["response"], row, terms, now=final_now)
        report["onchain_guard_observation_verified"] = True
        report["onchain_guard_observation_scope"] = (
            "read_only_near_genesis_and_final_code_hash_policy_query_not_transaction_authorization"
        )
        report["guard_account"] = args.guard_account
        report["guard_wasm_sha256"] = args.guard_wasm_sha256
        report["base_rpc_coverage"] = len(base.get("rpc_observations", []))
        print(json.dumps(report, indent=2, sort_keys=True))
        return 0
    except (AdmissionError, OSError, TypeError, KeyError, ValueError, json.JSONDecodeError) as exc:
        print(json.dumps({
            "classification": "HARD_DENIED_NO_SIGNING_OR_DEPLOYMENT",
            "reason": str(exc), "signed": False, "deployed": False, "broadcast": False,
        }), file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
