#!/usr/bin/env python3
"""Strict TESTNET-ONLY Tidepool quote signature verifier.

This proves a response was signed by the pinned maker key; it does not authorize
funds, deposits, mainnet use, owner mandates, or claim X3 Guard compatibility.
"""
from __future__ import annotations

import hashlib
import json
from typing import Any

from near_intents import _decode_ed25519, ed25519

MAKER_ACCOUNT = "maker.tidepool-sandbox.testnet"
# Key independently observed on NEAR testnet `view_access_key_list` 2026-11-07 UTC.
# Do not fetch/rotate it automatically: any rotation needs explicit code review.
PINNED_TESTNET_MAKER_KEY = "ed25519:U1Mu4u1nHnHmgABpCkfDGXt2FZj8k29UGqhkMAu4iWY"
TESTNET_ORIGIN_ASSET = "nep141:base-0x38d81431b9f76d9699af7517040d5221d9dea252.tp-omft.testnet"
TESTNET_DEST_ASSET = "nep141:usdc.tidepool-sandbox.testnet"
ALLOWED_RECIPIENT = "x3lang.testnet"


class TidepoolQuoteRejected(ValueError):
    pass


def signed_bytes(response: dict[str, Any]) -> bytes:
    """Tidepool's observed signature input: SHA-256(compact response JSON without signature).

    Dict insertion order is significant; reject non-standard JSON scalar forms.
    This is a testnet-specific observed format, not a claim of official 1Click parity.
    """
    if not isinstance(response, dict) or not isinstance(response.get("signature"), str):
        raise TidepoolQuoteRejected("missing quote or signature")
    content = {key: value for key, value in response.items() if key != "signature"}
    try:
        return hashlib.sha256(json.dumps(content, ensure_ascii=False, separators=(",", ":"), allow_nan=False).encode()).digest()
    except (TypeError, ValueError) as exc:
        raise TidepoolQuoteRejected("quote payload is not canonical JSON") from exc


def verify_signature(response: dict[str, Any], *, public_key: str = PINNED_TESTNET_MAKER_KEY) -> bool:
    """Pure signature check; caller MUST also enforce the exact transaction policy."""
    if public_key != PINNED_TESTNET_MAKER_KEY:
        return False
    try:
        sig = _decode_ed25519(response.get("signature"), 64)
        pub = _decode_ed25519(public_key, 32)
        return bool(ed25519.verify(pub, signed_bytes(response), sig))
    except (ValueError, TypeError, TidepoolQuoteRejected):
        return False


def verify_dry_quote(response: dict[str, Any], *, amount: str = "1000000") -> dict[str, str | bool]:
    """Read-only valid *dry* quote for a fixed destination; no spending permission."""
    if not verify_signature(response):
        raise TidepoolQuoteRejected("UNTRUSTED_TESTNET_MAKER_SIGNATURE")
    req = response.get("quoteRequest", {})
    q = response.get("quote", {})
    if not isinstance(req, dict) or not isinstance(q, dict):
        raise TidepoolQuoteRejected("MALFORMED_QUOTE")
    expected = {"dry": True, "swapType": "EXACT_INPUT", "slippageTolerance": 100,
                "originAsset": TESTNET_ORIGIN_ASSET, "destinationAsset": TESTNET_DEST_ASSET,
                "depositType": "ORIGIN_CHAIN", "recipientType": "DESTINATION_CHAIN",
                "recipient": ALLOWED_RECIPIENT, "refundType": "ORIGIN_CHAIN", "amount": amount}
    if any(req.get(k) != v for k, v in expected.items()):
        raise TidepoolQuoteRejected("TESTNET_ROUTE_OR_RECIPIENT_POLICY_MISMATCH")
    if q.get("depositAddress") is not None:
        raise TidepoolQuoteRejected("DRY_QUOTE_MUST_NOT_HAVE_DEPOSIT_ADDRESS")
    try:
        amount_in = int(q["amountIn"])
        amount_out = int(q["amountOut"])
        minimum = int(q["minAmountOut"])
    except (KeyError, ValueError, TypeError) as exc:
        raise TidepoolQuoteRejected("INVALID_AMOUNT") from exc
    if str(amount_in) != amount or minimum <= 0 or minimum > amount_out:
        raise TidepoolQuoteRejected("QUOTE_AMOUNT_CONSTRAINT_FAILED")
    return {"classification": "signed_testnet_dry_quote_only", "signature_valid": True,
            "maker": MAKER_ACCOUNT, "maker_key": PINNED_TESTNET_MAKER_KEY,
            "amount_in": str(amount_in), "amount_out": str(amount_out),
            "min_amount_out": str(minimum), "recipient": ALLOWED_RECIPIENT,
            "x3_guard_authorized": False, "funds_moved": False}
