#!/usr/bin/env python3
"""Classify original NEAR JSON-RPC permission errors; never authenticate a chain.

Read-only and dependency-free. Inputs must be collected independently from the
local node and the intended request. A caller-supplied response can be fabricated:
this module does NOT verify signatures, key scope, balances, execution, or finality.

Wire format: https://docs.near.org/api/rpc/transactions#invalid-transactions
The pinned SDK's generic InvalidTransaction text is deliberately insufficient.
"""
from __future__ import annotations

import argparse
from dataclasses import dataclass
import hashlib
import json
import math
from pathlib import Path
import sys
from typing import Any

MAX_RESPONSE_BYTES = 1_048_576
MAX_EXPECTATION_BYTES = 4096


class RefusalEvidenceError(ValueError):
    """The observation is missing, ambiguous, or not the expected rejection."""


def _require(condition: bool, message: str) -> None:
    if not condition:
        raise RefusalEvidenceError(message)


def _nonempty_text(value: Any, maximum: int = 128) -> bool:
    return type(value) is str and 0 < len(value) <= maximum and all(ord(c) >= 32 for c in value)


@dataclass(frozen=True)
class Expectation:
    """An independently chosen test expectation, NOT decoded transaction proof."""

    request_id: str
    case: str
    method_name: str | None = None
    tx_receiver: str | None = None
    ak_receiver: str | None = None

    def __post_init__(self) -> None:
        _require(_nonempty_text(self.request_id), "request_id must be a nonempty string of at most 128 characters")
        _require(type(self.case) is str and self.case in {"direct_transfer", "wrong_method", "wrong_receiver", "attached_deposit"}, "unknown permission case")
        if self.case == "wrong_method":
            _require(_nonempty_text(self.method_name, 256), "wrong_method needs the independently expected method_name")
            _require(self.tx_receiver is None and self.ak_receiver is None, "irrelevant receiver fields")
        elif self.case == "wrong_receiver":
            _require(_nonempty_text(self.tx_receiver, 64) and _nonempty_text(self.ak_receiver, 64), "wrong_receiver needs both expected receiver IDs")
            _require(self.tx_receiver != self.ak_receiver, "receiver mismatch requires different receiver IDs")
            _require(self.method_name is None, "irrelevant method field")
        else:
            _require(self.method_name is None and self.tx_receiver is None and self.ak_receiver is None, "irrelevant expectation fields")

    def reason(self) -> str | dict[str, Any]:
        if self.case == "direct_transfer":
            return "RequiresFullAccess"
        if self.case == "attached_deposit":
            return "DepositWithFunctionCall"
        if self.case == "wrong_method":
            return {"MethodNameMismatch": {"method_name": self.method_name}}
        return {"ReceiverMismatch": {"tx_receiver": self.tx_receiver, "ak_receiver": self.ak_receiver}}


def _unique_object(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for key, value in pairs:
        _require(key not in result, "duplicate JSON object key")
        result[key] = value
    return result


def _reject_constant(value: str) -> None:
    raise RefusalEvidenceError("non-finite JSON number")


def _finite_float(value: str) -> float:
    number = float(value)
    _require(math.isfinite(number), "non-finite JSON number")
    return number


def _parse(raw: bytes, maximum: int) -> Any:
    _require(type(raw) is bytes, "original response bytes required")
    _require(0 < len(raw) <= maximum, "empty or oversized JSON input")
    try:
        return json.loads(raw.decode("utf-8"), object_pairs_hook=_unique_object, parse_constant=_reject_constant, parse_float=_finite_float)
    except (UnicodeError, ValueError, RecursionError) as error:
        raise RefusalEvidenceError("malformed, ambiguous, or excessively nested JSON") from error


def _object(value: Any, label: str) -> dict[str, Any]:
    _require(type(value) is dict, f"missing or malformed {label}")
    return value


def _variant(value: Any, key: str) -> Any:
    obj = _object(value, key)
    _require(set(obj) == {key}, f"expected exactly the {key} variant")
    return obj[key]


def classify(raw_response: bytes, expected: Expectation) -> dict[str, Any]:
    """Accept only the exact expected pre-inclusion access-key error structure.

    RPC correlation is checked against the independently supplied request ID.
    This does not bind the response to signed transaction bytes or authenticate
    its origin. Those checks belong in the live collector and execution harness.
    """
    _require(type(expected) is Expectation, "validated expectation required")
    envelope = _object(_parse(raw_response, MAX_RESPONSE_BYTES), "JSON-RPC envelope")
    _require(envelope.get("jsonrpc") == "2.0", "unexpected JSON-RPC version")
    _require(type(envelope.get("id")) is str and envelope["id"] == expected.request_id, "response request ID mismatch")
    _require("result" not in envelope, "result/pending/included execution is not a pre-inclusion refusal")
    error = _object(envelope.get("error"), "RPC error")
    _require(type(error.get("code")) is int and error["code"] == -32000, "unexpected RPC error code")
    _require(error.get("name") == "HANDLER_ERROR", "not a transaction handler error")
    cause = _object(error.get("cause"), "error cause")
    _require(cause.get("name") == "INVALID_TRANSACTION", "not a transaction-validation refusal")
    _require(type(cause.get("info")) is dict and not cause["info"], "unsupported or conflicting cause info")
    transaction_error = _variant(error.get("data"), "TxExecutionError")
    invalid_transaction = _variant(transaction_error, "InvalidTxError")
    reason = _variant(invalid_transaction, "InvalidAccessKeyError")
    _require(reason == expected.reason(), "access-key subtype or action details do not match the expectation")
    return {
        "classification": "EXPECTED_PERMISSION_ERROR_IN_RPC_RESPONSE",
        "request_id": expected.request_id,
        "case": expected.case,
        "observed_reason": reason,
        "response_sha256": hashlib.sha256(raw_response).hexdigest(),
        "live_execution_verified": False,
        "signed_transaction_verified": False,
        "cryptographic_proof": False,
        "included_receipt_id": None,
    }


def _read_bounded(path: Path, limit: int) -> bytes:
    with path.open("rb") as handle:
        raw = handle.read(limit + 1)
    _require(len(raw) <= limit, "input exceeds size limit")
    return raw


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--response", type=Path, required=True, help="original raw JSON-RPC response file")
    parser.add_argument("--expectation", type=Path, required=True, help="independently prepared expectation JSON")
    args = parser.parse_args(argv)
    try:
        raw_expected = _read_bounded(args.expectation, MAX_EXPECTATION_BYTES)
        values = _object(_parse(raw_expected, MAX_EXPECTATION_BYTES), "expectation")
        expected = Expectation(**values)
        result = classify(_read_bounded(args.response, MAX_RESPONSE_BYTES), expected)
        result["expectation_sha256"] = hashlib.sha256(raw_expected).hexdigest()
    except (OSError, TypeError, RefusalEvidenceError) as error:
        print(f"INCONCLUSIVE: {error}", file=sys.stderr)
        return 2
    print(json.dumps(result, sort_keys=True, separators=(",", ":")))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
