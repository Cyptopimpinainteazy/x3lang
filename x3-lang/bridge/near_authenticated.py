#!/usr/bin/env python3
"""Authenticated NEAR testnet light-client state and inclusion checks.

The verifier starts only from a checkpoint approved by an offline trust policy.
RPC responses never become trusted through provider agreement. The resulting
state authenticates light-client headers and Merkle inclusion, not wallet
execution, durable contract storage, or settlement authorization.
"""
from __future__ import annotations

import argparse
import copy
import fcntl
import hashlib
import json
import os
import re
import sys
import tempfile
from dataclasses import asdict, dataclass
from functools import lru_cache
from pathlib import Path
from typing import Callable

from cryptography.exceptions import InvalidSignature
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PublicKey

from near_merkle import (
    check_inner_lite_encoding,
    decode_hash,
    encode_hash,
    lite_block_hash,
)
from near_outcome import (
    OutcomeProofError,
    verify_outcome_inclusion,
    verify_receipt_outcome,
)
from near_validator_commitment import compute_next_bps_hash


CHECKPOINT_SCHEMA = "x3.near-light-client-checkpoint.v1"
APPROVAL_SCHEMA = "x3.near-light-client-checkpoint-approval.v1"
TRUST_POLICY_SCHEMA = "x3.near-light-client-trust-policy.v1"
STATE_SCHEMA = "x3.near-light-client-state.v1"
CHECKPOINT_DOMAIN = b"X3_NEAR_LIGHT_CLIENT_CHECKPOINT_V1\x00"
NETWORKS = frozenset(("testnet", "mainnet"))
# Genesis hashes pinned from committed `status` RPC evidence:
#   testnet: evidence/near-proof/testnet/sha256/74/740117588c39...fc1b.json
#   mainnet: scripts/hackathon/fixtures/invoice06-mainnet-rpc.json
# A trust policy or checkpoint that pairs a network name with the other
# network's genesis (or any other genesis) is rejected before trust exists.
NETWORK_GENESIS = {
    "testnet": "FWJ9kR6KFWoyMoNjpLXXGHeuiy7tEY6GmoFeCA5yuc6b",
    "mainnet": "EPnLgE7iEq9s7yTkos96M3cWymH5avBAPm3qx3NXqR8H",
}
LIGHT_CLIENT_VIEW_FIELDS = frozenset(
    (
        "prev_block_hash",
        "next_block_inner_hash",
        "inner_lite",
        "inner_rest_hash",
        "next_bps",
        "approvals_after_next",
    )
)
BASE58_ALPHABET = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"
BASE58_RE = re.compile(r"[1-9A-HJ-NP-Za-km-z]+")
ACCOUNT_RE = re.compile(r"[a-z0-9_-]+(?:\.[a-z0-9_-]+)*\Z")
HEX64_RE = re.compile(r"[0-9a-f]{64}\Z")
U128_MAX = (1 << 128) - 1
U64_MAX = (1 << 64) - 1
ED25519_FIELD = (1 << 255) - 19
ED25519_D = (
    -121665 * pow(121666, ED25519_FIELD - 2, ED25519_FIELD)
) % ED25519_FIELD
ED25519_SQRT_M1 = pow(2, (ED25519_FIELD - 1) // 4, ED25519_FIELD)
ED25519_PRIME_ORDER = (
    2**252 + 27742317777372353535851937790883648493
)


class AuthenticatedLightClientError(ValueError):
    """The input cannot establish authenticated NEAR light-client state."""


def _canonical(value: object) -> bytes:
    return json.dumps(
        value, sort_keys=True, separators=(",", ":"), ensure_ascii=False, allow_nan=False
    ).encode("utf-8")


def _sha256(raw: bytes) -> str:
    return hashlib.sha256(raw).hexdigest()


def _without_checksum(document: dict, field: str) -> dict:
    value = copy.deepcopy(document)
    value.pop(field, None)
    return value


def _check_checksum(document: dict, field: str, label: str) -> str:
    supplied = document.get(field)
    if type(supplied) is not str or not HEX64_RE.fullmatch(supplied):
        raise AuthenticatedLightClientError(f"invalid {label} checksum")
    calculated = _sha256(_canonical(_without_checksum(document, field)))
    if calculated != supplied:
        raise AuthenticatedLightClientError(f"{label} checksum mismatch")
    return supplied


def _b58decode(value: str, *, label: str, length: int | None = None) -> bytes:
    if type(value) is not str:
        raise AuthenticatedLightClientError(f"invalid base58 {label}")
    # Bound Ed25519 fields before regex matching or large-integer decoding.
    maximum = {32: 44, 64: 88}.get(length)
    if maximum is not None and not length <= len(value) <= maximum:
        raise AuthenticatedLightClientError(f"{label} must decode to {length} bytes")
    if not BASE58_RE.fullmatch(value):
        raise AuthenticatedLightClientError(f"invalid base58 {label}")
    number = 0
    for character in value:
        number = number * 58 + BASE58_ALPHABET.index(character)
    zeros = len(value) - len(value.lstrip("1"))
    raw = b"\0" * zeros + (
        number.to_bytes((number.bit_length() + 7) // 8, "big") if number else b""
    )
    if length is not None and len(raw) != length:
        raise AuthenticatedLightClientError(f"{label} must decode to {length} bytes")
    if _b58encode(raw) != value:
        raise AuthenticatedLightClientError(f"non-canonical base58 {label}")
    return raw


def _b58encode(raw: bytes) -> str:
    number = int.from_bytes(raw, "big")
    symbols = []
    while number:
        number, remainder = divmod(number, 58)
        symbols.append(BASE58_ALPHABET[remainder])
    return "1" * (len(raw) - len(raw.lstrip(b"\0"))) + "".join(reversed(symbols))


def _key_bytes(value: object, label: str) -> bytes:
    if type(value) is not str or not value.startswith("ed25519:"):
        raise AuthenticatedLightClientError(f"{label} must be Ed25519")
    return _b58decode(value.removeprefix("ed25519:"), label=label, length=32)


def _signature_bytes(value: object, label: str) -> bytes:
    if type(value) is not str or not value.startswith("ed25519:"):
        raise AuthenticatedLightClientError(f"{label} must be Ed25519")
    return _b58decode(value.removeprefix("ed25519:"), label=label, length=64)


def _ed25519_public_point(public: bytes) -> tuple[int, int] | None:
    """Decode a canonical Ed25519 key into its affine Edwards point."""
    encoded = int.from_bytes(public, "little")
    sign_bit = encoded >> 255
    y = encoded & ((1 << 255) - 1)
    if y >= ED25519_FIELD:
        return None
    denominator = (ED25519_D * y * y + 1) % ED25519_FIELD
    if denominator == 0:
        return None
    x_squared = (y * y - 1) * pow(
        denominator, ED25519_FIELD - 2, ED25519_FIELD
    ) % ED25519_FIELD
    if x_squared == 0:
        return (0, y) if sign_bit == 0 else None
    x = pow(x_squared, (ED25519_FIELD + 3) // 8, ED25519_FIELD)
    if (x * x - x_squared) % ED25519_FIELD:
        x = x * ED25519_SQRT_M1 % ED25519_FIELD
    if (x * x - x_squared) % ED25519_FIELD:
        return None
    if (x & 1) != sign_bit:
        x = ED25519_FIELD - x
    return x, y


def _ed25519_point_add(
    left: tuple[int, int, int, int],
    right: tuple[int, int, int, int],
) -> tuple[int, int, int, int]:
    """Add extended Edwards points for a=-1, without secret-dependent branches."""
    x1, y1, z1, t1 = left
    x2, y2, z2, t2 = right
    a = (y1 - x1) * (y2 - x2) % ED25519_FIELD
    b = (y1 + x1) * (y2 + x2) % ED25519_FIELD
    c = 2 * ED25519_D * t1 * t2 % ED25519_FIELD
    d = 2 * z1 * z2 % ED25519_FIELD
    e = (b - a) % ED25519_FIELD
    f = (d - c) % ED25519_FIELD
    g = (d + c) % ED25519_FIELD
    h = (b + a) % ED25519_FIELD
    return e * f % ED25519_FIELD, g * h % ED25519_FIELD, f * g % ED25519_FIELD, e * h % ED25519_FIELD


def _ed25519_point_double(
    point: tuple[int, int, int, int],
) -> tuple[int, int, int, int]:
    x, y, z, _ = point
    a = x * x % ED25519_FIELD
    b = y * y % ED25519_FIELD
    c = 2 * z * z % ED25519_FIELD
    d = -a % ED25519_FIELD
    e = ((x + y) * (x + y) - a - b) % ED25519_FIELD
    g = (d + b) % ED25519_FIELD
    f = (g - c) % ED25519_FIELD
    h = (d - b) % ED25519_FIELD
    return e * f % ED25519_FIELD, g * h % ED25519_FIELD, f * g % ED25519_FIELD, e * h % ED25519_FIELD


def _ed25519_multiply(
    point: tuple[int, int, int, int], scalar: int
) -> tuple[int, int, int, int]:
    result = (0, 1, 1, 0)
    addend = point
    while scalar:
        if scalar & 1:
            result = _ed25519_point_add(result, addend)
        addend = _ed25519_point_double(addend)
        scalar >>= 1
    return result


@lru_cache(maxsize=8192)
def _valid_ed25519_public_key(public: bytes) -> bool:
    """Require a canonical, non-identity point in Ed25519's prime-order subgroup."""
    if type(public) is not bytes or len(public) != 32:
        return False
    point = _ed25519_public_point(public)
    if point is None:
        return False
    x, y = point
    if x == 0 and y == 1:
        return False
    extended = (x, y, 1, x * y % ED25519_FIELD)
    multiplied = _ed25519_multiply(extended, ED25519_PRIME_ORDER)
    return multiplied[0] == 0 and multiplied[1] == multiplied[2]


def _without_duplicate_keys(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise AuthenticatedLightClientError(f"duplicate JSON key: {key}")
        result[key] = value
    return result


def _reject_json_constant(value):
    raise AuthenticatedLightClientError(f"invalid JSON constant: {value}")


def _read_json(path: str | Path, limit: int = 16 * 1024 * 1024) -> dict:
    raw = Path(path).read_bytes()
    if len(raw) > limit:
        raise AuthenticatedLightClientError("oversized JSON document")
    try:
        value = json.loads(
            raw,
            object_pairs_hook=_without_duplicate_keys,
            parse_constant=_reject_json_constant,
        )
    except (UnicodeDecodeError, json.JSONDecodeError, RecursionError) as exc:
        raise AuthenticatedLightClientError("malformed JSON document") from exc
    if not isinstance(value, dict):
        raise AuthenticatedLightClientError("JSON document must be an object")
    return value


def _validate_validators(raw: object, label: str) -> list[dict]:
    if not isinstance(raw, list) or not 1 <= len(raw) <= 4096:
        raise AuthenticatedLightClientError(f"{label} must contain 1-4096 validators")
    required = {
        "account_id",
        "public_key",
        "stake",
        "validator_stake_struct_version",
    }
    accounts = set()
    total = 0
    validators = []
    for item in raw:
        if not isinstance(item, dict) or set(item) != required:
            raise AuthenticatedLightClientError(f"invalid {label} entry")
        if item.get("validator_stake_struct_version") != "V1":
            raise AuthenticatedLightClientError("unsupported validator stake version")
        account = item.get("account_id")
        if (
            type(account) is not str
            or len(account.encode("utf-8")) > 64
            or not ACCOUNT_RE.fullmatch(account)
            or account in accounts
        ):
            raise AuthenticatedLightClientError("invalid or duplicate validator account")
        accounts.add(account)
        key = _key_bytes(item.get("public_key"), "validator public key")
        if not _valid_ed25519_public_key(key):
            raise AuthenticatedLightClientError("malformed validator public key")
        # Distinct accounts may share a consensus key (real mainnet next_bps do);
        # each slot is counted separately, exactly as nearcore counts approvals.
        stake_raw = item.get("stake")
        if (
            type(stake_raw) is not str
            or not re.fullmatch(r"[1-9][0-9]*", stake_raw)
        ):
            raise AuthenticatedLightClientError("invalid validator stake")
        stake = int(stake_raw)
        if stake > U128_MAX:
            raise AuthenticatedLightClientError("validator stake exceeds u128")
        total += stake
        if total > U128_MAX:
            raise AuthenticatedLightClientError("validator stake total exceeds u128")
        validators.append(copy.deepcopy(item))
    if compute_next_bps_hash(validators) != compute_next_bps_hash(raw):
        raise AuthenticatedLightClientError("validator set changed during normalization")
    return validators


def _validate_view(raw: object) -> dict:
    if not isinstance(raw, dict):
        raise AuthenticatedLightClientError("invalid light-client view")
    if LIGHT_CLIENT_VIEW_FIELDS - set(raw):
        raise AuthenticatedLightClientError("missing light-client view field")
    if set(raw) - LIGHT_CLIENT_VIEW_FIELDS:
        raise AuthenticatedLightClientError("unauthenticated light-client view field")
    decode_hash(raw.get("prev_block_hash"))
    decode_hash(raw.get("next_block_inner_hash"))
    decode_hash(raw.get("inner_rest_hash"))
    inner = raw.get("inner_lite")
    if not isinstance(inner, dict):
        raise AuthenticatedLightClientError("missing light-client inner header")
    try:
        check_inner_lite_encoding(inner)
    except ValueError as exc:
        raise AuthenticatedLightClientError(str(exc)) from exc
    for field in (
        "epoch_id",
        "next_epoch_id",
        "prev_state_root",
        "outcome_root",
        "next_bp_hash",
        "block_merkle_root",
    ):
        decode_hash(inner.get(field))
    if inner["epoch_id"] == inner["next_epoch_id"]:
        raise AuthenticatedLightClientError("epoch and next epoch must differ")
    height = inner.get("height")
    timestamp = inner.get("timestamp")
    if type(height) is not int or not 0 < height <= U64_MAX:
        raise AuthenticatedLightClientError("invalid light-client height")
    if type(timestamp) is not int or not 0 < timestamp <= U64_MAX:
        raise AuthenticatedLightClientError("invalid light-client timestamp")
    next_bps = raw.get("next_bps")
    if next_bps is not None:
        next_bps = _validate_validators(next_bps, "next validator set")
        if compute_next_bps_hash(next_bps) != inner["next_bp_hash"]:
            raise AuthenticatedLightClientError("next validator commitment mismatch")
    approvals = raw.get("approvals_after_next")
    if not isinstance(approvals, list) or len(approvals) > 8192:
        raise AuthenticatedLightClientError("invalid approval vector")
    for signature in approvals:
        if signature is not None:
            _signature_bytes(signature, "validator approval")
    normalized = copy.deepcopy(raw)
    normalized["next_bps"] = next_bps
    return normalized


def _view_block_hash(view: dict) -> str:
    return encode_hash(lite_block_hash(view))


def _approval_message(next_block_hash: bytes, height: int) -> bytes:
    if len(next_block_hash) != 32 or not 0 <= height <= U64_MAX - 2:
        raise AuthenticatedLightClientError("invalid approval message input")
    return b"\x00" + next_block_hash + (height + 2).to_bytes(8, "little")


def _verify_approvals(
    validators: list[dict],
    approvals: list,
    message: bytes,
) -> tuple[int, int]:
    total_stake = 0
    approved_stake = 0
    for index, validator in enumerate(validators):
        stake = int(validator["stake"])
        total_stake += stake
        if total_stake > U128_MAX:
            raise AuthenticatedLightClientError("total validator stake overflow")
        signature = approvals[index] if index < len(approvals) else None
        if signature is None:
            continue
        public_key = Ed25519PublicKey.from_public_bytes(
            _strict_validator_key(validator["public_key"])
        )
        try:
            public_key.verify(
                _signature_bytes(signature, "validator approval"), message
            )
        except InvalidSignature as exc:
            raise AuthenticatedLightClientError("invalid validator approval") from exc
        approved_stake += stake
        if approved_stake > U128_MAX:
            raise AuthenticatedLightClientError("approved stake overflow")
    if approved_stake > U128_MAX // 3 or total_stake > U128_MAX // 2:
        raise AuthenticatedLightClientError("validator stake threshold arithmetic overflow")
    if approved_stake * 3 <= total_stake * 2:
        raise AuthenticatedLightClientError("validator approval does not exceed two-thirds stake")
    return approved_stake, total_stake


def _require_network_genesis(network: object, genesis_hash: object) -> None:
    if network not in NETWORK_GENESIS:
        raise AuthenticatedLightClientError("unsupported NEAR network")
    if genesis_hash != NETWORK_GENESIS[network]:
        raise AuthenticatedLightClientError(
            f"genesis hash is not the pinned NEAR {network} genesis"
        )


def _validate_trust_policy(policy: dict) -> dict:
    if not isinstance(policy, dict):
        raise AuthenticatedLightClientError("trust policy must be an object")
    if policy.get("schema") != TRUST_POLICY_SCHEMA:
        raise AuthenticatedLightClientError("wrong trust-policy schema")
    if policy.get("network") not in NETWORKS:
        raise AuthenticatedLightClientError("unsupported trust-policy network")
    decode_hash(policy.get("genesis_hash"))
    _require_network_genesis(policy["network"], policy["genesis_hash"])
    versions = policy.get("allowed_protocol_versions")
    if (
        not isinstance(versions, list)
        or not versions
        or len(versions) > 64
        or any(type(version) is not int or version <= 0 for version in versions)
        or len(set(versions)) != len(versions)
    ):
        raise AuthenticatedLightClientError("invalid allowed protocol versions")
    signers = policy.get("checkpoint_signers")
    if not isinstance(signers, list) or not 1 <= len(signers) <= 32:
        raise AuthenticatedLightClientError("invalid checkpoint signer list")
    ids = set()
    keys = set()
    for signer in signers:
        if not isinstance(signer, dict) or set(signer) != {"signer_id", "public_key"}:
            raise AuthenticatedLightClientError("invalid checkpoint signer")
        signer_id = signer.get("signer_id")
        if (
            type(signer_id) is not str
            or not re.fullmatch(r"[a-z0-9_.:-]{1,80}", signer_id)
            or signer_id in ids
        ):
            raise AuthenticatedLightClientError("invalid or duplicate signer ID")
        ids.add(signer_id)
        key = _key_bytes(signer.get("public_key"), "checkpoint signer key")
        if not _valid_ed25519_public_key(key):
            raise AuthenticatedLightClientError(
                "malformed or weak checkpoint signer public key"
            )
        if key in keys:
            raise AuthenticatedLightClientError("duplicate checkpoint signer key")
        keys.add(key)
    threshold = policy.get("threshold")
    if type(threshold) is not int or not 1 <= threshold <= len(signers):
        raise AuthenticatedLightClientError("invalid checkpoint signature threshold")
    return policy


def _strict_validator_key(value: object) -> bytes:
    public = _key_bytes(value, "validator public key")
    if not _valid_ed25519_public_key(public):
        raise AuthenticatedLightClientError(
            "malformed, weak, or torsion validator public key"
        )
    return public


def checkpoint_checksum(checkpoint: dict) -> str:
    if not isinstance(checkpoint, dict):
        raise AuthenticatedLightClientError("checkpoint must be an object")
    _check_checksum(checkpoint, "checksum", "checkpoint")
    return checkpoint["checksum"]


def seal_checkpoint_document(checkpoint: dict) -> dict:
    """Add the deterministic checksum without adding checkpoint trust."""
    if not isinstance(checkpoint, dict):
        raise AuthenticatedLightClientError("checkpoint must be an object")
    sealed = copy.deepcopy(checkpoint)
    sealed.pop("checksum", None)
    sealed["checksum"] = _sha256(_canonical(sealed))
    return sealed


def trust_policy_checksum(policy: dict) -> str:
    _validate_trust_policy(policy)
    return _sha256(_canonical(policy))


def checkpoint_signing_message(checksum: str) -> bytes:
    if type(checksum) is not str or not HEX64_RE.fullmatch(checksum):
        raise AuthenticatedLightClientError("invalid checkpoint checksum")
    return CHECKPOINT_DOMAIN + bytes.fromhex(checksum)


def _verify_checkpoint_approval(
    checkpoint_sha256: str,
    approval: dict,
    policy: dict,
) -> int:
    if approval.get("schema") != APPROVAL_SCHEMA:
        raise AuthenticatedLightClientError("wrong checkpoint approval schema")
    if approval.get("checkpoint_sha256") != checkpoint_sha256:
        raise AuthenticatedLightClientError("checkpoint approval targets another checksum")
    signatures = approval.get("signatures")
    if not isinstance(signatures, list) or not signatures:
        raise AuthenticatedLightClientError("checkpoint has no approval signatures")
    signer_keys = {
        signer["signer_id"]: signer["public_key"] for signer in policy["checkpoint_signers"]
    }
    verified = set()
    message = checkpoint_signing_message(checkpoint_sha256)
    for item in signatures:
        if not isinstance(item, dict) or set(item) != {"signer_id", "signature"}:
            raise AuthenticatedLightClientError("malformed checkpoint signature")
        signer_id = item.get("signer_id")
        if signer_id not in signer_keys or signer_id in verified:
            raise AuthenticatedLightClientError("unknown or duplicate checkpoint signer")
        public_key = Ed25519PublicKey.from_public_bytes(
            _key_bytes(signer_keys[signer_id], "checkpoint signer key")
        )
        try:
            public_key.verify(
                _signature_bytes(item.get("signature"), "checkpoint approval"),
                message,
            )
        except InvalidSignature as exc:
            raise AuthenticatedLightClientError("invalid checkpoint approval") from exc
        verified.add(signer_id)
    if len(verified) < policy["threshold"]:
        raise AuthenticatedLightClientError("checkpoint approval threshold not met")
    return len(verified)


def _checkpoint_to_state(
    checkpoint: dict,
    approval: dict,
    policy: dict,
    *,
    expected_policy_sha256: str,
    expected_checkpoint_sha256: str,
) -> dict:
    if not isinstance(checkpoint, dict) or not isinstance(approval, dict):
        raise AuthenticatedLightClientError("checkpoint and approval must be objects")
    policy = _validate_trust_policy(policy)
    if (
        type(expected_policy_sha256) is not str
        or not HEX64_RE.fullmatch(expected_policy_sha256)
        or _sha256(_canonical(policy)) != expected_policy_sha256
    ):
        raise AuthenticatedLightClientError("trust-policy file is not the pinned policy")
    checkpoint_sha = checkpoint_checksum(checkpoint)
    if checkpoint_sha != expected_checkpoint_sha256:
        raise AuthenticatedLightClientError("checkpoint is not the expected approval target")
    if checkpoint.get("schema") != CHECKPOINT_SCHEMA:
        raise AuthenticatedLightClientError("wrong checkpoint schema")
    if checkpoint.get("network") != policy["network"]:
        raise AuthenticatedLightClientError("checkpoint network does not match trust policy")
    if checkpoint.get("genesis_hash") != policy["genesis_hash"]:
        raise AuthenticatedLightClientError("checkpoint genesis does not match trust policy")
    protocol_version = checkpoint.get("protocol_version")
    if (
        type(protocol_version) is not int
        or protocol_version not in policy["allowed_protocol_versions"]
    ):
        raise AuthenticatedLightClientError("unsupported checkpoint protocol version")
    provenance = checkpoint.get("provenance")
    if not isinstance(provenance, dict):
        raise AuthenticatedLightClientError("missing checkpoint provenance")
    for field in ("source", "verified_by", "observed_at"):
        if type(provenance.get(field)) is not str or not provenance[field].strip():
            raise AuthenticatedLightClientError(f"missing checkpoint provenance {field}")
    _verify_checkpoint_approval(checkpoint_sha, approval, policy)

    view = _validate_view(checkpoint.get("light_client_block"))
    height = view["inner_lite"]["height"]
    if checkpoint.get("block_height") != height:
        raise AuthenticatedLightClientError("checkpoint height mismatch")
    if checkpoint.get("epoch_id") != view["inner_lite"]["epoch_id"]:
        raise AuthenticatedLightClientError("checkpoint epoch mismatch")
    if checkpoint.get("next_epoch_id") != view["inner_lite"]["next_epoch_id"]:
        raise AuthenticatedLightClientError("checkpoint next epoch mismatch")
    block_hash = _view_block_hash(view)
    if checkpoint.get("block_hash") != block_hash:
        raise AuthenticatedLightClientError("checkpoint block hash mismatch")
    # The checkpoint block itself does not commit to its own epoch's producers
    # (the *preceding* epoch's header does). A checkpoint may therefore omit
    # them; the verifier then refuses every same-epoch successor and only
    # accepts a successor in the next epoch, signed by the committed next_bps.
    if "current_validators" not in checkpoint:
        raise AuthenticatedLightClientError("checkpoint must state current validators or null")
    current_validators = checkpoint["current_validators"]
    current_hash = None
    if current_validators is not None:
        current_validators = _validate_validators(
            current_validators, "checkpoint validator set"
        )
        current_hash = compute_next_bps_hash(current_validators)
    commitments = checkpoint.get("validator_commitments")
    if (
        not isinstance(commitments, dict)
        or set(commitments) != {"current_validators_hash", "next_bp_hash"}
        or commitments.get("current_validators_hash") != current_hash
        or commitments.get("next_bp_hash") != view["inner_lite"]["next_bp_hash"]
    ):
        raise AuthenticatedLightClientError("checkpoint validator commitments mismatch")
    next_validators = view["next_bps"]
    if current_validators is None and next_validators is None:
        raise AuthenticatedLightClientError(
            "checkpoint authenticates no validator set for any successor"
        )
    if next_validators is not None:
        next_validators = _validate_validators(
            next_validators, "checkpoint next validator set"
        )
        if compute_next_bps_hash(next_validators) != view["inner_lite"]["next_bp_hash"]:
            raise AuthenticatedLightClientError("checkpoint next validator commitment mismatch")
    state = {
        "schema": STATE_SCHEMA,
        "kind": "authenticated_light_client_head",
        "network": checkpoint["network"],
        "genesis_hash": checkpoint["genesis_hash"],
        "protocol_version": protocol_version,
        "allowed_protocol_versions": copy.deepcopy(policy["allowed_protocol_versions"]),
        "checkpoint_sha256": checkpoint_sha,
        "trust_policy_sha256": expected_policy_sha256,
        "checkpoint_provenance": copy.deepcopy(provenance),
        "head": view,
        "head_hash": block_hash,
        "current_validators": current_validators,
        "current_validators_hash": current_hash,
        "next_validators": next_validators,
        "next_validators_hash": (
            compute_next_bps_hash(next_validators)
            if next_validators is not None
            else None
        ),
        "sequence": 0,
        "last_approved_stake": "0",
        "last_total_stake": "0",
    }
    state["state_sha256"] = _sha256(_canonical(_without_checksum(state, "state_sha256")))
    return _validate_state(state)


def _validate_state(state: dict) -> dict:
    if state.get("schema") != STATE_SCHEMA or state.get("kind") != "authenticated_light_client_head":
        raise AuthenticatedLightClientError("wrong authenticated-state schema")
    _check_checksum(state, "state_sha256", "authenticated state")
    if state.get("network") not in NETWORKS:
        raise AuthenticatedLightClientError("invalid authenticated-state network")
    decode_hash(state.get("genesis_hash"))
    _require_network_genesis(state["network"], state["genesis_hash"])
    versions = state.get("allowed_protocol_versions")
    if (
        not isinstance(versions, list)
        or type(state.get("protocol_version")) is not int
        or state.get("protocol_version") not in versions
    ):
        raise AuthenticatedLightClientError("unsupported authenticated-state protocol")
    checkpoint_sha = state.get("checkpoint_sha256")
    policy_sha = state.get("trust_policy_sha256")
    if (
        type(checkpoint_sha) is not str
        or not HEX64_RE.fullmatch(checkpoint_sha)
        or type(policy_sha) is not str
        or not HEX64_RE.fullmatch(policy_sha)
    ):
        raise AuthenticatedLightClientError("invalid authenticated-state provenance")
    view = _validate_view(state.get("head"))
    block_hash = _view_block_hash(view)
    if state.get("head_hash") != block_hash:
        raise AuthenticatedLightClientError("authenticated-state head hash mismatch")
    if "current_validators" not in state:
        raise AuthenticatedLightClientError("missing authenticated current validators")
    current = state["current_validators"]
    current_hash = None
    if current is not None:
        current = _validate_validators(current, "authenticated current validators")
        current_hash = compute_next_bps_hash(current)
    if state.get("current_validators_hash") != current_hash:
        raise AuthenticatedLightClientError("authenticated current validator commitment mismatch")
    next_validators = state.get("next_validators")
    if next_validators is not None:
        next_validators = _validate_validators(
            next_validators, "authenticated next validators"
        )
        next_hash = compute_next_bps_hash(next_validators)
        if state.get("next_validators_hash") != next_hash:
            raise AuthenticatedLightClientError("authenticated next validator commitment mismatch")
        if next_hash != view["inner_lite"]["next_bp_hash"]:
            raise AuthenticatedLightClientError("next validators do not match authenticated head")
    elif view["next_bps"] is not None or state.get("next_validators_hash") is not None:
        raise AuthenticatedLightClientError("missing authenticated next validators")
    if current is None and next_validators is None:
        raise AuthenticatedLightClientError("authenticated state has no usable validator set")
    sequence = state.get("sequence")
    if type(sequence) is not int or sequence < 0:
        raise AuthenticatedLightClientError("invalid authenticated-state sequence")
    for field in ("last_approved_stake", "last_total_stake"):
        if type(state.get(field)) is not str or not re.fullmatch(r"[0-9]+", state[field]):
            raise AuthenticatedLightClientError("invalid authenticated approval accounting")
    return state


def initialize_from_documents(
    checkpoint: dict,
    approval: dict,
    trust_policy: dict,
    *,
    expected_policy_sha256: str,
    expected_checkpoint_sha256: str,
) -> dict:
    """Create state only after external policy and checkpoint approvals verify."""
    return _checkpoint_to_state(
        checkpoint,
        approval,
        trust_policy,
        expected_policy_sha256=expected_policy_sha256,
        expected_checkpoint_sha256=expected_checkpoint_sha256,
    )


def _candidate_view(document: dict) -> tuple[dict, int | None]:
    if not isinstance(document, dict):
        raise AuthenticatedLightClientError("candidate document must be an object")
    if document.get("jsonrpc") == "2.0":
        if document.get("error") is not None:
            raise AuthenticatedLightClientError("candidate RPC response contains an error")
        result = document.get("result")
        protocol_version = document.get("protocol_version")
    else:
        result = document.get("result", document.get("light_client_block", document))
        protocol_version = document.get("protocol_version")
    if protocol_version is not None and type(protocol_version) is not int:
        raise AuthenticatedLightClientError("invalid candidate protocol metadata")
    return _validate_view(result), protocol_version


def build_checkpoint_candidate(
    candidate_document: dict,
    current_validators: list,
    *,
    network: str,
    genesis_hash: str,
    protocol_version: int,
    provenance: dict,
) -> dict:
    """Build an unsigned, unapproved checkpoint candidate for offline review."""
    if network not in NETWORKS:
        raise AuthenticatedLightClientError("unsupported checkpoint network")
    decode_hash(genesis_hash)
    _require_network_genesis(network, genesis_hash)
    view, _ = _candidate_view(candidate_document)
    validators = (
        None
        if current_validators is None
        else _validate_validators(current_validators, "checkpoint validator set")
    )
    if validators is None and view["next_bps"] is None:
        raise AuthenticatedLightClientError(
            "checkpoint authenticates no validator set for any successor"
        )
    return checkpoint_document(
        network=network,
        genesis_hash=genesis_hash,
        protocol_version=protocol_version,
        view=view,
        current_validators=validators,
        provenance=copy.deepcopy(provenance),
    )


def checkpoint_document(
    *,
    network: str,
    genesis_hash: str,
    protocol_version: int,
    view: dict,
    current_validators: list,
    provenance: dict,
) -> dict:
    """Create the deterministic checkpoint body before checksum and approval."""
    return {
        "schema": CHECKPOINT_SCHEMA,
        "network": network,
        "genesis_hash": genesis_hash,
        "protocol_version": protocol_version,
        "block_height": view["inner_lite"]["height"],
        "block_hash": _view_block_hash(view),
        "epoch_id": view["inner_lite"]["epoch_id"],
        "next_epoch_id": view["inner_lite"]["next_epoch_id"],
        "light_client_block": copy.deepcopy(view),
        "current_validators": copy.deepcopy(current_validators),
        "validator_commitments": {
            "current_validators_hash": (
                None
                if current_validators is None
                else compute_next_bps_hash(current_validators)
            ),
            "next_bp_hash": view["inner_lite"]["next_bp_hash"],
        },
        "provenance": copy.deepcopy(provenance),
    }


def _successor_validators(state: dict, candidate: dict) -> list[dict]:
    """Apply height/epoch succession rules and pick the approving validator set.

    Follows the Nomicon `validate_and_update_head` epoch rules and adds the
    nearcore header invariants a light client can check without the
    intermediate blocks: within one epoch `next_epoch_id` and `next_bp_hash`
    never change (nearcore chain.rs `InvalidNextBPHash`), and block timestamps
    strictly increase with height (`InvalidBlockPastTime`).
    """
    old = state["head"]["inner_lite"]
    new = candidate["inner_lite"]
    if new["height"] <= old["height"]:
        raise AuthenticatedLightClientError("candidate does not advance authenticated height")
    if new["timestamp"] <= old["timestamp"]:
        raise AuthenticatedLightClientError("candidate timestamp does not advance")
    if new["epoch_id"] == old["epoch_id"]:
        if new["next_epoch_id"] != old["next_epoch_id"]:
            raise AuthenticatedLightClientError(
                "same-epoch candidate commits a conflicting next epoch"
            )
        if new["next_bp_hash"] != old["next_bp_hash"]:
            raise AuthenticatedLightClientError(
                "same-epoch candidate commits a conflicting next validator set"
            )
        validators = state["current_validators"]
        if validators is None:
            raise AuthenticatedLightClientError(
                "validators of the checkpoint epoch were not authenticated"
            )
        return validators
    if new["epoch_id"] == old["next_epoch_id"]:
        if new["next_epoch_id"] == old["epoch_id"]:
            raise AuthenticatedLightClientError("candidate epoch transition loops back")
        validators = state["next_validators"]
        if validators is None:
            raise AuthenticatedLightClientError("candidate crosses into an unknown epoch")
        if candidate["next_bps"] is None:
            raise AuthenticatedLightClientError("epoch-crossing update omits next validator set")
        return validators
    raise AuthenticatedLightClientError("candidate epoch does not succeed authenticated head")


def advance_state(
    state: dict,
    candidate_document: dict,
    *,
    writer: Callable[[dict], None] | None = None,
) -> dict:
    """Verify one candidate update and atomically replace authenticated state."""
    state = _validate_state(copy.deepcopy(state))
    candidate, protocol_version = _candidate_view(candidate_document)
    candidate_network = candidate_document.get("network")
    if candidate_network is not None and candidate_network != state["network"]:
        raise AuthenticatedLightClientError("candidate network does not match authenticated state")
    candidate_genesis = candidate_document.get("genesis_hash")
    if candidate_genesis is not None and candidate_genesis != state["genesis_hash"]:
        raise AuthenticatedLightClientError("candidate genesis does not match authenticated state")
    if protocol_version is not None:
        if protocol_version not in state["allowed_protocol_versions"]:
            raise AuthenticatedLightClientError("candidate protocol version is unsupported")
        if protocol_version != state["protocol_version"]:
            raise AuthenticatedLightClientError("candidate changed protocol version without proof")
    validators = _successor_validators(state, candidate)

    candidate_hash = _view_block_hash(candidate)
    next_block_hash = hashlib.sha256(
        decode_hash(candidate["next_block_inner_hash"])
        + decode_hash(candidate_hash)
    ).digest()
    message = _approval_message(next_block_hash, candidate["inner_lite"]["height"])
    approved_stake, total_stake = _verify_approvals(
        validators,
        candidate["approvals_after_next"],
        message,
    )

    next_validators = (
        _validate_validators(candidate["next_bps"], "candidate next validator set")
        if candidate["next_bps"] is not None
        else state["next_validators"]
    )
    next_validators_hash = (
        compute_next_bps_hash(next_validators)
        if next_validators is not None
        else None
    )
    new_state = copy.deepcopy(state)
    new_state.update(
        {
            "head": candidate,
            "head_hash": candidate_hash,
            "current_validators": copy.deepcopy(validators),
            "current_validators_hash": compute_next_bps_hash(validators),
            "next_validators": next_validators,
            "next_validators_hash": next_validators_hash,
            "sequence": state["sequence"] + 1,
            "last_approved_stake": str(approved_stake),
            "last_total_stake": str(total_stake),
        }
    )
    new_state["state_sha256"] = _sha256(
        _canonical(_without_checksum(new_state, "state_sha256"))
    )
    new_state = _validate_state(new_state)
    if writer is not None:
        writer(new_state)
    return new_state


def _atomic_json_write(path: Path, document: dict) -> None:
    path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    descriptor, temporary = tempfile.mkstemp(prefix=f".{path.name}.", dir=path.parent)
    try:
        with os.fdopen(descriptor, "wb") as handle:
            handle.write(_canonical(document) + b"\n")
            handle.flush()
            os.fsync(handle.fileno())
        os.chmod(temporary, 0o600)
        os.replace(temporary, path)
        directory = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY)
        try:
            os.fsync(directory)
        finally:
            os.close(directory)
    except Exception:
        if os.path.exists(temporary):
            os.unlink(temporary)
        raise


class AuthenticatedHeadStore:
    """Crash-safe single-state store; unverified candidates never reach disk."""

    def __init__(self, path: str | Path):
        self.path = Path(path)
        self.lock_path = self.path.with_name(f".{self.path.name}.lock")

    def load(self) -> dict:
        return _validate_state(_read_json(self.path))

    def initialize(
        self,
        checkpoint: dict,
        approval: dict,
        trust_policy: dict,
        *,
        expected_policy_sha256: str,
        expected_checkpoint_sha256: str,
    ) -> dict:
        self.path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
        with self.lock_path.open("a+") as lock:
            os.chmod(self.lock_path, 0o600)
            fcntl.flock(lock, fcntl.LOCK_EX)
            if self.path.exists():
                raise AuthenticatedLightClientError("authenticated state already exists")
            state = initialize_from_documents(
                checkpoint,
                approval,
                trust_policy,
                expected_policy_sha256=expected_policy_sha256,
                expected_checkpoint_sha256=expected_checkpoint_sha256,
            )
            _atomic_json_write(self.path, state)
            return state

    def advance(self, candidate: dict, *, writer=None) -> dict:
        self.path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
        with self.lock_path.open("a+") as lock:
            os.chmod(self.lock_path, 0o600)
            fcntl.flock(lock, fcntl.LOCK_EX)
            state = self.load()
            return advance_state(
                state,
                candidate,
                writer=writer or (lambda value: _atomic_json_write(self.path, value)),
            )


@dataclass(frozen=True)
class AuthenticatedHead:
    network: str
    genesis_hash: str
    protocol_version: int
    block_hash: str
    height: int
    epoch_id: str
    next_epoch_id: str
    sequence: int
    state_sha256: str


@dataclass(frozen=True)
class AuthenticatedInclusion:
    receipt_id: str
    receiver_id: str
    block_hash: str
    authenticated_head: str
    shard_outcome_root: str
    block_outcome_root: str
    authenticated: bool
    authorized_to_sign: bool = False


def authenticated_head(state: dict) -> AuthenticatedHead:
    state = _validate_state(state)
    inner = state["head"]["inner_lite"]
    return AuthenticatedHead(
        network=state["network"],
        genesis_hash=state["genesis_hash"],
        protocol_version=state["protocol_version"],
        block_hash=state["head_hash"],
        height=inner["height"],
        epoch_id=inner["epoch_id"],
        next_epoch_id=inner["next_epoch_id"],
        sequence=state["sequence"],
        state_sha256=state["state_sha256"],
    )


def verify_authenticated_inclusion(
    state: dict,
    proof_response: dict,
    *,
    receipt_id: str,
    receiver_id: str,
) -> AuthenticatedInclusion:
    """Verify inclusion against the authenticated head, not an RPC head claim."""
    if not isinstance(proof_response, dict):
        raise AuthenticatedLightClientError("proof response must be an object")
    state = _validate_state(state)
    result = proof_response.get("result")
    if not isinstance(result, dict):
        raise AuthenticatedLightClientError("missing light-client proof result")
    root = decode_hash(state["head"]["inner_lite"]["block_merkle_root"])
    try:
        included = verify_receipt_outcome(
            proof_response,
            receipt_id=receipt_id,
            receiver_id=receiver_id,
            light_client_head=state["head_hash"],
            head_block_merkle_root=root,
        )
    except (OutcomeProofError, ValueError) as exc:
        raise AuthenticatedLightClientError("receipt is not included in authenticated head") from exc
    return AuthenticatedInclusion(
        receipt_id=included.receipt_id,
        receiver_id=included.receiver_id,
        block_hash=included.block_hash,
        authenticated_head=state["head_hash"],
        shard_outcome_root=included.shard_outcome_root,
        block_outcome_root=included.block_outcome_root,
        authenticated=True,
    )


@dataclass(frozen=True)
class AuthenticatedOutcome:
    outcome_id: str
    executor_id: str
    status: str
    block_hash: str
    authenticated_head: str
    shard_outcome_root: str
    block_outcome_root: str
    authenticated: bool
    authorized_to_sign: bool = False


def verify_authenticated_outcome(
    state: dict,
    proof_result: dict,
    *,
    outcome_id: str,
    executor_id: str,
) -> AuthenticatedOutcome:
    """Bind any execution outcome status to the authenticated head's block root.

    The returned status (`Unknown`, `Failure`, `SuccessValue`,
    `SuccessReceiptId`) is authenticated; a `Failure` carries no error detail
    because PartialExecutionStatus omits it. Not a settlement authorization.
    """
    state = _validate_state(state)
    root = decode_hash(state["head"]["inner_lite"]["block_merkle_root"])
    try:
        included = verify_outcome_inclusion(proof_result, head_block_merkle_root=root)
    except (OutcomeProofError, ValueError) as exc:
        raise AuthenticatedLightClientError("outcome is not included in authenticated head") from exc
    if included.outcome_id != outcome_id or included.executor_id != executor_id:
        raise AuthenticatedLightClientError("proof is for another outcome or executor")
    return AuthenticatedOutcome(
        outcome_id=included.outcome_id,
        executor_id=included.executor_id,
        status=included.status,
        block_hash=included.block_hash,
        authenticated_head=state["head_hash"],
        shard_outcome_root=included.shard_outcome_root,
        block_outcome_root=included.block_outcome_root,
        authenticated=True,
    )


def _print(value: object) -> None:
    if hasattr(value, "__dataclass_fields__"):
        value = asdict(value)
    print(json.dumps(value, indent=2, sort_keys=True))


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)

    seal = sub.add_parser("seal-checkpoint", help="add a deterministic checkpoint checksum")
    seal.add_argument("--input", required=True)
    seal.add_argument("--out", required=True)

    policy_digest = sub.add_parser(
        "trust-policy-digest", help="print the externally pinned trust-policy hash"
    )
    policy_digest.add_argument("--trust-policy", required=True)

    build = sub.add_parser(
        "build-checkpoint",
        help="build an unsigned checkpoint from a light-client response",
    )
    build.add_argument("--candidate", required=True)
    current_source = build.add_mutually_exclusive_group(required=True)
    current_source.add_argument("--current-validators")
    current_source.add_argument(
        "--no-current-validators",
        action="store_true",
        help="checkpoint epoch producers are not reviewed; only next-epoch successors verify",
    )
    build.add_argument("--network", choices=sorted(NETWORKS), required=True)
    build.add_argument("--genesis-hash", required=True)
    build.add_argument("--protocol-version", required=True, type=int)
    build.add_argument("--source", required=True)
    build.add_argument("--verified-by", required=True)
    build.add_argument("--observed-at", required=True)
    build.add_argument("--out", required=True)

    message = sub.add_parser("checkpoint-message", help="print the exact approval message")
    message.add_argument("--checkpoint", required=True)

    init = sub.add_parser("initialize", help="initialize from an externally approved checkpoint")
    init.add_argument("--checkpoint", required=True)
    init.add_argument("--approval", required=True)
    init.add_argument("--trust-policy", required=True)
    init.add_argument("--trust-policy-sha256", required=True)
    init.add_argument("--checkpoint-sha256", required=True)
    init.add_argument("--state", required=True)

    advance = sub.add_parser("advance", help="verify and persist one light-client update")
    advance.add_argument("--state", required=True)
    advance.add_argument("--candidate", required=True)

    head = sub.add_parser("head", help="print authenticated head metadata")
    head.add_argument("--state", required=True)

    inclusion = sub.add_parser("verify-inclusion", help="verify a receipt against authenticated state")
    inclusion.add_argument("--state", required=True)
    inclusion.add_argument("--proof", required=True)
    inclusion.add_argument("--receipt-id", required=True)
    inclusion.add_argument("--receiver-id", required=True)

    args = parser.parse_args(argv)
    try:
        if args.command == "build-checkpoint":
            if args.no_current_validators:
                validators = None
            else:
                validators_document = _read_json(args.current_validators)
                validators = validators_document.get("validators", validators_document)
            candidate = build_checkpoint_candidate(
                _read_json(args.candidate),
                validators,
                network=args.network,
                genesis_hash=args.genesis_hash,
                protocol_version=args.protocol_version,
                provenance={
                    "source": args.source,
                    "verified_by": args.verified_by,
                    "observed_at": args.observed_at,
                },
            )
            _atomic_json_write(Path(args.out), candidate)
            _print({"checkpoint_candidate": args.out, "approved": False})
            return 0
        if args.command == "seal-checkpoint":
            sealed = seal_checkpoint_document(_read_json(args.input))
            _atomic_json_write(Path(args.out), sealed)
            _print(
                {
                    "checkpoint": args.out,
                    "checkpoint_sha256": sealed["checksum"],
                }
            )
            return 0
        if args.command == "trust-policy-digest":
            trust_policy = _read_json(args.trust_policy)
            _print(
                {
                    "trust_policy_sha256": trust_policy_checksum(trust_policy)
                }
            )
            return 0
        if args.command == "checkpoint-message":
            checkpoint = _read_json(args.checkpoint)
            checksum = checkpoint_checksum(checkpoint)
            _print(
                {
                    "checkpoint_sha256": checksum,
                    "domain": CHECKPOINT_DOMAIN.rstrip(b"\0").decode("ascii"),
                    "signing_message_hex": checkpoint_signing_message(checksum).hex(),
                }
            )
            return 0
        if args.command == "initialize":
            store = AuthenticatedHeadStore(args.state)
            state = store.initialize(
                _read_json(args.checkpoint),
                _read_json(args.approval),
                _read_json(args.trust_policy),
                expected_policy_sha256=args.trust_policy_sha256,
                expected_checkpoint_sha256=args.checkpoint_sha256,
            )
            _print(authenticated_head(state))
            return 0
        if args.command == "advance":
            state = AuthenticatedHeadStore(args.state).advance(_read_json(args.candidate))
            _print(authenticated_head(state))
            return 0
        if args.command == "head":
            _print(authenticated_head(AuthenticatedHeadStore(args.state).load()))
            return 0
        if args.command == "verify-inclusion":
            state = AuthenticatedHeadStore(args.state).load()
            _print(
                verify_authenticated_inclusion(
                    state,
                    _read_json(args.proof),
                    receipt_id=args.receipt_id,
                    receiver_id=args.receiver_id,
                )
            )
            return 0
    except (AuthenticatedLightClientError, ValueError, OSError) as exc:
        print(f"authenticated light-client error: {exc}", file=sys.stderr)
        return 2
    raise AssertionError("unreachable")


if __name__ == "__main__":
    sys.exit(main())
