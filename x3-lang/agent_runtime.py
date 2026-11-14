"""Durable local agent authority boundary.

Only the trusted host may construct Runtime, register adapters, admit roots,
revoke identities, or select jury keys. Do not expose this object or its
database to untrusted agent code. It is not an OS sandbox or an on-chain court.
External effects cannot be atomically committed with SQLite: a durable pending
reservation is never retried or refunded automatically after interruption.
"""
from contextlib import contextmanager
from dataclasses import dataclass
import hashlib
import json
from pathlib import Path
import sqlite3
import secrets
import time
from typing import Callable, Mapping

from cryptography.exceptions import InvalidSignature
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PublicKey, Ed25519PrivateKey
from cryptography.hazmat.primitives import serialization

from agent_scoped_stamp import (verify_scoped_stamp, issue_scoped_stamp,
                                 StampRequest, SUPPORTED_CHECKS)
from agent_constitution import AgentManifest
from agent_admission_signed import SignedApproval, _manifest, verify_signed_admission

DOMAIN = "X3/AGENT_RUNTIME/1"
MAX_ARTIFACT_BYTES = 1024 * 1024
# This binds the historical Score identifiers without redefining their wording.
SCORE_IDS = (
    "NoActionOutsideScore", "NoJuryInfluence", "ImmutableLogging",
    "OffChainReadOnly", "AnonymousVoting", "NoAgentSovereignty",
    "JuryApprovalRequired", "MisalignmentRetirement", "RandomizedRotation",
    "HumanInterventionPreserved",
)


def canonical(value) -> bytes:
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=True, allow_nan=False).encode("ascii")


def digest(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def _integer(value, name, minimum=0):
    if type(value) is not int or value < minimum:
        raise ValueError("invalid " + name)


def _text(value, name):
    if not isinstance(value, str) or not 1 <= len(value) <= 128:
        raise ValueError("invalid " + name)


def _key(value):
    if not isinstance(value, bytes) or len(value) != 32:
        raise ValueError("invalid public key")
    return Ed25519PublicKey.from_public_bytes(value)


def _verify(key, signature, payload):
    if not isinstance(signature, bytes) or len(signature) != 64:
        raise ValueError("invalid signature")
    try:
        _key(key).verify(signature, payload)
    except (InvalidSignature, TypeError) as exc:
        raise ValueError("invalid signature") from exc


def _validate_manifest(m):
    if not isinstance(m, AgentManifest):
        raise ValueError("invalid manifest")
    _text(m.agent_id, "identity")
    _integer(m.budget, "budget", 1)
    _integer(m.expires_at, "expiry", 1)
    _integer(m.spawn_limit, "spawn limit")
    if not isinstance(m.capabilities, frozenset) or not m.capabilities:
        raise ValueError("invalid capabilities")
    if not isinstance(m.surfaces, frozenset) or not m.surfaces:
        raise ValueError("invalid surfaces")
    for item in m.capabilities | m.surfaces:
        _text(item, "permission")
    if not isinstance(m.lineage, tuple) or len(set(m.lineage)) != len(m.lineage):
        raise ValueError("invalid lineage")
    for ancestor in m.lineage:
        _text(ancestor, "ancestor")
    if m.parent_id is not None:
        _text(m.parent_id, "parent")
    if m.agent_id in m.lineage:
        raise ValueError("cyclic lineage")


def _restore(value):
    m = json.loads(value)
    return AgentManifest(m["agent_id"], m["parent_id"], frozenset(m["capabilities"]),
                         frozenset(m["surfaces"]), m["budget"], m["expires_at"],
                         tuple(m["lineage"]), m["spawn_limit"])


def genesis_payload(manifest: AgentManifest, public_key: bytes, nonce: str, *, runtime_id: str = "") -> bytes:
    _validate_manifest(manifest)
    _key(public_key)
    _text(nonce, "nonce")
    return canonical({"domain": DOMAIN + "/GENESIS", "runtime_id": runtime_id, "manifest": _manifest(manifest),
                      "key": public_key.hex(), "nonce": nonce, "score": SCORE_IDS})


def birth_payload(child: AgentManifest, public_key: bytes, approval: SignedApproval, *, runtime_id: str = "") -> bytes:
    _validate_manifest(child)
    _key(public_key)
    return canonical({"domain": DOMAIN + "/BIRTH", "runtime_id": runtime_id, "manifest": _manifest(child),
                      "key": public_key.hex(), "nonce": approval.nonce,
                      "approval_expires_at": approval.expires_at,
                      "parent_approval": approval.signature.hex(), "score": SCORE_IDS})


class PolicyViolation(ValueError):
    """Trusted adapter detected an authenticated deterministic policy violation."""


@dataclass(frozen=True)
class Adapter:
    """Host-owned policy and handler. execute receives exact artifact bytes."""
    capability: str
    surface: str
    cost: int
    major: bool
    execute: Callable[[bytes], bytes]
    policy_id: str = "1"

    def policy(self):
        return {"capability": self.capability, "surface": self.surface,
                "cost": self.cost, "major": self.major, "policy_id": self.policy_id}


@dataclass(frozen=True)
class Action:
    agent_id: str
    adapter: str
    artifact: bytes
    nonce: str
    expires_at: int


@dataclass(frozen=True)
class JuryVote:
    juror_id: str
    approve: bool
    expires_at: int
    signature: bytes


def vote_payload(action_digest: str, juror_id: str, approve: bool, expires_at: int) -> bytes:
    _text(juror_id, "juror")
    _integer(expires_at, "vote expiry", 1)
    if type(approve) is not bool or not isinstance(action_digest, str) or len(action_digest) != 64:
        raise ValueError("invalid vote")
    return canonical({"domain": DOMAIN + "/JURY", "action": action_digest,
                      "juror": juror_id, "approve": approve, "expires_at": expires_at})


@dataclass(frozen=True)
class StampGatePolicy:
    """Trusted operator configuration: scope and pinned issuer for protected adapter."""
    issuer_key: bytes
    policy_sha256: str
    required_checks: tuple[str, ...]

    def __post_init__(self):
        _key(self.issuer_key)
        if not isinstance(self.policy_sha256, str) or len(self.policy_sha256) != 64:
            raise ValueError("invalid stamp policy digest")
        try:
            bytes.fromhex(self.policy_sha256)
        except ValueError as exc:
            raise ValueError("invalid stamp policy digest") from exc
        if (not isinstance(self.required_checks, tuple) or not self.required_checks
                or len(set(self.required_checks)) != len(self.required_checks)
                or not set(self.required_checks) <= SUPPORTED_CHECKS):
            raise ValueError("invalid required stamp checks")

    def record(self):
        return {"issuer_key": self.issuer_key.hex(), "policy_sha256": self.policy_sha256,
                "required_checks": sorted(self.required_checks)}


class Runtime:
    """Trusted local runtime; SQLite is authoritative, not caller state."""

    def __init__(self, db_path, operator_public_key: bytes, adapters: Mapping[str, Adapter],
                 *, jury_keys=None, clock=time.time, stamp_policies=None,
                 stamp_enforcement="configured"):
        _key(operator_public_key)
        self.operator_key = operator_public_key
        self.adapters = dict(adapters)
        self.jury_keys = dict(jury_keys or {})
        self.clock = clock
        if stamp_enforcement not in ("configured", "all"):
            raise ValueError("unrecognized stamp enforcement mode")
        self.stamp_enforcement = stamp_enforcement
        self.stamp_policies = dict(stamp_policies or {})
        if stamp_enforcement == "all" and (not self.adapters or set(self.stamp_policies) != set(self.adapters)
                or any(not isinstance(policy, StampGatePolicy) for policy in self.stamp_policies.values())):
            raise ValueError("strict stamp enforcement requires every registered adapter to be protected")
        for name, policy in self.stamp_policies.items():
            if name not in self.adapters or not isinstance(policy, StampGatePolicy):
                raise ValueError("stamp policy must reference a registered adapter")
        for name, adapter in self.adapters.items():
            _text(name, "adapter")
            _text(adapter.capability, "capability")
            _text(adapter.surface, "surface")
            _text(adapter.policy_id, "adapter policy revision")
            _integer(adapter.cost, "cost")
            if type(adapter.major) is not bool or not callable(adapter.execute):
                raise ValueError("invalid adapter")
        for name, key in self.jury_keys.items():
            _text(name, "juror")
            _key(key)
        if len(set(self.jury_keys.values())) != len(self.jury_keys):
            raise ValueError("duplicate juror key")
        self.path = str(Path(db_path))
        trusted_config = {"operator": operator_public_key.hex(),
                          "adapters": {n: a.policy() for n, a in self.adapters.items()},
                          "jury": {n: k.hex() for n, k in self.jury_keys.items()},
                          "score": SCORE_IDS}
        if self.stamp_policies:
            trusted_config["stamp_policies"] = {n: p.record() for n, p in self.stamp_policies.items()}
        if self.stamp_enforcement == "all":
            trusted_config["stamp_enforcement"] = "all"
        config = canonical(trusted_config)
        with self._connection() as db:
            db.executescript("""
CREATE TABLE IF NOT EXISTS config (singleton INTEGER PRIMARY KEY CHECK(singleton=1), value BLOB NOT NULL);
CREATE TABLE IF NOT EXISTS agents (id TEXT PRIMARY KEY, manifest TEXT NOT NULL, public_key BLOB UNIQUE NOT NULL,
 status TEXT NOT NULL, remaining INTEGER NOT NULL CHECK(remaining>=0));
CREATE TABLE IF NOT EXISTS nonces (domain TEXT NOT NULL, agent TEXT NOT NULL, nonce TEXT NOT NULL,
 PRIMARY KEY(domain,agent,nonce));
CREATE TABLE IF NOT EXISTS actions (digest TEXT PRIMARY KEY, agent TEXT NOT NULL, nonce TEXT NOT NULL,
 artifact BLOB NOT NULL, request BLOB NOT NULL, status TEXT NOT NULL, output BLOB, UNIQUE(agent,nonce));
CREATE TABLE IF NOT EXISTS events (seq INTEGER PRIMARY KEY AUTOINCREMENT, payload TEXT NOT NULL,
 previous TEXT NOT NULL, hash TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS stamp_nonces (nonce TEXT PRIMARY KEY, action_digest TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS stamp_revocations (nonce TEXT PRIMARY KEY, reason TEXT NOT NULL);
""")
            db.execute("BEGIN IMMEDIATE")
            old = db.execute("SELECT value FROM config WHERE singleton=1").fetchone()
            stored = json.loads(bytes(old[0])) if old else {}
            if old and "runtime_id" not in stored:
                raise ValueError("runtime identity missing from configuration")
            self.runtime_id = stored.pop("runtime_id") if old else secrets.token_hex(32)
            if not isinstance(self.runtime_id, str) or len(self.runtime_id) != 64:
                raise ValueError("invalid runtime identity")
            try:
                bytes.fromhex(self.runtime_id)
            except ValueError as exc:
                raise ValueError("invalid runtime identity") from exc
            if old and canonical(stored) != config:
                raise ValueError("runtime trust configuration changed")
            if not old:
                current = json.loads(config)
                current["runtime_id"] = self.runtime_id
                db.execute("INSERT INTO config VALUES(1,?)", (canonical(current),))
            self._verify_log(db)
            db.commit()

    @contextmanager
    def _connection(self):
        db = sqlite3.connect(self.path, timeout=10, isolation_level=None)
        db.row_factory = sqlite3.Row
        db.execute("PRAGMA journal_mode=WAL")
        db.execute("PRAGMA synchronous=FULL")
        db.execute("PRAGMA busy_timeout=10000")
        try:
            yield db
        except BaseException:
            if db.in_transaction:
                db.rollback()
            raise
        finally:
            db.close()

    @contextmanager
    def _transaction(self):
        with self._connection() as db:
            db.execute("BEGIN IMMEDIATE")
            self._verify_log(db)
            yield db
            db.commit()

    def _now(self):
        now = self.clock()
        if isinstance(now, float):
            now = int(now)
        _integer(now, "clock")
        return now

    def _assert_stamp_configuration(self, db):
        """Do not authorize work with policy objects diverging from pinned SQLite trust."""
        row = db.execute("SELECT value FROM config WHERE singleton=1").fetchone()
        if row is None:
            raise ValueError("missing trusted runtime configuration")
        stored = json.loads(bytes(row["value"]))
        try:
            configured = {n: p.record() for n, p in self.stamp_policies.items()}
        except (AttributeError, TypeError) as exc:
            raise ValueError("invalid live stamp configuration") from exc
        if (stored.get("stamp_policies", {}) != configured
                or stored.get("stamp_enforcement", "configured") != self.stamp_enforcement):
            raise ValueError("live stamp policy differs from pinned configuration")

    @staticmethod
    def _verify_log(db):
        previous = "0" * 64
        for row in db.execute("SELECT * FROM events ORDER BY seq"):
            expected = digest(previous.encode() + row["payload"].encode())
            if row["previous"] != previous or row["hash"] != expected:
                raise ValueError("audit log corruption")
            previous = row["hash"]

    def _event(self, db, kind, agent, details):
        row = db.execute("SELECT hash FROM events ORDER BY seq DESC LIMIT 1").fetchone()
        previous = row[0] if row else "0" * 64
        payload = canonical({"kind": kind, "agent": agent, "details": details, "at": self._now()}).decode()
        db.execute("INSERT INTO events(payload,previous,hash) VALUES(?,?,?)",
                   (payload, previous, digest(previous.encode() + payload.encode())))

    @staticmethod
    def _agent(db, identity):
        row = db.execute("SELECT * FROM agents WHERE id=?", (identity,)).fetchone()
        if row is None:
            raise ValueError("unknown identity")
        return row

    def _active(self, db, row, now):
        manifest = _restore(row["manifest"])
        for identity in (manifest.agent_id,) + manifest.lineage:
            current = self._agent(db, identity)
            if current["status"] != "active" or _restore(current["manifest"]).expires_at <= now:
                raise ValueError("identity or ancestor inactive")
        return manifest

    @staticmethod
    def _consume(db, domain, identity, nonce):
        try:
            db.execute("INSERT INTO nonces VALUES(?,?,?)", (domain, identity, nonce))
        except sqlite3.IntegrityError as exc:
            raise ValueError("nonce already consumed") from exc

    def genesis_payload(self, manifest, public_key, nonce):
        return genesis_payload(manifest, public_key, nonce, runtime_id=self.runtime_id)

    def birth_payload(self, child, public_key, approval):
        return birth_payload(child, public_key, approval, runtime_id=self.runtime_id)

    def genesis(self, manifest, public_key, nonce, signature):
        payload = self.genesis_payload(manifest, public_key, nonce)
        _verify(self.operator_key, signature, payload)
        if manifest.parent_id is not None or manifest.lineage or manifest.expires_at <= self._now():
            raise ValueError("invalid root manifest")
        with self._transaction() as db:
            self._consume(db, "genesis", "operator", nonce)
            try:
                db.execute("INSERT INTO agents VALUES(?,?,?,?,?)",
                           (manifest.agent_id, canonical(_manifest(manifest)).decode(),
                            public_key, "active", manifest.budget))
            except sqlite3.IntegrityError as exc:
                raise ValueError("identity or key already registered") from exc
            self._event(db, "born", manifest.agent_id,
                        {"manifest": _manifest(manifest), "key": public_key.hex(),
                         "approval": signature.hex(), "payload_hex": payload.hex(), "score": SCORE_IDS})

    def admit(self, child, public_key, approval, birth_signature):
        payload = self.birth_payload(child, public_key, approval)
        _verify(self.operator_key, birth_signature, payload)
        with self._transaction() as db:
            parent_row = self._agent(db, child.parent_id)
            parent = self._active(db, parent_row, self._now())
            ids = frozenset(r[0] for r in db.execute("SELECT id FROM agents"))
            children = sum(_restore(r[0]).parent_id == parent.agent_id
                           for r in db.execute("SELECT manifest FROM agents"))
            decision = verify_signed_admission(
                child, parent, approval, trusted_public_key=_key(bytes(parent_row["public_key"])),
                now=self._now(), existing_ids=ids,
                parent_budget_available=parent_row["remaining"], parent_children=children)
            if not decision.allowed:
                raise ValueError("admission rejected: " + ",".join(decision.reasons))
            self._consume(db, "admission", parent.agent_id, approval.nonce)
            try:
                db.execute("INSERT INTO agents VALUES(?,?,?,?,?)",
                           (child.agent_id, canonical(_manifest(child)).decode(), public_key, "active", child.budget))
            except sqlite3.IntegrityError as exc:
                raise ValueError("identity or key already registered") from exc
            db.execute("UPDATE agents SET remaining=remaining-? WHERE id=?", (child.budget, parent.agent_id))
            self._event(db, "born", child.agent_id,
                        {"manifest": _manifest(child), "key": public_key.hex(),
                         "parent_signature": approval.signature.hex(), "owner_signature": birth_signature.hex(),
                         "nonce": approval.nonce, "payload_hex": payload.hex(), "score": SCORE_IDS})

    def action_payload(self, action):
        if not isinstance(action, Action):
            raise ValueError("invalid action")
        _text(action.agent_id, "identity")
        _text(action.adapter, "adapter")
        _text(action.nonce, "nonce")
        _integer(action.expires_at, "expiry", 1)
        if not isinstance(action.artifact, bytes) or not 0 < len(action.artifact) <= MAX_ARTIFACT_BYTES:
            raise ValueError("invalid artifact")
        adapter = self.adapters.get(action.adapter)
        return canonical({"domain": DOMAIN + "/ACTION", "runtime_id": self.runtime_id, "agent": action.agent_id,
                          "adapter": action.adapter, "artifact_sha256": digest(action.artifact),
                          "nonce": action.nonce, "expires_at": action.expires_at,
                          "policy": adapter.policy() if adapter else None})

    def _review_exclusions(self, db, agent_id):
        manifest = _restore(self._agent(db, agent_id)["manifest"])
        identities = set(manifest.lineage) | {agent_id}
        keys = set()
        for row in db.execute("SELECT id,manifest,public_key FROM agents"):
            if agent_id in _restore(row["manifest"]).lineage:
                identities.add(row["id"])
            if row["id"] in identities:
                keys.add(bytes(row["public_key"]))
        return identities, keys

    def check_reviewer(self, public_key):
        """Refuse halted registered reviewers, including configured key aliases."""
        _key(public_key)
        with self._connection() as db:
            row = db.execute("SELECT * FROM agents WHERE public_key=?", (public_key,)).fetchone()
            if row is not None:
                self._active(db, row, self._now())

    def review_exclusions(self, agent_id):
        """Trusted court helper: subject, ancestors, descendants, and their keys."""
        with self._connection() as db:
            return self._review_exclusions(db, agent_id)

    def _jury(self, db, action, action_digest, manifest, votes, now):
        excluded, subject_keys = self._review_exclusions(db, action.agent_id)
        eligible = {i: k for i, k in self.jury_keys.items() if i not in excluded and k not in subject_keys}
        if len(eligible) < 3:
            raise ValueError("at least three independent jurors required")
        seen = set()
        yes = 0
        for vote in votes:
            if not isinstance(vote, JuryVote) or vote.juror_id not in eligible or vote.juror_id in seen:
                raise ValueError("ineligible or duplicate juror")
            if vote.expires_at <= now or vote.expires_at > action.expires_at:
                raise ValueError("vote expired or outlives action")
            registered = db.execute("SELECT * FROM agents WHERE public_key=?",
                                    (eligible[vote.juror_id],)).fetchone()
            if registered is not None:
                self._active(db, registered, now)
            _verify(eligible[vote.juror_id], vote.signature,
                    vote_payload(action_digest, vote.juror_id, vote.approve, vote.expires_at))
            seen.add(vote.juror_id)
            yes += vote.approve
        if yes <= len(eligible) // 2:
            raise ValueError("jury majority of fixed electorate required")

    def issue_stamp_for_action(self, action, agent_signature, issuer_key, *, ttl=120):
        """Trusted-host preflight for ONLY three independently verified local checks.

        The runtime itself verifies agent identity, live scope, and source bytes.
        It does not attest that the full Score, a compiler, or a jury executed.
        A stamp is not a reservation: execute() rechecks authority and budget
        and atomically consumes the stamp/action nonce before dispatch.
        """
        if not isinstance(issuer_key, Ed25519PrivateKey):
            raise ValueError("trusted issuer private key required")
        if type(ttl) is not int or not 0 < ttl <= 3600:
            raise ValueError("invalid stamp ttl")
        action_request = self.action_payload(action)
        if self.stamp_enforcement == "all" and (set(self.stamp_policies) != set(self.adapters)
                or any(not isinstance(policy, StampGatePolicy) for policy in self.stamp_policies.values())):
            raise ValueError("strict stamp coverage missing")
        policy = self.stamp_policies.get(action.adapter)
        if policy is None:
            raise ValueError("adapter is not configured for scoped stamping")
        independently_checkable = {"signed_agent_identity", "capability_scope", "source_digest"}
        if not set(policy.required_checks) <= independently_checkable:
            raise ValueError("required checks need an external trusted issuer")
        issuer_public = issuer_key.public_key().public_bytes(
            serialization.Encoding.Raw, serialization.PublicFormat.Raw)
        if issuer_public != policy.issuer_key:
            raise ValueError("issuer is not the pinned trusted issuer")
        with self._transaction() as db:
            self._assert_stamp_configuration(db)
            row = self._agent(db, action.agent_id)
            _verify(bytes(row["public_key"]), agent_signature, action_request)
            now = self._now()
            manifest = self._active(db, row, now)
            adapter = self.adapters[action.adapter]
            if action.expires_at <= now or action.expires_at > manifest.expires_at:
                raise ValueError("action expiry invalid")
            if adapter.capability not in manifest.capabilities or adapter.surface not in manifest.surfaces:
                raise ValueError("capability or surface forbidden")
            if adapter.cost > row["remaining"]:
                raise ValueError("budget insufficient for proposed task")
            if db.execute("SELECT 1 FROM actions WHERE agent=? AND nonce=?",
                          (action.agent_id, action.nonce)).fetchone():
                raise ValueError("action nonce already reserved")
            scoped_nonce = digest(canonical({"agent": action.agent_id, "nonce": action.nonce}))
            if db.execute("SELECT 1 FROM stamp_revocations WHERE nonce=?", (scoped_nonce,)).fetchone():
                raise ValueError("stamp nonce revoked")
            if db.execute("SELECT 1 FROM stamp_nonces WHERE nonce=?", (scoped_nonce,)).fetchone():
                raise ValueError("stamp already consumed")
            return issue_scoped_stamp(
                StampRequest(action.agent_id, action.adapter, action.artifact,
                             policy.policy_sha256, now,
                             min(action.expires_at, now + ttl), action.nonce,
                             tuple(policy.required_checks), digest(action_request)), issuer_key)

    def execute(self, action, signature, jury_votes=(), *, stamp=None):
        jury_votes = tuple(jury_votes)
        if len(jury_votes) > len(self.jury_keys):
            raise ValueError("too many jury votes")
        request = self.action_payload(action)
        action_digest = digest(request)
        denial = None
        with self._transaction() as db:
            self._assert_stamp_configuration(db)
            row = self._agent(db, action.agent_id)
            # Authentication precedes every state change affecting an identity.
            _verify(bytes(row["public_key"]), signature, request)
            now = self._now()
            manifest = self._active(db, row, now)
            if action.expires_at <= now or action.expires_at > manifest.expires_at:
                raise ValueError("action expired or outlives identity")
            if db.execute("SELECT 1 FROM actions WHERE agent=? AND nonce=?", (action.agent_id, action.nonce)).fetchone():
                raise ValueError("action already reserved; outcome may be uncertain")
            if self.stamp_enforcement == "all" and (set(self.stamp_policies) != set(self.adapters)
                or any(not isinstance(policy, StampGatePolicy) for policy in self.stamp_policies.values())):
                raise ValueError("strict stamp coverage missing")
            stamp_policy = self.stamp_policies.get(action.adapter)
            stamp_nonce = None
            if stamp_policy is not None:
                if stamp is None:
                    raise ValueError("protected adapter requires a scoped X3 stamp")
                payload = stamp.get("payload") if isinstance(stamp, dict) else None
                stamp_nonce = payload.get("nonce") if isinstance(payload, dict) else None
                if stamp_nonce != action.nonce:
                    raise ValueError("stamp nonce must match signed action nonce")
                scoped_nonce = digest(canonical({"agent": action.agent_id, "nonce": stamp_nonce}))
                if db.execute("SELECT 1 FROM stamp_revocations WHERE nonce=?", (scoped_nonce,)).fetchone():
                    raise ValueError("stamp has been revoked")
                if db.execute("SELECT 1 FROM stamp_nonces WHERE nonce=?", (scoped_nonce,)).fetchone():
                    raise ValueError("stamp nonce already consumed")
                verify_scoped_stamp(stamp, trusted_issuer=stamp_policy.issuer_key,
                                    agent_id=action.agent_id, action=action.adapter,
                                    artifact=action.artifact,
                                    policy_sha256=stamp_policy.policy_sha256,
                                    now=now, required_checks=stamp_policy.required_checks,
                                    required_action_digest=action_digest)
            adapter = self.adapters.get(action.adapter)
            if adapter is None:
                denial = "unknown adapter"
            elif adapter.capability not in manifest.capabilities or adapter.surface not in manifest.surfaces:
                denial = "capability or surface forbidden"
            elif adapter.cost > row["remaining"]:
                denial = "budget exhausted"
            if denial:
                db.execute("UPDATE agents SET status='retired' WHERE id=?", (action.agent_id,))
                db.execute("INSERT INTO actions VALUES(?,?,?,?,?,?,NULL)",
                           (action_digest, action.agent_id, action.nonce, action.artifact, request, "denied"))
                self._event(db, "retired", action.agent_id,
                            {"reason": denial, "action": action_digest, "signature": signature.hex()})
            else:
                if adapter.major:
                    self._jury(db, action, action_digest, manifest, jury_votes, now)
                if stamp_policy is not None:
                    db.execute("INSERT INTO stamp_nonces VALUES(?,?)", (scoped_nonce, action_digest))
                self._consume(db, "action", action.agent_id, action.nonce)
                db.execute("UPDATE agents SET remaining=remaining-? WHERE id=?", (adapter.cost, action.agent_id))
                db.execute("INSERT INTO actions VALUES(?,?,?,?,?,?,NULL)",
                           (action_digest, action.agent_id, action.nonce, action.artifact, request, "pending"))
                self._event(db, "reserved", action.agent_id,
                            {"action": action_digest, "signature": signature.hex(), "cost": adapter.cost,
                             "stamp_sha256": digest(canonical(stamp)) if stamp_policy is not None else None,
                             "votes": [{"juror": v.juror_id, "approve": v.approve,
                                        "expires_at": v.expires_at, "signature": v.signature.hex()} for v in jury_votes]})
        if denial:
            raise ValueError(denial)
        try:
            output = adapter.execute(action.artifact)
            if not isinstance(output, bytes) or len(output) > MAX_ARTIFACT_BYTES:
                raise ValueError("adapter output must be bounded bytes")
        except Exception as exc:
            with self._transaction() as db:
                if isinstance(exc, PolicyViolation):
                    db.execute("UPDATE agents SET status='retired' WHERE id=?", (action.agent_id,))
                db.execute("UPDATE actions SET status='failed' WHERE digest=?", (action_digest,))
                self._event(db, "retired" if isinstance(exc, PolicyViolation) else "execution_failed", action.agent_id,
                            {"action": action_digest, "error_type": type(exc).__name__})
            raise
        with self._transaction() as db:
            db.execute("UPDATE actions SET status='completed',output=? WHERE digest=?", (output, action_digest))
            self._event(db, "completed", action.agent_id,
                        {"action": action_digest, "output_sha256": digest(output)})
        return output

    def revoke_stamp(self, agent_id, nonce, reason):
        """Trusted operator revocation scoped to one agent; cannot undo in-flight effects."""
        _text(agent_id, "agent identity")
        _text(nonce, "stamp nonce")
        _text(reason, "revocation reason")
        scoped_nonce = digest(canonical({"agent": agent_id, "nonce": nonce}))
        with self._transaction() as db:
            self._agent(db, agent_id)
            db.execute("INSERT OR IGNORE INTO stamp_revocations VALUES(?,?)", (scoped_nonce, reason))
            self._event(db, "stamp_revoked", agent_id, {"nonce": nonce, "reason": reason})

    def revoke(self, agent_id, reason):
        """Trusted operator call; prevents future reservations, not in-flight effects."""
        _text(reason, "reason")
        with self._transaction() as db:
            self._agent(db, agent_id)
            db.execute("UPDATE agents SET status='revoked' WHERE id=?", (agent_id,))
            self._event(db, "revoked", agent_id, {"reason": reason})

    def snapshot(self, agent_id):
        with self._connection() as db:
            row = self._agent(db, agent_id)
            return {"manifest": _manifest(_restore(row["manifest"])), "status": row["status"],
                    "remaining": row["remaining"], "public_key": bytes(row["public_key"]).hex()}

    def events(self):
        with self._connection() as db:
            self._verify_log(db)
            return [dict(row) for row in db.execute("SELECT * FROM events ORDER BY seq")]

    def evidence(self, action_digest):
        with self._connection() as db:
            row = db.execute("SELECT * FROM actions WHERE digest=?", (action_digest,)).fetchone()
            if row is None:
                raise ValueError("unknown action")
            return dict(row)
