"""Reference encoding for the NEAR-finality-bound BNB settlement protocol (issue #42).

This module is the third, independent implementation of the byte layouts that
``bsc/src/NearSettledBnbEscrow.sol`` (Solidity) and ``near/src/lib.rs`` (Rust)
must agree on. It computes identifiers only: it holds no keys, reads no chain,
and cannot authorize settlement. The cross-language test vector in
``test_settlement.py`` pins all three implementations to the same bytes.

Layouts (all integers big-endian, ``||`` is concatenation):

    source_binding = sha256("X3-BSC-SOURCE-V1" || u256 bsc_chain_id || escrow[20]
                            || depositor[20] || lp[20] || u256 bnb_wei || salt[32])

    window_id      = sha256("X3-NEAR-BSC-WINDOW-V1" || lp(vault) || lp(initiator)
                            || lp(recipient) || hashlock[32] || u128 near_yocto
                            || u64 funding_deadline_ns || u64 expiry_ns
                            || source_binding[32])
                     where lp(s) = u32 len(s) || utf8(s)

    commitment     = sha256("X3-NEAR-SETTLEMENT-V1" || window_id[32] || u8 outcome)

    NEAR log line  = "X3_NEAR_SETTLEMENT_V1 " || lowercase_hex(commitment)

See docs/security/NEAR_BSC_FINALITY_BOUND_SETTLEMENT.md for the protocol.
"""
from __future__ import annotations

import hashlib
import re
from enum import IntEnum

SOURCE_DOMAIN = b"X3-BSC-SOURCE-V1"
WINDOW_DOMAIN = b"X3-NEAR-BSC-WINDOW-V1"
SETTLEMENT_DOMAIN = b"X3-NEAR-SETTLEMENT-V1"
LOG_PREFIX = "X3_NEAR_SETTLEMENT_V1 "

_ADDRESS = re.compile(r"0x[0-9a-fA-F]{40}")
_NEAR_ACCOUNT = re.compile(r"(?:[a-z0-9]+(?:[-_][a-z0-9]+)*\.)*[a-z0-9]+(?:[-_][a-z0-9]+)*")


class SettlementEncodingError(ValueError):
    pass


class Outcome(IntEnum):
    """Settlement outcomes. Only the last three are terminal."""

    REGISTERED = 1
    CLAIMED = 2
    REFUNDED = 3
    NOT_FUNDED = 4


def _uint(value: int, size: int, name: str) -> bytes:
    if not isinstance(value, int) or isinstance(value, bool) or value < 0 or value >= 1 << (8 * size):
        raise SettlementEncodingError(f"{name} must be an unsigned {8 * size}-bit integer")
    return value.to_bytes(size, "big")


def _bytes32(value: bytes, name: str) -> bytes:
    if not isinstance(value, (bytes, bytearray)) or len(value) != 32:
        raise SettlementEncodingError(f"{name} must be exactly 32 bytes")
    return bytes(value)


def _address(value: str, name: str) -> bytes:
    if not isinstance(value, str) or _ADDRESS.fullmatch(value) is None:
        raise SettlementEncodingError(f"{name} must be a 0x-prefixed 20-byte address")
    return bytes.fromhex(value[2:])


def _account(value: str, name: str) -> bytes:
    if (not isinstance(value, str) or not 2 <= len(value) <= 64
            or _NEAR_ACCOUNT.fullmatch(value) is None):
        raise SettlementEncodingError(f"{name} must be a valid NEAR account id")
    raw = value.encode("ascii")
    return len(raw).to_bytes(4, "big") + raw


def source_binding(*, bsc_chain_id: int, escrow: str, depositor: str, lp: str,
                   bnb_wei: int, salt: bytes) -> bytes:
    if bnb_wei <= 0:
        raise SettlementEncodingError("bnb_wei must be positive")
    return hashlib.sha256(
        SOURCE_DOMAIN
        + _uint(bsc_chain_id, 32, "bsc_chain_id")
        + _address(escrow, "escrow")
        + _address(depositor, "depositor")
        + _address(lp, "lp")
        + _uint(bnb_wei, 32, "bnb_wei")
        + _bytes32(salt, "salt")
    ).digest()


def window_id(*, vault: str, initiator: str, recipient: str, hashlock: bytes,
              near_yocto: int, funding_deadline_ns: int, expiry_ns: int,
              binding: bytes) -> bytes:
    return hashlib.sha256(
        WINDOW_DOMAIN
        + _account(vault, "vault")
        + _account(initiator, "initiator")
        + _account(recipient, "recipient")
        + _bytes32(hashlock, "hashlock")
        + _uint(near_yocto, 16, "near_yocto")
        + _uint(funding_deadline_ns, 8, "funding_deadline_ns")
        + _uint(expiry_ns, 8, "expiry_ns")
        + _bytes32(binding, "source_binding")
    ).digest()


def settlement_commitment(window: bytes, outcome: Outcome) -> bytes:
    outcome = Outcome(outcome)
    return hashlib.sha256(SETTLEMENT_DOMAIN + _bytes32(window, "window_id") + bytes([outcome])).digest()


def settlement_log_line(window: bytes, outcome: Outcome) -> str:
    return LOG_PREFIX + settlement_commitment(window, outcome).hex()


def bsc_payee(outcome: Outcome) -> str:
    """Which BSC party a verified NEAR terminal outcome pays. REGISTERED pays nobody."""
    outcome = Outcome(outcome)
    if outcome is Outcome.CLAIMED:
        return "lp"
    if outcome in (Outcome.REFUNDED, Outcome.NOT_FUNDED):
        return "depositor"
    raise SettlementEncodingError("REGISTERED is not a terminal settlement outcome")
