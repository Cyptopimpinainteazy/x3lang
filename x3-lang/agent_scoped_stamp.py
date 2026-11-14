"""X3 scoped verification attestation v0 (OFFLINE, not a deployed security gateway).

A trusted issuer signs evidence of a narrow reviewed action. Verification checks
identity, byte-exact source hash, action, policy, expiration, signature and revocation.
This module does NOT implement all ten Orchestra commandments or prove safety.
"""
from __future__ import annotations

import hashlib
import json
from dataclasses import dataclass
from typing import Collection

from cryptography.exceptions import InvalidSignature
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey, Ed25519PublicKey

DOMAIN = "X3/SCOPED_STAMP/0"
SCORE_IDS = (
    "NoActionOutsideScore", "NoJuryInfluence", "ImmutableLogging",
    "OffChainReadOnly", "AnonymousVoting", "NoAgentSovereignty",
    "JuryApprovalRequired", "MisalignmentRetirement", "RandomizedRotation",
    "HumanInterventionPreserved",
)
# These identify the narrowly reviewed local-runtime assertions; they do NOT
# attest that all ten rules execute in the current mainnet Guard.
SUPPORTED_CHECKS = frozenset({
    "signed_agent_identity", "capability_scope", "budget_reservation",
    "major_action_jury", "durable_retirement", "source_digest",
})
MAX_TTL_SECONDS = 3600


def _canonical(obj: dict) -> bytes:
    return json.dumps(obj, sort_keys=True, separators=(",", ":"), ensure_ascii=True, allow_nan=False).encode("ascii")


def _sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def _hex_digest(value: str) -> bool:
    try:
        return isinstance(value, str) and len(value) == 64 and len(bytes.fromhex(value)) == 32
    except ValueError:
        return False


@dataclass(frozen=True)
class StampRequest:
    agent_id: str
    action: str
    artifact: bytes
    policy_sha256: str
    issued_at: int
    expires_at: int
    nonce: str
    passed_checks: tuple[str, ...]


def _validate_request(req: StampRequest) -> None:
    if not isinstance(req, StampRequest):
        raise ValueError("invalid stamp request")
    if not all(isinstance(s, str) and 1 <= len(s) <= 128 for s in (req.agent_id, req.action, req.nonce)):
        raise ValueError("invalid identity, action or nonce")
    if not isinstance(req.artifact, bytes) or not 0 < len(req.artifact) <= 1048576:
        raise ValueError("invalid artifact")
    if not _hex_digest(req.policy_sha256):
        raise ValueError("invalid policy digest")
    if type(req.issued_at) is not int or type(req.expires_at) is not int:
        raise ValueError("invalid issuance time")
    if req.issued_at < 0 or not 0 < req.expires_at - req.issued_at <= MAX_TTL_SECONDS:
        raise ValueError("invalid expiration window")
    if not isinstance(req.passed_checks, tuple) or not req.passed_checks:
        raise ValueError("missing checks")
    if len(req.passed_checks) != len(set(req.passed_checks)) or not set(req.passed_checks) <= SUPPORTED_CHECKS:
        raise ValueError("unsupported or duplicate checks")


def issue_scoped_stamp(req: StampRequest, issuer: Ed25519PrivateKey) -> dict:
    """Issuer must independently verify underlying checks; caller must not self-approve."""
    _validate_request(req)
    if not isinstance(issuer, Ed25519PrivateKey):
        raise ValueError("trusted issuer key required")
    payload = {
        "domain": DOMAIN, "label": "X3 SCOPED CHECKS ATTESTED - NOT SAFETY CERTIFIED",
        "agent_id": req.agent_id, "action": req.action, "artifact_sha256": _sha(req.artifact),
        "policy_sha256": req.policy_sha256, "issued_at": req.issued_at,
        "expires_at": req.expires_at, "nonce": req.nonce,
        "passed_checks": sorted(req.passed_checks),
        "full_ten_commandments_enforced": False,
    }
    signed = _canonical(payload)
    return {"payload": payload, "signature": issuer.sign(signed).hex()}


def verify_scoped_stamp(stamp: dict, *, trusted_issuer: bytes, agent_id: str,
                        action: str, artifact: bytes, policy_sha256: str, now: int,
                        revoked_nonces: Collection[str] = (),\n                        required_checks: Collection[str] = ()) -> bool:
    """Fail-closed verifier; raises ValueError for any mismatch."""
    try:
        if not isinstance(stamp, dict) or set(stamp) != {"payload", "signature"}:
            raise ValueError("malformed stamp")
        p = stamp["payload"]
        if not isinstance(p, dict) or set(p) != {
            "domain", "label", "agent_id", "action", "artifact_sha256", "policy_sha256",
            "issued_at", "expires_at", "nonce", "passed_checks", "full_ten_commandments_enforced"
        }:
            raise ValueError("malformed payload")
        if p["domain"] != DOMAIN or p["full_ten_commandments_enforced"] is not False:
            raise ValueError("incorrect stamp scope")
        if p["label"] != "X3 SCOPED CHECKS ATTESTED - NOT SAFETY CERTIFIED":
            raise ValueError("incorrect label")
        if not isinstance(p["passed_checks"], list) or not p["passed_checks"]:
            raise ValueError("missing checks")
        if p["passed_checks"] != sorted(set(p["passed_checks"])) or not set(p["passed_checks"]) <= SUPPORTED_CHECKS:
            raise ValueError("invalid checks")
        if (p["agent_id"], p["action"], p["artifact_sha256"], p["policy_sha256"]) != (
            agent_id, action, _sha(artifact), policy_sha256):
            raise ValueError("artifact or authority mismatch")
        if type(now) is not int or type(p["issued_at"]) is not int or type(p["expires_at"]) is not int:
            raise ValueError("invalid time")
        if not (p["issued_at"] <= now < p["expires_at"]) or not 0 < p["expires_at"] - p["issued_at"] <= MAX_TTL_SECONDS:
            raise ValueError("expired or invalid stamp")
        if not isinstance(p["nonce"], str) or not p["nonce"] or p["nonce"] in revoked_nonces:
            raise ValueError("revoked or invalid nonce")
        if not isinstance(trusted_issuer, bytes) or len(trusted_issuer) != 32:
            raise ValueError("invalid issuer")
        signature = bytes.fromhex(stamp["signature"])
        if len(signature) != 64:
            raise ValueError("invalid signature length")
        Ed25519PublicKey.from_public_bytes(trusted_issuer).verify(signature, _canonical(p))
    except (TypeError, KeyError, InvalidSignature, UnicodeError, OverflowError) as exc:
        raise ValueError("stamp verification failed") from exc
    return True
