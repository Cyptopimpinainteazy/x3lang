"""Read-only incident court and reviewable regression proposals.

Court verdicts cannot restore agents, change commandments, or grant authority.
Jurors are authenticated by host-selected keys; this local v1 is not anonymous
commit-reveal voting and does not claim to implement the full historical court.
"""
from dataclasses import dataclass
import json
from cryptography.exceptions import InvalidSignature
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PublicKey
from agent_runtime import canonical, digest


@dataclass(frozen=True)
class Finding:
    juror_id: str
    decision: str
    reason: str
    expires_at: int
    signature: bytes


def finding_payload(case_hash, juror_id, decision, reason, expires_at):
    if decision not in ("violation", "inconclusive"):
        raise ValueError("invalid decision")
    if not isinstance(reason, str) or not 1 <= len(reason) <= 2048:
        raise ValueError("invalid reason")
    if type(expires_at) is not int:
        raise ValueError("invalid expiry")
    return canonical({"domain": "X3/INCIDENT_COURT/1", "case": case_hash,
                      "juror": juror_id, "decision": decision, "reason": reason,
                      "expires_at": expires_at})


def incident_packet(runtime, action_digest):
    """Freeze exact stored source, request and outcome for independent review."""
    evidence = runtime.evidence(action_digest)
    request = bytes(evidence["request"])
    artifact = bytes(evidence["artifact"])
    parsed = json.loads(request)
    if digest(request) != action_digest or digest(artifact) != parsed["artifact_sha256"]:
        raise ValueError("incident evidence mismatch")
    subject = runtime.snapshot(evidence["agent"])
    packet = {"action_digest": action_digest, "request": json.loads(request),
              "artifact_hex": artifact.hex(), "status": evidence["status"],
              "subject": subject, "output_hex": bytes(evidence["output"] or b"").hex(),
              "kind": "local-incident-evidence"}
    return packet


class IncidentCourt:
    """Trusted host creates court; reviewers only return signed findings."""

    def __init__(self, juror_keys):
        self.keys = dict(juror_keys)
        if len(self.keys) < 3 or len(set(self.keys.values())) != len(self.keys):
            raise ValueError("three independent juror keys required")
        for identity, key in self.keys.items():
            if not isinstance(identity, str) or not identity or not isinstance(key, bytes) or len(key) != 32:
                raise ValueError("invalid juror")
            Ed25519PublicKey.from_public_bytes(key)

    def review(self, runtime, action_digest, findings, *, now):
        packet = incident_packet(runtime, action_digest)
        if packet["status"] not in ("denied", "failed"):
            raise ValueError("only rejected or failed actions can become rejection regressions")
        excluded_ids, excluded_keys = runtime.review_exclusions(packet["request"]["agent"])
        eligible = {i:k for i,k in self.keys.items() if i not in excluded_ids and k not in excluded_keys}
        if len(eligible) < 3:
            raise ValueError("insufficient independent electorate")
        case_hash = digest(canonical(packet))
        seen = set()
        violations = 0
        recorded = []
        for f in findings:
            if f.juror_id not in eligible or f.juror_id in seen:
                raise ValueError("ineligible or duplicate juror")
            if f.expires_at <= now:
                raise ValueError("expired finding")
            payload = finding_payload(case_hash, f.juror_id, f.decision, f.reason, f.expires_at)
            try:
                Ed25519PublicKey.from_public_bytes(eligible[f.juror_id]).verify(f.signature, payload)
            except InvalidSignature as exc:
                raise ValueError("invalid finding signature") from exc
            seen.add(f.juror_id)
            violations += f.decision == "violation"
            recorded.append({**json.loads(payload), "signature": f.signature.hex()})
        if violations <= len(eligible)//2:
            raise ValueError("independent majority not established")
        return {"kind": "reviewed-regression-proposal", "case_hash": case_hash,
                "case": packet, "findings": recorded,
                "electorate": {i:k.hex() for i,k in eligible.items()},
                "regression": {"source_hex": packet["artifact_hex"], "expected": "reject"},
                "activation": "requires owner review and passing regression tests",
                "changes_authority": False}
