"""Signed, versioned handoff of one retirement incident to outside reviewers.

The trusted host exports exact stored evidence for one rejected action whose
subject this runtime retired: the action request bytes, source artifact, the
full hash-chained event log, an optional deterministic recheck, the restart
observation and an optional court proposal. A consumer must pin the signer's
public key out of band; the key named in the envelope only selects among keys
the consumer already trusts.

The handoff records facts. It does not stop an operating-system process,
revoke a wallet or blockchain key, or reverse an effect that already happened.
The restart observation is the producer's signed statement; a consumer cannot
re-run it. The jury electorate inside a court proposal is chosen by the
producer, so a consumer that needs independent jurors must pin their keys too.
"""
import json

from cryptography.exceptions import InvalidSignature
from cryptography.hazmat.primitives import serialization
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PublicKey

from agent_runtime import DOMAIN, PolicyViolation, canonical, digest
from agent_runtime_court import finding_payload, incident_packet

FORMAT = "x3.incident-handoff"
VERSION = 1
HANDOFF_DOMAIN = "X3/INCIDENT_HANDOFF/1"
MAX_ENVELOPE_BYTES = 4 * 1024 * 1024
PRODUCER = "x3lang/agent_runtime"
# Fixed for v1: the runtime refuses the subject's future actions, and nothing else.
EFFECTS = {
    "runtime_refuses_future_actions": True,
    "os_process_terminated": False,
    "blockchain_key_revoked": False,
    "external_effects_reversed": False,
}
ENVELOPE_FIELDS = {"format", "version", "signer", "payload", "signature"}
PAYLOAD_FIELDS = {"domain", "version", "issued_at", "producer", "incident", "request_hex",
                  "event_log", "retirement_event_seq", "recheck", "restart", "court", "effects"}
COURT_FIELDS = {"kind", "case_hash", "case", "findings", "electorate", "regression",
                "activation", "changes_authority"}


def recheck_rejection(pure_check, artifact: bytes) -> dict:
    """Re-run a side-effect-free host check, which must still reject the artifact.

    Never pass an adapter with external effects: this calls it directly.
    """
    try:
        pure_check(artifact)
    except PolicyViolation as exc:
        return {"rejected": True, "error_type": "PolicyViolation", "error": str(exc)}
    raise ValueError("recheck accepted the artifact; it is not a rejection regression")


def build_incident_payload(runtime, action_digest, *, issued_at, recheck=None, restart=None, court=None):
    """Collect evidence from the durable store; refuses anything it would not verify."""
    packet = incident_packet(runtime, action_digest)
    if packet["status"] not in ("denied", "failed"):
        raise ValueError("only rejected or failed actions can be handed off")
    if packet["subject"]["status"] != "retired":
        raise ValueError("subject is not retired")
    if court is not None and court.get("case_hash") != digest(canonical(packet)):
        raise ValueError("court proposal does not match incident")
    agent = packet["request"]["agent"]
    log = [{k: row[k] for k in ("seq", "payload", "previous", "hash")} for row in runtime.events()]
    matches = [row["seq"] for row in log if _event(row) == ("retired", agent, action_digest)]
    if len(matches) != 1:
        raise ValueError("retirement event missing")
    payload = {
        "domain": HANDOFF_DOMAIN, "version": VERSION, "issued_at": issued_at,
        "producer": {"system": PRODUCER, "runtime_domain": DOMAIN, "runtime_id": runtime.runtime_id},
        "incident": packet,
        "request_hex": bytes(runtime.evidence(action_digest)["request"]).hex(),
        "event_log": log, "retirement_event_seq": matches[0],
        "recheck": recheck, "restart": restart, "court": court, "effects": dict(EFFECTS),
    }
    _validate(json.loads(canonical(payload)))
    return payload


def sign_incident_handoff(payload, private_key) -> bytes:
    """Return canonical envelope bytes; the signature covers the exact payload text."""
    body = canonical(payload)
    _validate(json.loads(body))
    signer = private_key.public_key().public_bytes(serialization.Encoding.Raw, serialization.PublicFormat.Raw)
    envelope = canonical({"format": FORMAT, "version": VERSION, "signer": signer.hex(),
                          "payload": body.decode("ascii"), "signature": private_key.sign(body).hex()})
    if len(envelope) > MAX_ENVELOPE_BYTES:
        raise ValueError("envelope size exceeds limit")
    return envelope


def verify_incident_handoff(envelope: bytes, trusted_signers) -> dict:
    """Verify a pinned signature first, then every evidence binding. Returns the payload."""
    if not isinstance(envelope, bytes) or len(envelope) > MAX_ENVELOPE_BYTES:
        raise ValueError("envelope size exceeds limit")
    outer = _canonical_object(envelope, "envelope")
    if set(outer) != ENVELOPE_FIELDS:
        raise ValueError("unexpected envelope fields")
    if outer["format"] != FORMAT:
        raise ValueError("unsupported handoff format")
    if type(outer["version"]) is not int or outer["version"] != VERSION:
        raise ValueError("unsupported handoff version")
    trusted = set()
    for key in trusted_signers:
        Ed25519PublicKey.from_public_bytes(key)
        trusted.add(bytes(key).hex())
    if not isinstance(outer["signer"], str) or outer["signer"] not in trusted:
        raise ValueError("untrusted signer")
    if not isinstance(outer["payload"], str):
        raise ValueError("invalid payload")
    body = outer["payload"].encode("utf-8")
    _verify(outer["signer"], outer["signature"], body)
    payload = _canonical_object(body, "payload")
    try:
        _validate(payload)
    except (TypeError, AttributeError, KeyError) as exc:
        raise ValueError("malformed payload") from exc
    return payload


def _canonical_object(data, name):
    try:
        value = json.loads(data.decode("ascii"))
    except (UnicodeDecodeError, ValueError) as exc:
        raise ValueError(name + " is not canonical JSON") from exc
    if not isinstance(value, dict) or canonical(value) != data:
        raise ValueError(name + " is not canonical JSON")
    return value


def _verify(public_hex, signature_hex, message, error="invalid signature"):
    try:
        key = Ed25519PublicKey.from_public_bytes(bytes.fromhex(public_hex))
        signature = bytes.fromhex(signature_hex)
        if len(signature) != 64:
            raise ValueError
        key.verify(signature, message)
    except (InvalidSignature, ValueError, TypeError) as exc:
        raise ValueError(error) from exc


def _hex(value, name, size=None):
    if not isinstance(value, str) or (size is not None and len(value) != size * 2):
        raise ValueError("invalid " + name)
    try:
        return bytes.fromhex(value)
    except ValueError as exc:
        raise ValueError("invalid " + name) from exc


def _parsed(row):
    try:
        event = json.loads(row["payload"])
    except ValueError:
        return {}
    return event if isinstance(event, dict) else {}


def _event(row):
    event = _parsed(row)
    details = event.get("details")
    return event.get("kind"), event.get("agent"), details.get("action") if isinstance(details, dict) else None


def _validate(payload):
    if (not isinstance(payload, dict) or payload.get("domain") != HANDOFF_DOMAIN
            or type(payload.get("version")) is not int or payload.get("version") != VERSION):
        raise ValueError("unsupported payload domain or version")
    if set(payload) != PAYLOAD_FIELDS:
        raise ValueError("unexpected payload fields")
    if type(payload["issued_at"]) is not int or payload["issued_at"] < 1:
        raise ValueError("invalid issue time")
    producer = payload["producer"]
    if (not isinstance(producer, dict) or set(producer) != {"system", "runtime_domain", "runtime_id"}
            or producer["system"] != PRODUCER or producer["runtime_domain"] != DOMAIN):
        raise ValueError("unexpected producer")
    _hex(producer["runtime_id"], "runtime identity", 32)
    incident = _check_incident(payload, producer["runtime_id"])
    _check_log(payload, incident)
    _check_recheck(payload["recheck"])
    _check_restart(payload["restart"])
    if payload["court"] is not None:
        _check_court(payload["court"], incident)
    if payload["effects"] != EFFECTS:
        raise ValueError("effects claim mismatch")


def _check_incident(payload, runtime_id):
    incident = payload["incident"]
    if not isinstance(incident, dict) or set(incident) != {
            "action_digest", "request", "artifact_hex", "status", "subject", "output_hex", "kind"}:
        raise ValueError("invalid incident")
    if incident["kind"] != "local-incident-evidence" or incident["status"] not in ("denied", "failed"):
        raise ValueError("invalid incident")
    action = incident["action_digest"]
    _hex(action, "action digest", 32)
    request_bytes = _hex(payload["request_hex"], "request")
    request = incident["request"]
    if (digest(request_bytes) != action or not isinstance(request, dict)
            or json.loads(request_bytes) != request or canonical(request) != request_bytes):
        raise ValueError("request does not match action digest")
    if request.get("domain") != DOMAIN + "/ACTION" or request.get("runtime_id") != runtime_id:
        raise ValueError("runtime identity mismatch")
    if digest(_hex(incident["artifact_hex"], "artifact")) != request.get("artifact_sha256"):
        raise ValueError("artifact does not match request")
    _hex(incident["output_hex"], "output")
    subject = incident["subject"]
    if (not isinstance(subject, dict) or not isinstance(subject.get("manifest"), dict)
            or subject["manifest"].get("agent_id") != request.get("agent")):
        raise ValueError("subject does not match request")
    if subject.get("status") != "retired":
        raise ValueError("subject is not retired")
    _hex(subject.get("public_key"), "subject key", 32)
    return incident


def _check_log(payload, incident):
    log = payload["event_log"]
    if not isinstance(log, list) or not log:
        raise ValueError("invalid event log")
    previous = "0" * 64
    for index, row in enumerate(log, start=1):
        if not isinstance(row, dict) or set(row) != {"seq", "payload", "previous", "hash"}:
            raise ValueError("invalid event log")
        if type(row["seq"]) is not int or row["seq"] != index:
            raise ValueError("event log is not contiguous")
        if (not isinstance(row["payload"], str) or row["previous"] != previous
                or row["hash"] != digest(previous.encode() + row["payload"].encode())):
            raise ValueError("audit chain mismatch")
        previous = row["hash"]
    seq = payload["retirement_event_seq"]
    agent = incident["subject"]["manifest"]["agent_id"]
    if type(seq) is not int or not 1 <= seq <= len(log) or \
            _event(log[seq - 1]) != ("retired", agent, incident["action_digest"]):
        raise ValueError("retirement event mismatch")
    for row in log[:seq - 1]:
        event = _parsed(row)
        if (event.get("kind"), event.get("agent")) == ("born", agent):
            details = event.get("details")
            if not isinstance(details, dict) or details.get("key") != incident["subject"]["public_key"]:
                raise ValueError("subject birth key mismatch")
            return
    raise ValueError("subject birth record missing")


def _check_recheck(recheck):
    if recheck is None:
        return
    if (not isinstance(recheck, dict) or set(recheck) != {"rejected", "error_type", "error"}
            or recheck["rejected"] is not True or recheck["error_type"] != "PolicyViolation"
            or not isinstance(recheck["error"], str) or not 1 <= len(recheck["error"]) <= 512):
        raise ValueError("invalid recheck")


def _check_restart(restart):
    if restart is None:
        return
    if (not isinstance(restart, dict) or set(restart) != {"status_after_restart", "probe_action_refused", "probe_error"}
            or restart["status_after_restart"] != "retired" or type(restart["probe_action_refused"]) is not bool
            or not (restart["probe_error"] is None or isinstance(restart["probe_error"], str))):
        raise ValueError("invalid restart observation")


def _check_court(court, incident):
    if not isinstance(court, dict) or set(court) != COURT_FIELDS:
        raise ValueError("invalid court proposal")
    case_hash = digest(canonical(incident))
    if court["case_hash"] != case_hash or court["case"] != incident:
        raise ValueError("court proposal does not match incident")
    if (court["kind"] != "reviewed-regression-proposal" or court["changes_authority"] is not False
            or court["regression"] != {"source_hex": incident["artifact_hex"], "expected": "reject"}):
        raise ValueError("invalid court proposal")
    electorate = court["electorate"]
    if (not isinstance(electorate, dict) or len(electorate) < 3
            or not all(isinstance(key, str) and len(key) == 64 for key in electorate.values())
            or len(set(electorate.values())) != len(electorate)):
        raise ValueError("invalid court electorate")
    seen = set()
    violations = 0
    for finding in court["findings"] if isinstance(court["findings"], list) else [None]:
        if not isinstance(finding, dict) or set(finding) != {
                "domain", "case", "juror", "decision", "reason", "expires_at", "signature"}:
            raise ValueError("invalid court finding")
        juror = finding["juror"]
        if not isinstance(juror, str) or juror not in electorate or juror in seen or finding["case"] != case_hash:
            raise ValueError("ineligible or duplicate juror")
        message = finding_payload(case_hash, juror, finding["decision"], finding["reason"], finding["expires_at"])
        _verify(electorate[juror], finding["signature"], message, "invalid finding signature")
        seen.add(juror)
        violations += finding["decision"] == "violation"
    if violations <= len(electorate) // 2:
        raise ValueError("independent majority not established")
