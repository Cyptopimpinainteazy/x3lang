"""Ed25519-authorized child-admission preflight, NOT persistent admission.

The trusted signer must be selected by the runtime using the parent identity;
the candidate must never choose its own trusted public key.
"""
from dataclasses import dataclass
import json
from typing import FrozenSet

from cryptography.exceptions import InvalidSignature
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PublicKey

from agent_constitution import AgentManifest, AdmissionDecision, check_admission

DOMAIN = "X3/CHILD_ADMISSION/1"
MAX_APPROVAL_BYTES = 8192


@dataclass(frozen=True)
class SignedApproval:
    nonce: str
    expires_at: int
    signature: bytes


def _manifest(m: AgentManifest) -> dict:
    return {
        "agent_id": m.agent_id,
        "parent_id": m.parent_id,
        "capabilities": sorted(m.capabilities),
        "surfaces": sorted(m.surfaces),
        "budget": m.budget,
        "expires_at": m.expires_at,
        "lineage": list(m.lineage),
        "spawn_limit": m.spawn_limit,
    }


def approval_payload(child: AgentManifest, parent: AgentManifest, *, nonce: str, expires_at: int) -> bytes:
    """Canonical bytes binding exact child and parent manifests to approval.

    The runtime must require a never-before-consumed nonce during atomic admission.
    """
    if not isinstance(nonce, str) or not (1 <= len(nonce) <= 128):
        raise ValueError("invalid nonce")
    if type(expires_at) is not int or expires_at < 0:
        raise ValueError("invalid expiry")
    raw = json.dumps(
        {"domain": DOMAIN, "nonce": nonce, "approval_expires_at": expires_at,
         "child": _manifest(child), "parent": _manifest(parent)},
        sort_keys=True, separators=(",", ":"), ensure_ascii=True, allow_nan=False,
    ).encode("ascii")
    if len(raw) > MAX_APPROVAL_BYTES:
        raise ValueError("approval payload too large")
    return raw


def verify_signed_admission(
    candidate: AgentManifest, parent: AgentManifest, approval: SignedApproval, *,
    trusted_public_key: Ed25519PublicKey, now: int, existing_ids: FrozenSet[str],
    parent_budget_available: int, parent_children: int,
) -> AdmissionDecision:
    """Preflight only. Caller must atomically persist identity, budget and nonce.

    Public key MUST come from a trusted parent/operator registry, never from
    agent-submitted contents. A valid signature grants no tool access.
    """
    failures = []
    if type(approval.expires_at) is not int or approval.expires_at <= now:
        failures.append("approval_expired")
    if approval.expires_at > candidate.expires_at:
        failures.append("approval_outlives_child")
    try:
        if not isinstance(trusted_public_key, Ed25519PublicKey):
            raise ValueError("untrusted verifier type")
        if not isinstance(approval.signature, bytes) or len(approval.signature) != 64:
            raise ValueError("invalid signature size")
        payload = approval_payload(candidate, parent, nonce=approval.nonce, expires_at=approval.expires_at)
        trusted_public_key.verify(approval.signature, payload)
    except (InvalidSignature, ValueError, TypeError, OverflowError, AttributeError):
        failures.append("signature")
    policy = check_admission(
        candidate, parent, now=now, existing_ids=existing_ids,
        parent_budget_available=parent_budget_available,
        parent_children=parent_children, approval_verified=not failures,
    )
    failures.extend(policy.reasons)
    return AdmissionDecision(not failures, tuple(dict.fromkeys(failures)))
