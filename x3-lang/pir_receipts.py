"""Signed PIR evidence envelopes. A signature authenticates a trusted issuer,
not the truth of an arbitrary chain observation. No execution authorization.
"""

import json
import re
import sqlite3
from pathlib import Path

from agent_runtime import (
    DOMAIN as RUNTIME_DOMAIN,
)
from agent_runtime import (
    MAX_ARTIFACT_BYTES,
    Runtime,
    canonical,
    digest,
)
from cryptography.exceptions import InvalidSignature
from cryptography.hazmat.primitives import serialization
from cryptography.hazmat.primitives.asymmetric.ed25519 import (
    Ed25519PrivateKey,
    Ed25519PublicKey,
)
from pir_agent_gate import decode_signed

DOMAIN = b"X3/PIR/RECEIPT/1\x00"
SCHEMA = "x3.pir.receipt.v1"
MAX_EVIDENCE = 65536
MAX_RECEIPT = 8192
IDENT = re.compile(r"[A-Za-z0-9][A-Za-z0-9:._/-]{0,255}", re.ASCII)
CONTEXT_FIELDS = frozenset(
    (
        "profile",
        "status",
        "scope",
        "context",
        "subject",
        "source_sha256",
        "policy_sha256",
        "native_id",
        "artifact_sha256",
    )
)
CLAIMS = CONTEXT_FIELDS | {"result_sha256"}
PAYLOAD_FIELDS = CLAIMS | {"schema", "issuer", "evidence_sha256"}
STATUSES = {
    "local": frozenset(("completed", "pending", "failed")),
    "evm": frozenset(("observed_unverified",)),
    "settlement": frozenset(("blocked",)),
}


def _hex(value, size=32):
    if (
        type(value) is not str
        or len(value) != size * 2
        or not re.fullmatch(r"[0-9a-f]+", value)
    ):
        raise ValueError("receipt: invalid hex")
    return bytes.fromhex(value)


def _key_bytes(key):
    if not isinstance(key, Ed25519PrivateKey):
        raise TypeError("receipt: issuer private key required")
    return key.public_key().public_bytes(
        serialization.Encoding.Raw, serialization.PublicFormat.Raw
    )


def _claims(c):
    if type(c) is not dict or set(c) != CLAIMS:
        raise ValueError("receipt: unexpected claim fields")
    profile, status = c["profile"], c["status"]
    if (
        type(profile) is not str
        or type(status) is not str
        or status not in STATUSES.get(profile, ())
    ):
        raise ValueError("receipt: unsupported profile/status")
    for name in ("scope", "context", "subject"):
        if type(c[name]) is not str or not IDENT.fullmatch(c[name]):
            raise ValueError("receipt: invalid context")
    for name in ("source_sha256", "policy_sha256", "native_id", "artifact_sha256"):
        _hex(c[name])
    if profile == "local" and status == "completed":
        _hex(c["result_sha256"])
    elif c["result_sha256"] is not None:
        raise ValueError("receipt: noncompletion cannot have result")
    if profile == "local":
        if not c["context"].startswith("runtime:"):
            raise ValueError("receipt: runtime context required")
        _hex(c["context"][8:])


def _evidence(evidence):
    if type(evidence) is not bytes or not 0 < len(evidence) <= MAX_EVIDENCE:
        raise ValueError("receipt: invalid evidence size")
    return digest(evidence)


def _sign(claims, evidence, key):
    _claims(claims)
    payload = {
        **claims,
        "schema": SCHEMA,
        "issuer": _key_bytes(key).hex(),
        "evidence_sha256": _evidence(evidence),
    }
    signature = key.sign(DOMAIN + canonical(payload)).hex()
    return canonical({"payload": payload, "signature": signature})


def issue_reference(claims, evidence, key):
    """Only explicitly non-authoritative reference observations/plans."""
    _claims(claims)
    if claims["profile"] == "local":
        raise ValueError("receipt: local evidence must come from runtime")
    return _sign(claims, evidence, key)


def _unique(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("receipt: duplicate field")
        result[key] = value
    return result


def verify_receipt(receipt, evidence, trusted_key, expected):
    """Verify the issuer, exact evidence bytes and independently expected context.
    The caller is responsible for trust policy and native evidence verification.
    """
    if type(receipt) is not bytes or not 0 < len(receipt) <= MAX_RECEIPT:
        raise ValueError("receipt: invalid envelope size")
    if type(trusted_key) is not bytes or len(trusted_key) != 32:
        raise ValueError("receipt: invalid trusted issuer")
    if type(expected) is not dict or set(expected) != CONTEXT_FIELDS:
        raise ValueError("receipt: complete expected context required")
    try:
        env = json.loads(receipt, object_pairs_hook=_unique)
        if type(env) is not dict or set(env) != {"payload", "signature"}:
            raise ValueError("receipt: invalid envelope")
        p = env["payload"]
        if type(p) is not dict or set(p) != PAYLOAD_FIELDS:
            raise ValueError("receipt: invalid payload")
        _claims({k: p[k] for k in CLAIMS})
        if (
            p["schema"] != SCHEMA
            or _hex(p["issuer"]) != trusted_key
            or p["evidence_sha256"] != _evidence(evidence)
            or any(p[k] != expected[k] for k in CONTEXT_FIELDS)
            or canonical(env) != receipt
        ):
            raise ValueError("receipt: context or evidence mismatch")
        Ed25519PublicKey.from_public_bytes(trusted_key).verify(
            _hex(env["signature"], 64), DOMAIN + canonical(p)
        )
    except (InvalidSignature, UnicodeError, TypeError, RecursionError) as exc:
        raise ValueError("receipt: verification failed") from exc
    return p


def issue_local(runtime, action_id, key):
    """Trusted host export of actual SQLite evidence; no tool dispatch or state change.
    A pending or failed action does not prove that external effects did not occur.
    """
    if not isinstance(runtime, Runtime):
        raise TypeError("receipt: runtime required")
    _hex(action_id)
    with runtime._transaction() as db:
        config = json.loads(
            bytes(
                db.execute("SELECT value FROM config WHERE singleton=1").fetchone()[0]
            )
        )
        pir = config.get("pir")
        if pir is None or config["runtime_id"] != runtime.runtime_id:
            raise ValueError("receipt: PIR runtime required")
        row = db.execute(
            "SELECT * FROM actions WHERE digest=?", (action_id,)
        ).fetchone()
        if row is None or row["status"] not in STATUSES["local"]:
            raise ValueError("receipt: no eligible action")
        request = bytes(row["request"])
        req = json.loads(request)
        artifact_hash = digest(bytes(row["artifact"]))
        if (
            digest(request) != action_id
            or canonical(req) != request
            or req["domain"] != RUNTIME_DOMAIN + "/ACTION"
            or req["runtime_id"] != runtime.runtime_id
            or req["agent"] != row["agent"]
            or req["nonce"] != row["nonce"]
            or req["artifact_sha256"] != artifact_hash
            or req["policy"] != config["adapters"].get(req["adapter"])
        ):
            raise ValueError("receipt: inconsistent action")
        events = []
        for event in db.execute("SELECT * FROM events ORDER BY seq"):
            body = json.loads(event["payload"])
            if body["details"].get("action") == action_id:
                if body["agent"] != row["agent"]:
                    raise ValueError("receipt: wrong event identity")
                events.append((body, event["hash"]))
        reserved = [(body, h) for body, h in events if body["kind"] == "reserved"]
        if len(reserved) != 1:
            raise ValueError("receipt: missing unique reservation")
        reservation, reservation_hash = reserved[0]
        details = reservation["details"]
        policy = details.get("pir_grant_sha256")
        _hex(policy)
        operator = _hex(config["operator"])
        records = []
        for record in pir["grants"]:
            g = decode_signed(record, operator)
            if g.subject == row["agent"] and g.adapter == req["adapter"]:
                records.append((record, g))
        if len(records) != 1:
            raise ValueError("receipt: missing grant")
        record, grant = records[0]
        if (
            grant.commitment != policy
            or grant.scope != pir["scope"]
            or grant.artifact_hash.hex() != artifact_hash
        ):
            raise ValueError("receipt: grant mismatch")
        agent_key = bytes(runtime._agent(db, row["agent"])["public_key"])
        try:
            Ed25519PublicKey.from_public_bytes(agent_key).verify(
                _hex(details["signature"], 64), request
            )
        except InvalidSignature as exc:
            raise ValueError("receipt: invalid agent request signature") from exc
        status = row["status"]
        terminal = [
            (body, h)
            for body, h in events
            if body["kind"] in ("completed", "execution_failed", "retired")
        ]
        result = None
        terminal_hash = None
        if status == "pending":
            if terminal or row["output"] is not None:
                raise ValueError("receipt: pending evidence mismatch")
        else:
            if len(terminal) != 1:
                raise ValueError("receipt: missing terminal evidence")
            body, terminal_hash = terminal[0]
            if status == "completed":
                if body["kind"] != "completed" or row["output"] is None:
                    raise ValueError("receipt: completion mismatch")
                result = digest(bytes(row["output"]))
                if body["details"].get("output_sha256") != result:
                    raise ValueError("receipt: output mismatch")
            elif (
                body["kind"] not in ("execution_failed", "retired")
                or row["output"] is not None
            ):
                raise ValueError("receipt: failure mismatch")
        evidence = canonical(
            {
                "request_hex": request.hex(),
                "agent_public_key": agent_key.hex(),
                "agent_signature": details["signature"],
                "owner_public_key": operator.hex(),
                "owner_grant": record,
                "reservation_event_sha256": reservation_hash,
                "terminal_event_sha256": terminal_hash,
                "status": status,
                "output_sha256": result,
            }
        )
        claims = {
            "profile": "local",
            "status": status,
            "scope": pir["scope"],
            "context": "runtime:" + runtime.runtime_id,
            "subject": row["agent"],
            "source_sha256": record["source_sha256"],
            "policy_sha256": policy,
            "native_id": action_id,
            "artifact_sha256": artifact_hash,
            "result_sha256": result,
        }
    return _sign(claims, evidence, key), evidence


class ReceiptLedger:
    """Local consumer deduplication, not global replay protection or execution."""

    def __init__(self, path):
        self.path = str(Path(path))
        with sqlite3.connect(self.path) as db:
            db.execute(
                "CREATE TABLE IF NOT EXISTS pir_receipts (context TEXT NOT NULL, native_id TEXT NOT NULL, receipt_sha256 TEXT NOT NULL, PRIMARY KEY(context,native_id))"
            )

    def accept_local_completion(
        self, receipt, evidence, trusted_key, expected, **native_inputs
    ):
        p = verify_local_provenance(
            receipt, evidence, trusted_key, expected, **native_inputs
        )
        with sqlite3.connect(self.path, timeout=10) as db:
            db.execute("PRAGMA synchronous=FULL")
            try:
                db.execute(
                    "INSERT INTO pir_receipts VALUES(?,?,?)",
                    (p["context"], p["native_id"], digest(receipt)),
                )
            except sqlite3.IntegrityError as exc:
                raise ValueError("receipt: replay") from exc
        return p


def _canonical_object(data, fields):
    try:
        obj = json.loads(data, object_pairs_hook=_unique)
        if type(obj) is not dict or set(obj) != fields or canonical(obj) != data:
            raise ValueError("receipt: invalid native evidence")
    except (UnicodeError, TypeError, RecursionError) as exc:
        raise ValueError("receipt: malformed native evidence") from exc
    return obj


def verify_local_provenance(
    receipt,
    evidence,
    trusted_key,
    expected,
    *,
    owner_key=None,
    agent_key=None,
    source=None,
    artifact=None,
    output=None,
):
    """Verify a local completion against independently supplied keys and bytes.

    The owner and agent sign the grant and request respectively. The trusted
    receipt issuer attests completion, output and event hashes. These bytes do
    not establish external effects or independently prove callback execution.
    """
    payload = verify_receipt(receipt, evidence, trusted_key, expected)
    if payload["profile"] != "local" or payload["status"] != "completed":
        raise ValueError("receipt: local completion required")
    if any(type(key) is not bytes or len(key) != 32 for key in (owner_key, agent_key)):
        raise ValueError("receipt: independently trusted owner and agent keys required")
    if (
        type(source) is not bytes
        or not 0 < len(source) <= MAX_EVIDENCE
        or type(artifact) is not bytes
        or not 0 < len(artifact) <= MAX_ARTIFACT_BYTES
        or type(output) is not bytes
        or len(output) > MAX_ARTIFACT_BYTES
    ):
        raise ValueError("receipt: invalid source, artifact or output")

    proof = _canonical_object(
        evidence,
        {
            "request_hex",
            "agent_public_key",
            "agent_signature",
            "owner_public_key",
            "owner_grant",
            "reservation_event_sha256",
            "terminal_event_sha256",
            "status",
            "output_sha256",
        },
    )
    if (
        _hex(proof["agent_public_key"]) != agent_key
        or _hex(proof["owner_public_key"]) != owner_key
        or proof["status"] != payload["status"]
        or proof["output_sha256"] != payload["result_sha256"]
        or digest(output) != payload["result_sha256"]
    ):
        raise ValueError("receipt: signer, status or output mismatch")
    _hex(proof["reservation_event_sha256"])
    _hex(proof["terminal_event_sha256"])

    grant_record = proof["owner_grant"]
    grant = decode_signed(grant_record, owner_key)
    if (
        digest(source) != grant_record["source_sha256"]
        or grant_record["source_sha256"] != payload["source_sha256"]
        or grant.commitment != payload["policy_sha256"]
        or grant.scope != payload["scope"]
        or grant.subject != payload["subject"]
        or digest(artifact) != grant.artifact_hash.hex()
        or grant.artifact_hash.hex() != payload["artifact_sha256"]
    ):
        raise ValueError("receipt: owner grant, source or artifact mismatch")

    request_hex = proof["request_hex"]
    if (
        type(request_hex) is not str
        or len(request_hex) % 2
        or not re.fullmatch(r"[0-9a-f]+", request_hex)
    ):
        raise ValueError("receipt: invalid signed request encoding")
    request = bytes.fromhex(request_hex)
    action = _canonical_object(
        request,
        {
            "domain",
            "runtime_id",
            "agent",
            "adapter",
            "artifact_sha256",
            "nonce",
            "expires_at",
            "policy",
        },
    )
    if (
        digest(request) != payload["native_id"]
        or action["domain"] != RUNTIME_DOMAIN + "/ACTION"
        or action["runtime_id"] != payload["context"][len("runtime:") :]
        or action["agent"] != grant.subject
        or action["adapter"] != grant.adapter
        or action["artifact_sha256"] != payload["artifact_sha256"]
        or type(action["nonce"]) is not str
        or not 0 < len(action["nonce"]) <= 128
        or type(action["expires_at"]) is not int
        or not 0 < action["expires_at"] <= grant.expires
    ):
        raise ValueError("receipt: signed action mismatch")
    try:
        Ed25519PublicKey.from_public_bytes(agent_key).verify(
            _hex(proof["agent_signature"], 64), request
        )
    except InvalidSignature as exc:
        raise ValueError("receipt: invalid native action signature") from exc
    return payload
