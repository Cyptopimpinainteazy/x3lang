"""Experimental exact-tool PIR gate for the trusted Agent Law host.

Only owner-signed, canonical tool/NoAsset grants with empty requirements
are accepted. No claim of source-to-artifact correctness for untrusted
compiler output, OS isolation, external rollback, or financial authority.
"""
import hashlib
import re
from dataclasses import dataclass
from cryptography.exceptions import InvalidSignature
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PublicKey

DOMAIN = b"X3/PIR/OWNER-GRANT/1\x00"
WIRE = b"X3/PIR/GRANT/1\x00"
IDENT = re.compile(r"[A-Za-z0-9][A-Za-z0-9:._/-]{0,127}", re.ASCII)


@dataclass(frozen=True)
class ToolGrant:
    subject: str
    parent: str | None
    scope: str
    adapter: str
    artifact_hash: bytes
    expires: int
    depth: int
    commitment: str


def _deny(message):
    raise ValueError("PIR: " + message)


def _hex(value, size=None):
    if (type(value) is not str or len(value) > 4096 or len(value) % 2
            or re.fullmatch(r"[0-9a-f]+", value) is None
            or (size is not None and len(value) != 2 * size)):
        _deny("malformed hex")
    return bytes.fromhex(value)


def decode_signed(record, operator):
    if type(record) is not dict or set(record) != {"encoded", "source_sha256", "signature"}:
        _deny("unexpected grant fields")
    raw = _hex(record["encoded"])
    source = _hex(record["source_sha256"], 32)
    sig = _hex(record["signature"], 64)
    try:
        Ed25519PublicKey.from_public_bytes(operator).verify(sig, DOMAIN + source + raw)
    except (InvalidSignature, ValueError, TypeError):
        _deny("owner signature invalid")
    pos = 0

    def take(n):
        nonlocal pos
        if pos + n > len(raw):
            _deny("truncated grant")
        result = raw[pos:pos+n]
        pos += n
        return result

    def integer(n):
        return int.from_bytes(take(n), "big")

    def ident():
        try:
            result = take(integer(2)).decode("ascii")
        except UnicodeError:
            _deny("invalid identifier")
        if not IDENT.fullmatch(result):
            _deny("invalid identifier")
        return result

    if take(len(WIRE)) != WIRE:
        _deny("wrong grant domain")
    subject = ident()
    tag = integer(1)
    if tag not in (0, 1):
        _deny("invalid parent tag")
    parent = ident() if tag else None
    scope = ident()
    if parent == subject or integer(1) != 2:
        _deny("only exact tool resources supported")
    adapter = ident()
    artifact_hash = take(32)
    if integer(1) != 0:
        _deny("asset authority unsupported")
    expires, depth = integer(8), integer(1)
    if not expires or integer(1) != 0 or pos != len(raw):
        _deny("unsupported requirements or malformed grant")
    return ToolGrant(subject, parent, scope, adapter, artifact_hash, expires,
                     depth, hashlib.sha256(raw).hexdigest())


def configuration(scope, records, operator, adapters):
    """Provision from trusted owner-reviewed Rust compiler output only."""
    if scope is None and records is None:
        return None
    if (type(scope) is not str or not IDENT.fullmatch(scope)
            or type(records) not in (list, tuple) or len(records) > 1024):
        _deny("invalid strict configuration")
    items = {}
    for record in records:
        grant = decode_signed(record, operator)
        key = (grant.subject, grant.adapter)
        if grant.scope != scope or grant.adapter not in adapters or key in items:
            _deny("invalid scope, adapter or duplicate grant")
        items[key] = dict(record)
    return {"scope": scope, "grants": [items[k] for k in sorted(items)]}


def authorize(config, operator, action, lineage, now):
    """Check root-to-leaf authenticated native manifests before reservation."""
    if config is None:
        return None
    grants = {}
    for record in config["grants"]:
        g = decode_signed(record, operator)
        grants[(g.subject, g.adapter)] = g
    prior = None
    actual = hashlib.sha256(action.artifact).digest()
    for manifest in lineage:
        g = grants.get((manifest.agent_id, action.adapter))
        if (g is None or g.scope != config["scope"] or g.parent != manifest.parent_id
                or g.artifact_hash != actual or now >= g.expires
                or action.expires_at > g.expires or g.expires > manifest.expires_at):
            _deny("missing or out-of-scope, expired or mismatched grant")
        if prior is None:
            if g.parent is not None:
                _deny("root grant expected")
        elif (g.parent != prior.subject or g.depth >= prior.depth
              or g.expires > prior.expires):
            _deny("delegation expands parent authority")
        prior = g
    if prior is None or prior.subject != action.agent_id:
        _deny("invalid lineage")
    return prior.commitment
