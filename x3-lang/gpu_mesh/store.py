"""Durable state for the coordinator: contributors, app keys, nonces, receipts and the audit log.

SQLite in WAL mode. The admin CLI writes to the same database while the coordinator runs, and
the coordinator reads contributor status on every request, so an approval, suspension or
revocation takes effect at once without a restart.

Receipts are append-only and hash-chained: each row stores the previous row's hash, and the
coordinator signs every receipt hash. ``verify_receipts`` recomputes the chain, so an edited,
deleted or reordered row is detected.
"""

from __future__ import annotations

import hmac
import json
import secrets
import sqlite3
import threading
from datetime import datetime, timezone
from typing import Any, Dict, Iterable, List, Optional

from .protocol import ProtocolError, canonical, is_pubkey, now_ms, now_s, sha256_hex, verify_signature

GENESIS = "0" * 64
TIERS = ("fleet", "public")
STATUSES = ("pending", "approved", "suspended", "revoked")

SCHEMA = """
CREATE TABLE IF NOT EXISTS workers (
    pubkey TEXT PRIMARY KEY,
    label TEXT NOT NULL,
    tier TEXT NOT NULL CHECK (tier IN ('fleet', 'public')),
    status TEXT NOT NULL CHECK (status IN ('pending', 'approved', 'suspended', 'revoked')),
    status_reason TEXT NOT NULL DEFAULT '',
    payout_account TEXT,
    registered_at INTEGER NOT NULL,
    approved_at INTEGER,
    last_seen INTEGER,
    last_heartbeat INTEGER,
    gpus_json TEXT NOT NULL DEFAULT '[]',
    models_json TEXT NOT NULL DEFAULT '[]',
    max_concurrency INTEGER NOT NULL DEFAULT 1,
    version TEXT NOT NULL DEFAULT '',
    jobs_ok INTEGER NOT NULL DEFAULT 0,
    jobs_failed INTEGER NOT NULL DEFAULT 0,
    consecutive_failures INTEGER NOT NULL DEFAULT 0,
    canary_pass INTEGER NOT NULL DEFAULT 0,
    canary_fail INTEGER NOT NULL DEFAULT 0,
    consecutive_canary_fail INTEGER NOT NULL DEFAULT 0,
    last_canary_at INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS fleet_enrollment (
    pubkey TEXT PRIMARY KEY,
    label TEXT NOT NULL,
    added_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS app_keys (
    name TEXT PRIMARY KEY,
    key_sha256 TEXT NOT NULL UNIQUE,
    allow_public INTEGER NOT NULL DEFAULT 0,
    requests_per_minute INTEGER NOT NULL,
    daily_token_budget INTEGER NOT NULL,
    max_tokens_cap INTEGER NOT NULL,
    created_at INTEGER NOT NULL,
    revoked_at INTEGER
);
CREATE TABLE IF NOT EXISTS nonces (
    pubkey TEXT NOT NULL,
    nonce TEXT NOT NULL,
    expires_at INTEGER NOT NULL,
    PRIMARY KEY (pubkey, nonce)
);
CREATE TABLE IF NOT EXISTS challenges (
    challenge TEXT PRIMARY KEY,
    expires_at INTEGER NOT NULL,
    used_by TEXT
);
CREATE TABLE IF NOT EXISTS receipts (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    job_id TEXT NOT NULL UNIQUE,
    kind TEXT NOT NULL CHECK (kind IN ('job', 'canary')),
    app TEXT NOT NULL,
    worker TEXT NOT NULL,
    worker_tier TEXT NOT NULL,
    model TEXT NOT NULL,
    model_digest TEXT NOT NULL,
    request_sha256 TEXT NOT NULL,
    response_sha256 TEXT NOT NULL,
    worker_sig TEXT NOT NULL,
    prompt_tokens INTEGER NOT NULL,
    completion_tokens INTEGER NOT NULL,
    started_ms INTEGER NOT NULL,
    finished_ms INTEGER NOT NULL,
    outcome TEXT NOT NULL CHECK (outcome IN ('ok', 'canary_pass', 'canary_fail')),
    recorded_ms INTEGER NOT NULL,
    prev_hash TEXT NOT NULL,
    receipt_hash TEXT NOT NULL UNIQUE,
    coordinator_sig TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS receipts_app_time ON receipts (app, recorded_ms);
CREATE INDEX IF NOT EXISTS receipts_worker_time ON receipts (worker, recorded_ms);
CREATE TABLE IF NOT EXISTS events (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    ts INTEGER NOT NULL,
    kind TEXT NOT NULL,
    subject TEXT NOT NULL,
    detail_json TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS meta (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
"""

RECEIPT_FIELDS = ("job_id", "kind", "app", "worker", "worker_tier", "model", "model_digest", "request_sha256",
                  "response_sha256", "worker_sig", "prompt_tokens", "completion_tokens", "started_ms",
                  "finished_ms", "outcome", "recorded_ms", "prev_hash")


def receipt_hash(record: Dict[str, Any]) -> str:
    return sha256_hex(canonical({k: record[k] for k in RECEIPT_FIELDS}))


def utc_day_start_ms(ts_ms: Optional[int] = None) -> int:
    moment = datetime.fromtimestamp((ts_ms if ts_ms is not None else now_ms()) / 1000, tz=timezone.utc)
    return int(moment.replace(hour=0, minute=0, second=0, microsecond=0).timestamp() * 1000)


class Store:
    def __init__(self, path: str):
        self.path = path
        self._lock = threading.RLock()
        self._db = sqlite3.connect(path, check_same_thread=False, isolation_level=None, timeout=30)
        self._db.row_factory = sqlite3.Row
        self._db.execute("PRAGMA journal_mode=WAL")
        self._db.execute("PRAGMA foreign_keys=ON")
        self._db.execute("PRAGMA busy_timeout=30000")
        self._db.executescript(SCHEMA)

    def close(self) -> None:
        with self._lock:
            self._db.close()

    # ---------------------------------------------------------------- helpers

    def _tx(self):
        store = self

        class _Tx:
            def __enter__(self_inner):
                store._lock.acquire()
                store._db.execute("BEGIN IMMEDIATE")
                return store._db

            def __exit__(self_inner, exc_type, exc, tb):
                try:
                    store._db.execute("ROLLBACK" if exc_type else "COMMIT")
                finally:
                    store._lock.release()
                return False

        return _Tx()

    def _one(self, sql: str, args: Iterable[Any] = ()) -> Optional[sqlite3.Row]:
        with self._lock:
            return self._db.execute(sql, tuple(args)).fetchone()

    def _all(self, sql: str, args: Iterable[Any] = ()) -> List[sqlite3.Row]:
        with self._lock:
            return self._db.execute(sql, tuple(args)).fetchall()

    def log_event(self, kind: str, subject: str, detail: Optional[Dict[str, Any]] = None) -> None:
        with self._tx() as db:
            db.execute("INSERT INTO events (ts, kind, subject, detail_json) VALUES (?, ?, ?, ?)",
                       (now_s(), kind, subject, json.dumps(detail or {}, sort_keys=True)))

    def events(self, limit: int = 100) -> List[Dict[str, Any]]:
        rows = self._all("SELECT * FROM events ORDER BY seq DESC LIMIT ?", (limit,))
        return [{**dict(r), "detail": json.loads(r["detail_json"])} for r in rows]

    def meta_get(self, key: str) -> Optional[str]:
        row = self._one("SELECT value FROM meta WHERE key = ?", (key,))
        return row["value"] if row else None

    def meta_set(self, key: str, value: str) -> None:
        with self._tx() as db:
            db.execute("INSERT INTO meta (key, value) VALUES (?, ?) "
                       "ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                       (key, value))

    # ---------------------------------------------------------------- replay protection

    def consume_nonce(self, pubkey: str, nonce: str, ttl_s: int) -> None:
        now = now_s()
        with self._tx() as db:
            db.execute("DELETE FROM nonces WHERE expires_at < ?", (now,))
            try:
                db.execute("INSERT INTO nonces (pubkey, nonce, expires_at) VALUES (?, ?, ?)",
                           (pubkey, nonce, now + ttl_s))
            except sqlite3.IntegrityError:
                raise ProtocolError("nonce already used (replayed message)", status=401, code="replay") from None

    def new_challenge(self, ttl_s: int = 120, max_open: int = 2000) -> Dict[str, Any]:
        challenge = secrets.token_hex(32)
        now = now_s()
        with self._tx() as db:
            db.execute("DELETE FROM challenges WHERE expires_at < ?", (now,))
            open_count = db.execute("SELECT COUNT(*) AS n FROM challenges WHERE used_by IS NULL").fetchone()["n"]
            if open_count >= max_open:
                raise ProtocolError("too many open registration challenges; try again shortly", status=429,
                                    code="busy")
            db.execute("INSERT INTO challenges (challenge, expires_at) VALUES (?, ?)", (challenge, now + ttl_s))
        return {"challenge": challenge, "expires_at": now + ttl_s}

    def use_challenge(self, challenge: Any, pubkey: str) -> None:
        if not isinstance(challenge, str):
            raise ProtocolError("challenge required")
        with self._tx() as db:
            row = db.execute("SELECT * FROM challenges WHERE challenge = ?", (challenge,)).fetchone()
            if row is None:
                raise ProtocolError("unknown challenge; GET /mesh/challenge first", status=401, code="bad_challenge")
            if row["used_by"] is not None:
                raise ProtocolError("challenge already used", status=401, code="bad_challenge")
            if row["expires_at"] < now_s():
                raise ProtocolError("challenge expired", status=401, code="bad_challenge")
            db.execute("UPDATE challenges SET used_by = ? WHERE challenge = ?", (pubkey, challenge))

    # ---------------------------------------------------------------- contributors

    def enroll_fleet(self, pubkey: str, label: str) -> None:
        if not is_pubkey(pubkey):
            raise ProtocolError("public key must be 64 lowercase hex characters")
        with self._tx() as db:
            db.execute("INSERT INTO fleet_enrollment (pubkey, label, added_at) VALUES (?, ?, ?) "
                       "ON CONFLICT(pubkey) DO UPDATE SET label = excluded.label", (pubkey, label, now_s()))
            row = db.execute("SELECT status FROM workers WHERE pubkey = ?", (pubkey,)).fetchone()
            if row is not None and row["status"] == "pending":
                db.execute("UPDATE workers SET tier = 'fleet', status = 'approved', approved_at = ?, status_reason = "
                           "'fleet enrollment' WHERE pubkey = ?", (now_s(), pubkey))
        self.log_event("fleet_enrolled", pubkey, {"label": label})

    def is_fleet_enrolled(self, pubkey: str) -> bool:
        return self._one("SELECT 1 FROM fleet_enrollment WHERE pubkey = ?", (pubkey,)) is not None

    def register_worker(self, pubkey: str, *, label: str, payout_account: Optional[str], gpus: List[Dict[str, Any]],
                        models: List[Dict[str, str]], max_concurrency: int, version: str) -> Dict[str, Any]:
        now = now_s()
        fleet = self.is_fleet_enrolled(pubkey)
        with self._tx() as db:
            row = db.execute("SELECT * FROM workers WHERE pubkey = ?", (pubkey,)).fetchone()
            if row is not None and row["status"] == "revoked":
                raise ProtocolError("this contributor key is revoked", status=403, code="revoked")
            if row is None:
                tier, status = ("fleet", "approved") if fleet else ("public", "pending")
                db.execute(
                    "INSERT INTO workers (pubkey, label, tier, status, status_reason, payout_account, registered_at, "
                    "approved_at, last_seen, last_heartbeat, gpus_json, models_json, max_concurrency, version) "
                    "VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                    (pubkey, label, tier, status, "fleet enrollment" if fleet else "awaiting owner approval",
                     payout_account, now, now if fleet else None, now, now, json.dumps(gpus), json.dumps(models),
                     max_concurrency, version))
                created = True
            else:
                db.execute("UPDATE workers SET label = ?, payout_account = ?, last_seen = ?, last_heartbeat = ?, "
                           "gpus_json = ?, models_json = ?, max_concurrency = ?, version = ? WHERE pubkey = ?",
                           (label, payout_account, now, now, json.dumps(gpus), json.dumps(models), max_concurrency,
                            version, pubkey))
                created = False
        self.log_event("worker_registered" if created else "worker_reregistered", pubkey,
                       {"label": label, "models": [m["name"] for m in models]})
        return self.worker(pubkey)

    def heartbeat(self, pubkey: str, *, gpus: List[Dict[str, Any]], models: List[Dict[str, str]]) -> Dict[str, Any]:
        now = now_s()
        with self._tx() as db:
            cur = db.execute("UPDATE workers SET last_seen = ?, last_heartbeat = ?, gpus_json = ?, models_json = ? "
                             "WHERE pubkey = ? AND status != 'revoked'", (now, now, json.dumps(gpus),
                                                                          json.dumps(models), pubkey))
            if cur.rowcount == 0:
                raise ProtocolError("unknown or revoked contributor; register first", status=403, code="not_registered")
        return self.worker(pubkey)

    def touch(self, pubkey: str) -> None:
        with self._tx() as db:
            db.execute("UPDATE workers SET last_seen = ? WHERE pubkey = ?", (now_s(), pubkey))

    def worker(self, pubkey: str) -> Optional[Dict[str, Any]]:
        row = self._one("SELECT * FROM workers WHERE pubkey = ?", (pubkey,))
        return _worker_dict(row) if row else None

    def count_workers(self, status: str) -> int:
        return int(self._one("SELECT COUNT(*) AS n FROM workers WHERE status = ?", (status,))["n"])

    def workers(self, status: Optional[str] = None) -> List[Dict[str, Any]]:
        if status:
            rows = self._all("SELECT * FROM workers WHERE status = ? ORDER BY registered_at", (status,))
        else:
            rows = self._all("SELECT * FROM workers ORDER BY registered_at")
        return [_worker_dict(r) for r in rows]

    def set_status(self, pubkey: str, status: str, reason: str, *, actor: str = "owner") -> Dict[str, Any]:
        if status not in STATUSES:
            raise ProtocolError(f"status must be one of {', '.join(STATUSES)}")
        with self._tx() as db:
            row = db.execute("SELECT status FROM workers WHERE pubkey = ?", (pubkey,)).fetchone()
            if row is None:
                raise ProtocolError(f"no contributor {pubkey}", status=404, code="not_found")
            if row["status"] == "revoked" and status != "revoked":
                raise ProtocolError("a revoked contributor cannot be reinstated; it must register a new key",
                                    status=409, code="revoked")
            db.execute("UPDATE workers SET status = ?, status_reason = ?, approved_at = CASE WHEN ? = 'approved' "
                       "THEN ? ELSE approved_at END, consecutive_failures = CASE WHEN ? = 'approved' THEN 0 ELSE "
                       "consecutive_failures END, consecutive_canary_fail = CASE WHEN ? = 'approved' THEN 0 ELSE "
                       "consecutive_canary_fail END WHERE pubkey = ?",
                       (status, reason, status, now_s(), status, status, pubkey))
        self.log_event(f"worker_{status}", pubkey, {"reason": reason, "actor": actor})
        return self.worker(pubkey)

    def set_tier(self, pubkey: str, tier: str) -> Dict[str, Any]:
        if tier not in TIERS:
            raise ProtocolError(f"tier must be one of {', '.join(TIERS)}")
        with self._tx() as db:
            cur = db.execute("UPDATE workers SET tier = ? WHERE pubkey = ?", (tier, pubkey))
            if cur.rowcount == 0:
                raise ProtocolError(f"no contributor {pubkey}", status=404, code="not_found")
        self.log_event("worker_tier", pubkey, {"tier": tier})
        return self.worker(pubkey)

    def record_job_outcome(self, pubkey: str, ok: bool) -> Dict[str, Any]:
        with self._tx() as db:
            if ok:
                db.execute("UPDATE workers SET jobs_ok = jobs_ok + 1, consecutive_failures = 0 WHERE pubkey = ?",
                           (pubkey,))
            else:
                db.execute("UPDATE workers SET jobs_failed = jobs_failed + 1, consecutive_failures = "
                           "consecutive_failures + 1 WHERE pubkey = ?", (pubkey,))
        return self.worker(pubkey)

    def record_canary(self, pubkey: str, passed: bool) -> Dict[str, Any]:
        with self._tx() as db:
            if passed:
                db.execute("UPDATE workers SET canary_pass = canary_pass + 1, consecutive_canary_fail = 0 "
                           "WHERE pubkey = ?", (pubkey,))
            else:
                db.execute("UPDATE workers SET canary_fail = canary_fail + 1, consecutive_canary_fail = "
                           "consecutive_canary_fail + 1 WHERE pubkey = ?", (pubkey,))
        return self.worker(pubkey)

    def mark_canary_sent(self, pubkey: str) -> None:
        with self._tx() as db:
            db.execute("UPDATE workers SET last_canary_at = ? WHERE pubkey = ?", (now_s(), pubkey))

    # ---------------------------------------------------------------- app keys

    def add_app_key(self, name: str, *, allow_public: bool, requests_per_minute: int, daily_token_budget: int,
                    max_tokens_cap: int) -> str:
        if not (isinstance(name, str) and 0 < len(name) <= 48 and name.replace("-", "").replace("_", "").isalnum()):
            raise ProtocolError("app name must be 1–48 letters, digits, - or _")
        for label, value, hi in (("requests_per_minute", requests_per_minute, 100_000),
                                 ("daily_token_budget", daily_token_budget, 10_000_000_000),
                                 ("max_tokens_cap", max_tokens_cap, 8192)):
            if isinstance(value, bool) or not isinstance(value, int) or not 1 <= value <= hi:
                raise ProtocolError(f"{label} must be between 1 and {hi}")
        key = "x3m_" + secrets.token_urlsafe(32)
        with self._tx() as db:
            existing = db.execute("SELECT revoked_at FROM app_keys WHERE name = ?", (name,)).fetchone()
            if existing is not None and existing["revoked_at"] is None:
                raise ProtocolError(f"app {name!r} already has an active key; revoke it first", status=409,
                                    code="exists")
            db.execute("DELETE FROM app_keys WHERE name = ?", (name,))
            db.execute("INSERT INTO app_keys (name, key_sha256, allow_public, requests_per_minute, daily_token_budget, "
                       "max_tokens_cap, created_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
                       (name, sha256_hex(key.encode()), int(allow_public), requests_per_minute, daily_token_budget,
                        max_tokens_cap, now_s()))
        self.log_event("app_key_created", name, {"allow_public": allow_public, "requests_per_minute":
                                                 requests_per_minute, "daily_token_budget": daily_token_budget,
                                                 "max_tokens_cap": max_tokens_cap})
        return key

    def revoke_app_key(self, name: str) -> None:
        with self._tx() as db:
            cur = db.execute("UPDATE app_keys SET revoked_at = ? WHERE name = ? AND revoked_at IS NULL",
                             (now_s(), name))
            if cur.rowcount == 0:
                raise ProtocolError(f"no active key for app {name!r}", status=404, code="not_found")
        self.log_event("app_key_revoked", name, {})

    def app_for_key(self, presented: str) -> Optional[Dict[str, Any]]:
        if not isinstance(presented, str) or not presented.startswith("x3m_") or len(presented) > 128:
            return None
        digest = sha256_hex(presented.encode())
        row = self._one("SELECT * FROM app_keys WHERE key_sha256 = ? AND revoked_at IS NULL", (digest,))
        if row is None or not hmac.compare_digest(row["key_sha256"], digest):
            return None
        return dict(row)

    def apps(self) -> List[Dict[str, Any]]:
        rows = self._all("SELECT name, allow_public, requests_per_minute, daily_token_budget, max_tokens_cap, "
                         "created_at, revoked_at FROM app_keys ORDER BY name")
        return [dict(r) for r in rows]

    def tokens_used_today(self, app: str) -> int:
        row = self._one("SELECT COALESCE(SUM(prompt_tokens + completion_tokens), 0) AS used FROM receipts "
                        "WHERE app = ? AND kind = 'job' AND recorded_ms >= ?", (app, utc_day_start_ms()))
        return int(row["used"])

    # ---------------------------------------------------------------- receipts

    def append_receipt(self, record: Dict[str, Any], sign) -> Dict[str, Any]:
        """Append a receipt. ``sign(receipt_hash_hex) -> sig_hex`` is the coordinator's key."""
        with self._tx() as db:
            last = db.execute("SELECT receipt_hash FROM receipts ORDER BY seq DESC LIMIT 1").fetchone()
            full = dict(record)
            full["recorded_ms"] = now_ms()
            full["prev_hash"] = last["receipt_hash"] if last else GENESIS
            full["receipt_hash"] = receipt_hash(full)
            full["coordinator_sig"] = sign(full["receipt_hash"])
            columns = list(RECEIPT_FIELDS) + ["receipt_hash", "coordinator_sig"]
            db.execute(f"INSERT INTO receipts ({', '.join(columns)}) VALUES ({', '.join('?' for _ in columns)})",
                       tuple(full[c] for c in columns))
            seq = db.execute("SELECT last_insert_rowid() AS seq").fetchone()["seq"]
        full["seq"] = seq
        return full

    def receipts(self, *, since_ms: int = 0, until_ms: Optional[int] = None, worker: Optional[str] = None,
                 app: Optional[str] = None) -> List[Dict[str, Any]]:
        sql = "SELECT * FROM receipts WHERE recorded_ms >= ?"
        args: List[Any] = [since_ms]
        if until_ms is not None:
            sql += " AND recorded_ms < ?"
            args.append(until_ms)
        if worker:
            sql += " AND worker = ?"
            args.append(worker)
        if app:
            sql += " AND app = ?"
            args.append(app)
        return [dict(r) for r in self._all(sql + " ORDER BY seq", args)]

    def verify_receipts(self, coordinator_pubkey: str) -> Dict[str, Any]:
        """Recompute the hash chain and every coordinator and worker signature."""
        prev = GENESIS
        count = 0
        for row in self._all("SELECT * FROM receipts ORDER BY seq"):
            record = dict(row)
            if record["prev_hash"] != prev:
                return {"ok": False, "seq": record["seq"], "error": "chain broken: prev_hash does not match"}
            if receipt_hash(record) != record["receipt_hash"]:
                return {"ok": False, "seq": record["seq"], "error": "receipt contents do not match receipt_hash"}
            if not verify_signature(coordinator_pubkey, record["receipt_hash"].encode(), record["coordinator_sig"]):
                return {"ok": False, "seq": record["seq"], "error": "bad coordinator signature"}
            if not verify_signature(record["worker"], worker_result_message(record).encode(), record["worker_sig"]):
                return {"ok": False, "seq": record["seq"], "error": "bad worker signature"}
            prev = record["receipt_hash"]
            count += 1
        return {"ok": True, "receipts": count, "head": prev}


def worker_result_message(record: Dict[str, Any]) -> str:
    """The exact string a worker signs to attest a result (also stored in receipts)."""
    return ":".join([
        "x3.gpu-mesh.v1.result", record["job_id"], record["request_sha256"], record["response_sha256"],
        record["model"], record["model_digest"],
    ])


def _worker_dict(row: sqlite3.Row) -> Dict[str, Any]:
    data = dict(row)
    data["gpus"] = json.loads(data.pop("gpus_json"))
    data["models"] = json.loads(data.pop("models_json"))
    return data


def parse_time_ms(value: Optional[str]) -> Optional[int]:
    """Accept YYYY-MM-DD (UTC) or an ISO-8601 timestamp."""
    if value is None:
        return None
    try:
        if len(value) == 10:
            moment = datetime.strptime(value, "%Y-%m-%d").replace(tzinfo=timezone.utc)
        else:
            moment = datetime.fromisoformat(value.replace("Z", "+00:00"))
            if moment.tzinfo is None:
                moment = moment.replace(tzinfo=timezone.utc)
    except ValueError as exc:
        raise ProtocolError(f"bad time {value!r}: use YYYY-MM-DD or ISO-8601") from exc
    return int(moment.timestamp() * 1000)
