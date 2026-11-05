"""X3-native LP quotation and reservation ledger (NO custody or settlement).

Only independently reconciled, executable inventory may be offered.
SQLite transactions prevent local oversubscription; remote wallets, bridges,
and other processes require separate source-of-truth reconciliation.
"""
from __future__ import annotations

import sqlite3
import time
from dataclasses import dataclass
from pathlib import Path


class LiquidityError(ValueError):
    pass


@dataclass(frozen=True)
class Quote:
    provider_id: str
    source_asset: str
    destination_asset: str
    source_amount: int
    destination_amount: int
    fee_amount: int
    expires_at: int


class LiquidityBook:
    def __init__(self, path: str | Path):
        self.path = str(path)
        with self._db() as db:
            db.executescript("""
            CREATE TABLE IF NOT EXISTS providers (
                id TEXT PRIMARY KEY, bsc_address TEXT NOT NULL,
                near_account TEXT NOT NULL, authorized INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE IF NOT EXISTS offers (
                id TEXT PRIMARY KEY, provider_id TEXT NOT NULL,
                source_asset TEXT NOT NULL, destination_asset TEXT NOT NULL,
                price_num INTEGER NOT NULL CHECK(price_num>0),
                price_den INTEGER NOT NULL CHECK(price_den>0),
                fee_bps INTEGER NOT NULL CHECK(fee_bps BETWEEN 0 AND 10000),
                verified_inventory INTEGER NOT NULL DEFAULT 0 CHECK(verified_inventory>=0),
                max_input INTEGER NOT NULL CHECK(max_input>0),
                expires_at INTEGER NOT NULL,
                FOREIGN KEY(provider_id) REFERENCES providers(id)
            );
            CREATE TABLE IF NOT EXISTS reservations (
                id TEXT PRIMARY KEY, offer_id TEXT NOT NULL,
                amount_out INTEGER NOT NULL CHECK(amount_out>0),
                expires_at INTEGER NOT NULL,
                state TEXT NOT NULL CHECK(state IN ('reserved','released','settled')),
                FOREIGN KEY(offer_id) REFERENCES offers(id)
            );
            """)

    def _db(self):
        db = sqlite3.connect(self.path, timeout=10)
        db.execute("PRAGMA foreign_keys=ON")
        db.execute("PRAGMA busy_timeout=10000")
        return db

    def register(self, provider_id: str, bsc_address: str, near_account: str):
        if not provider_id.strip() or not near_account.strip():
            raise LiquidityError("provider ID and NEAR account required")
        if len(bsc_address) != 42 or not bsc_address.startswith("0x"):
            raise LiquidityError("expected a 20-byte hex BSC address")
        try:
            int(bsc_address[2:], 16)
        except ValueError as exc:
            raise LiquidityError("invalid BSC address") from exc
        with self._db() as db:
            db.execute("INSERT INTO providers(id,bsc_address,near_account) VALUES(?,?,?)",
                       (provider_id, bsc_address, near_account))

    def configure_offer(self, offer_id: str, provider_id: str, *, source_asset: str,
                        destination_asset: str, price_num: int, price_den: int,
                        fee_bps: int, max_input: int, expires_at: int):
        if not source_asset or not destination_asset or price_num <= 0 or price_den <= 0:
            raise LiquidityError("invalid assets or integer price ratio")
        if not 0 <= fee_bps <= 10000 or max_input <= 0 or expires_at <= int(time.time()):
            raise LiquidityError("invalid fee, limit, or expiry")
        with self._db() as db:
            db.execute("""INSERT INTO offers
                (id,provider_id,source_asset,destination_asset,price_num,price_den,
                 fee_bps,max_input,expires_at) VALUES(?,?,?,?,?,?,?,?,?)""",
                       (offer_id, provider_id, source_asset, destination_asset,
                        price_num, price_den, fee_bps, max_input, expires_at))

    def mark_authorized(self, provider_id: str, *, verified: bool):
        """Operator action, NOT proof of a signed wallet authorization."""
        if verified:
            raise LiquidityError("authorization must be verified by a chain/wallet adapter")
        with self._db() as db:
            db.execute("UPDATE providers SET authorized=0 WHERE id=?", (provider_id,))

    def reserve(self, reservation_id: str, offer_id: str, amount_in: int,
                *, now: int | None = None, ttl_seconds: int = 120) -> Quote:
        now = int(time.time()) if now is None else now
        if amount_in <= 0 or ttl_seconds <= 0 or ttl_seconds > 3600:
            raise LiquidityError("invalid input or reservation TTL")
        db = self._db()
        try:
            db.execute("BEGIN IMMEDIATE")
            o = db.execute("""SELECT o.provider_id,o.source_asset,o.destination_asset,
                    o.price_num,o.price_den,o.fee_bps,o.max_input,o.expires_at,
                    o.verified_inventory,p.authorized
                    FROM offers o JOIN providers p ON p.id=o.provider_id WHERE o.id=?""",
                           (offer_id,)).fetchone()
            if not o:
                raise LiquidityError("offer not found")
            pid, src, dst, num, den, fee_bps, max_input, deadline, inventory, authorized = o
            if not authorized:
                raise LiquidityError("LP authorization not verified; no executable offers")
            if deadline <= now or amount_in > max_input:
                raise LiquidityError("offer expired or amount exceeds cap")
            gross = amount_in * num // den
            fee = (gross * fee_bps + 9999) // 10000
            amount_out = gross - fee
            if amount_out <= 0:
                raise LiquidityError("output too small")
            used = db.execute("""SELECT COALESCE(SUM(amount_out),0) FROM reservations
                WHERE offer_id=? AND (state='settled' OR (state='reserved' AND expires_at>?))""",
                              (offer_id, now)).fetchone()[0]
            if amount_out > inventory - used:
                raise LiquidityError("insufficient verified and unreserved destination liquidity")
            expiry = min(deadline, now + ttl_seconds)
            db.execute("""INSERT INTO reservations(id,offer_id,amount_out,expires_at,state)
                VALUES(?,?,?,?,'reserved')""", (reservation_id, offer_id, amount_out, expiry))
            db.commit()
            return Quote(pid, src, dst, amount_in, amount_out, fee, expiry)
        except Exception:
            db.rollback()
            raise
        finally:
            db.close()

    def release(self, reservation_id: str):
        """Cancel an unconsumed reservation; settlement marking needs chain evidence elsewhere."""
        with self._db() as db:
            row = db.execute("SELECT state FROM reservations WHERE id=?", (reservation_id,)).fetchone()
            if not row or row[0] != "reserved":
                raise LiquidityError("reservation not releasable")
            db.execute("UPDATE reservations SET state='released' WHERE id=?", (reservation_id,))


__all__ = ["LiquidityBook", "LiquidityError", "Quote"]
