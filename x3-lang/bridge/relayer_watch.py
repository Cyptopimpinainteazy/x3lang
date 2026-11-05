"""Crash-persistent, read-only watch scheduling for the experimental NEAR bridge.

Never signs, certifies finality, submits transactions, or infers funding from
missing data. Observations must already be independently inspected elsewhere.
"""
from __future__ import annotations

import re
import sqlite3
from dataclasses import dataclass
from pathlib import Path


class WatchError(ValueError):
    pass


@dataclass(frozen=True)
class Watch:
    network: str
    vault: str
    swap_id: str
    deadline_sec: int
    phase: str
    last_outcome: str | None
    block_hash: str | None
    authorized_to_sign: bool = False


class WatchJournal:
    def __init__(self, path: str | Path):
        self.path = str(path)
        with self._db() as db:
            db.execute("""CREATE TABLE IF NOT EXISTS watches (
                network TEXT NOT NULL, vault TEXT NOT NULL, swap_id TEXT NOT NULL,
                deadline_sec INTEGER NOT NULL, phase TEXT NOT NULL,
                last_outcome TEXT, block_hash TEXT,
                lease_owner TEXT, lease_until INTEGER NOT NULL DEFAULT 0,
                retry_at INTEGER NOT NULL DEFAULT 0, failure_reason TEXT,
                PRIMARY KEY(network,vault,swap_id))""")

    def _db(self):
        db = sqlite3.connect(self.path, timeout=10)
        db.execute("PRAGMA busy_timeout=10000")
        return db

    @staticmethod
    def _validate(network: str, vault: str, swap_id: str):
        if network != "testnet":
            raise WatchError("read-only pilot restricted to testnet")
        if not isinstance(vault, str) or not re.fullmatch(r"[a-z0-9_-]+(?:\.[a-z0-9_-]+)*", vault):
            raise WatchError("invalid NEAR vault")
        if not isinstance(swap_id, str) or not re.fullmatch(r"[0-9a-f]{64}", swap_id):
            raise WatchError("invalid swap identifier")

    @staticmethod
    def _row(row) -> Watch:
        return Watch(*row)

    def enqueue(self, network: str, vault: str, swap_id: str, *, deadline_sec: int) -> bool:
        self._validate(network, vault, swap_id)
        if type(deadline_sec) is not int or deadline_sec <= 0:
            raise WatchError("invalid deadline")
        with self._db() as db:
            db.execute("BEGIN IMMEDIATE")
            old = db.execute("SELECT deadline_sec FROM watches WHERE network=? AND vault=? AND swap_id=?",
                             (network, vault, swap_id)).fetchone()
            if old is not None:
                if old[0] != deadline_sec:
                    raise WatchError("conflicting registration")
                return False
            db.execute("INSERT INTO watches(network,vault,swap_id,deadline_sec,phase) VALUES(?,?,?,?,'PENDING')",
                       (network, vault, swap_id, deadline_sec))
            return True

    def lease(self, owner: str, *, now: int, lease_sec: int = 60, limit: int = 20) -> list[Watch]:
        if (not isinstance(owner, str) or not owner.strip() or type(now) is not int or now < 0
                or type(lease_sec) is not int or not 1 <= lease_sec <= 300
                or type(limit) is not int or not 1 <= limit <= 100):
            raise WatchError("invalid lease parameters")
        with self._db() as db:
            db.execute("BEGIN IMMEDIATE")
            rows = db.execute("""SELECT network,vault,swap_id,deadline_sec,phase,last_outcome,block_hash,0
                FROM watches WHERE phase='PENDING' AND retry_at<=? AND lease_until<=?
                ORDER BY retry_at,network,vault,swap_id LIMIT ?""", (now, now, limit)).fetchall()
            for row in rows:
                db.execute("""UPDATE watches SET lease_owner=?,lease_until=?
                    WHERE network=? AND vault=? AND swap_id=?""", (owner, now+lease_sec, *row[:3]))
            return [self._row(row) for row in rows]

    def _owned(self, db, owner, network, vault, swap_id, now):
        self._validate(network, vault, swap_id)
        row = db.execute("""SELECT phase,lease_owner,lease_until,last_outcome,block_hash FROM watches
            WHERE network=? AND vault=? AND swap_id=?""", (network,vault,swap_id)).fetchone()
        if (row is None or row[0] != "PENDING" or row[1] != owner or row[2] <= now):
            raise WatchError("expired or unauthorized lease")
        return row

    def record_observation(self, owner, network, vault, swap_id, *, state, block_hash, now):
        # A local RPC observation is NEVER an attestation or a settlement proof.
        if state not in ("Claimed", "Refunded") or not isinstance(block_hash,str) or not block_hash:
            raise WatchError("only explicit observed terminal states are recordable")
        with self._db() as db:
            db.execute("BEGIN IMMEDIATE")
            existing = db.execute("""SELECT last_outcome,block_hash FROM watches
                WHERE network=? AND vault=? AND swap_id=?""", (network,vault,swap_id)).fetchone()
            if existing and existing[0] is not None:
                if existing != (state,block_hash):
                    raise WatchError("conflicting terminal observation")
                return False
            self._owned(db, owner, network, vault, swap_id, now)
            db.execute("""UPDATE watches SET phase='REVIEW_REQUIRED',last_outcome=?,block_hash=?,
                lease_owner=NULL,lease_until=0 WHERE network=? AND vault=? AND swap_id=?""",
                       (state,block_hash,network,vault,swap_id))
            return True

    def fail(self, owner, network, vault, swap_id, *, reason, now, retry_after_sec=30):
        if (not isinstance(reason,str) or not reason.strip() or len(reason) > 200
                or type(retry_after_sec) is not int or not 1 <= retry_after_sec <= 86400):
            raise WatchError("invalid retry metadata")
        with self._db() as db:
            db.execute("BEGIN IMMEDIATE")
            self._owned(db, owner, network, vault, swap_id, now)
            db.execute("""UPDATE watches SET retry_at=?,failure_reason=?,lease_owner=NULL,lease_until=0
                WHERE network=? AND vault=? AND swap_id=?""",
                       (now+retry_after_sec,reason,network,vault,swap_id))

    def get(self, network: str, vault: str, swap_id: str) -> Watch | None:
        self._validate(network,vault,swap_id)
        with self._db() as db:
            row = db.execute("""SELECT network,vault,swap_id,deadline_sec,phase,last_outcome,block_hash,0
                FROM watches WHERE network=? AND vault=? AND swap_id=?""",(network,vault,swap_id)).fetchone()
            return self._row(row) if row else None
