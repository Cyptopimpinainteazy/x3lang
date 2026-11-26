"""Durable, local-only holds against an operator's opening treasury inventory."""
from contextlib import closing, contextmanager
import json
from pathlib import Path
import re
import sqlite3
import time

from . import CoinError, U64_MAX, _fields, _uint
from .sale import network_id, plan_sale
from . import delivery as sale_delivery


def _asset(value):
    if not isinstance(value, str) or not re.fullmatch(r"[0-9a-f]{64}", value):
        raise CoinError("asset_id must be a 64-character lowercase hex identifier")
    return value


def _identifier(value, field):
    if not isinstance(value, str) or not re.fullmatch(r"[a-zA-Z0-9][a-zA-Z0-9:._-]{0,127}", value):
        raise CoinError(f"{field} requires a 1..128 character identifier")
    return value


def _opening_config(config):
    _fields(config, ("asset_id", "decimals", "total_inventory", "networks"), "opening inventory")
    _asset(config["asset_id"])
    decimals = config["decimals"]
    if type(decimals) is not int or not 0 <= decimals <= 18:
        raise CoinError("decimals must be an integer from 0 to 18")
    total = _uint(config["total_inventory"], "total_inventory", U64_MAX, 1)
    if not isinstance(config["networks"], list) or not config["networks"]:
        raise CoinError("a nonempty list of participating networks is required")
    opening = {}
    for network in config["networks"]:
        _fields(network, ("id", "opening"), "network")
        name = network_id(network["id"])
        if name in opening:
            raise CoinError("duplicate network in opening inventory")
        opening[name] = _uint(network["opening"], "opening", U64_MAX)
    if sum(opening.values()) != total:
        raise CoinError("opening balances must sum to total_inventory")
    return {**config, "networks": [{"id": n, "opening": str(opening[n])} for n in sorted(opening)]}


def _request(request):
    _fields(request, ("asset_id", "order_id", "buyer", "destination", "amount", "expires_at"), "reservation")
    _asset(request["asset_id"])
    _identifier(request["order_id"], "order_id")
    _identifier(request["buyer"], "buyer")
    network_id(request["destination"])
    _uint(request["amount"], "amount", U64_MAX, 1)
    expiry = request["expires_at"]
    if type(expiry) is not int or not 0 < expiry <= 2**63 - 1:
        raise CoinError("expires_at must be a positive Unix timestamp in seconds")


_SCHEMA = (
    "CREATE TABLE sale_inventory (singleton INTEGER PRIMARY KEY CHECK(singleton=1), config TEXT NOT NULL)",
    """CREATE TABLE sale_holds (
        order_id TEXT PRIMARY KEY NOT NULL, terms TEXT NOT NULL,
        destination TEXT NOT NULL, amount TEXT NOT NULL, charge TEXT NOT NULL,
        expires_at INTEGER NOT NULL, status TEXT NOT NULL CHECK(status IN ('reserved','expired','released'))
    )""",
    "CREATE INDEX open_sale_holds ON sale_holds(expires_at) WHERE status='reserved'",
)


class ReservationBook:
    """One asset and one immutable opening allocation per local SQLite file.

    Only live holds reduce availability. A hold is a single row claiming an
    equal quota at each member and the full delivery amount at its destination.
    The two views describe the same inventory; they are never added together.
    """

    def __init__(self, path, *, clock=time.time):
        self.path = Path(path)
        self.clock = clock

    @contextmanager
    def _transaction(self, *, create=False):
        uri = self.path.resolve().as_uri() + ("?mode=rwc" if create else "?mode=rw")
        try:
            conn = sqlite3.connect(uri, uri=True, timeout=10, isolation_level=None)
        except sqlite3.OperationalError as error:
            raise CoinError("cannot open reservation book at the requested path") from error
        with closing(conn) as db, db:
            db.row_factory = sqlite3.Row
            db.execute("PRAGMA synchronous=FULL")
            db.execute("BEGIN IMMEDIATE")
            exists = db.execute("SELECT 1 FROM sqlite_master WHERE type='table' AND name='sale_inventory'").fetchone()
            if not exists:
                if not create or db.execute("SELECT 1 FROM sqlite_master LIMIT 1").fetchone():
                    raise CoinError("path does not contain an initialized reservation book")
                for statement in _SCHEMA:
                    db.execute(statement)
            # Upgrade existing books inside the writer lock before the expiry sweep.
            # The trigger protects prepared holds even when an older CLI opens the file.
            for statement in sale_delivery.SCHEMA:
                db.execute(statement)
            # Observe the operator's clock after acquiring the lock. A terminal
            # state stays terminal even if a subsequent clock reading moves back.
            now = int(self.clock())
            db.execute("UPDATE sale_holds SET status='expired' WHERE status='reserved' AND expires_at<=?", (now,))
            # An ordinary refusal must not undo elapsed holds if the host clock
            # later moves backwards. The savepoint rolls back only the requested
            # operation. Unexpected database errors still abort the transaction.
            db.execute("SAVEPOINT requested_operation")
            try:
                yield db, now
            except CoinError:
                db.execute("ROLLBACK TO requested_operation")
                db.execute("RELEASE requested_operation")
                db.commit()
                raise
            db.execute("RELEASE requested_operation")

    def initialize(self, config):
        config = _opening_config(config)
        canonical = json.dumps(config, sort_keys=True, separators=(",", ":"))
        with self._transaction(create=True) as (db, _):
            first = db.execute("SELECT config FROM sale_inventory WHERE singleton=1").fetchone()
            if first is None:
                db.execute("INSERT INTO sale_inventory VALUES(1, ?)", (canonical,))
            elif first["config"] != canonical:
                raise CoinError("reservation book already has a different opening inventory")
            return self._inventory(db, config)

    @staticmethod
    def _config(db, asset_id):
        _asset(asset_id)
        config = json.loads(db.execute("SELECT config FROM sale_inventory WHERE singleton=1").fetchone()["config"])
        if config["asset_id"] != asset_id:
            raise CoinError("asset_id differs from the book's opening inventory")
        return config

    @staticmethod
    def _inventory(db, config):
        quota_held = 0
        float_held = {}
        # SQL INTEGER and SUM cannot safely represent the full token u64 range.
        for row in db.execute("SELECT destination, amount, charge FROM sale_holds WHERE status='reserved'"):
            quota_held += int(row["charge"])
            destination = row["destination"]
            float_held[destination] = float_held.get(destination, 0) + int(row["amount"])
        return {
            **config,
            "networks": [
                {**n, "quota_available": str(int(n["opening"]) - quota_held),
                 "float_available": str(int(n["opening"]) - float_held.get(n["id"], 0))}
                for n in config["networks"]
            ],
        }

    def inventory(self, asset_id):
        with self._transaction() as (db, _):
            return self._inventory(db, self._config(db, asset_id))

    @staticmethod
    def _receipt(db, config, order_id):
        row = db.execute("SELECT * FROM sale_holds WHERE order_id=?", (order_id,)).fetchone()
        if row is None:
            raise CoinError("unknown reservation order_id")
        delivery = db.execute("SELECT evidence FROM sale_deliveries WHERE order_id=?", (order_id,)).fetchone()
        status = row["status"] if delivery is None else ("delivered" if delivery["evidence"] else "awaiting_confirmation")
        return {
            **json.loads(row["terms"]),
            "status": status,
            "scope": "local_inventory_hold" if delivery is None else "evm_sale_delivery",
            "shares": [{"network": n["id"], "charge": row["charge"]} for n in config["networks"]],
        }

    def reserve(self, request):
        _request(request)
        terms = json.dumps(request, sort_keys=True, separators=(",", ":"))
        order_id = request["order_id"]
        with self._transaction() as (db, now):
            config = self._config(db, request["asset_id"])
            previous = db.execute("SELECT terms FROM sale_holds WHERE order_id=?", (order_id,)).fetchone()
            if previous is not None:
                if previous["terms"] != terms:
                    raise CoinError("order_id already binds different reservation terms")
                return self._receipt(db, config, order_id)
            if request["expires_at"] <= now:
                raise CoinError("reservation expiry must be in the future")
            networks = self._inventory(db, config)["networks"]
            destination = request["destination"]
            floats = {n["id"]: n["float_available"] for n in networks}
            if destination not in floats:
                raise CoinError("destination must be one of the participating networks")
            quote = plan_sale({
                "decimals": config["decimals"], "destination": destination, "amount": request["amount"],
                "networks": [{"id": n["id"], "available": n["quota_available"]} for n in networks],
                "destination_float": floats[destination],
            })
            db.execute(
                """INSERT INTO sale_holds(order_id,terms,destination,amount,charge,expires_at,status)
                   VALUES(?,?,?,?,?,?,'reserved')""",
                (order_id, terms, destination, quote["amount"], quote["destination_local_share"], request["expires_at"]),
            )
            return self._receipt(db, config, order_id)

    def get(self, asset_id, order_id):
        _identifier(order_id, "order_id")
        with self._transaction() as (db, _):
            return self._receipt(db, self._config(db, asset_id), order_id)

    def release(self, asset_id, order_id):
        _identifier(order_id, "order_id")
        with self._transaction() as (db, _):
            config = self._config(db, asset_id)
            if db.execute("SELECT 1 FROM sale_deliveries WHERE order_id=?", (order_id,)).fetchone():
                raise CoinError("prepared or delivered purchases cannot release their inventory")
            db.execute("UPDATE sale_holds SET status='released' WHERE order_id=? AND status='reserved'", (order_id,))
            return self._receipt(db, config, order_id)

    def prepare_delivery(self, asset_id, order_id, policy):
        """Persist the purchase identity and freeze its hold before returning wallet calls."""
        _identifier(order_id, "order_id")
        with self._transaction() as (db, _):
            config = self._config(db, asset_id)
            receipt = self._receipt(db, config, order_id)
            return sale_delivery.prepare(db, receipt, config["decimals"], policy)

    def delivery(self, asset_id, order_id):
        _identifier(order_id, "order_id")
        with self._transaction() as (db, _):
            self._config(db, asset_id)
            return sale_delivery.read_delivery(db, order_id)

    def reconcile_delivery(self, asset_id, order_id, rpcs):
        """Discover the original purchase after a restart. Never submit a transfer."""
        from .delivery_evidence import confirmed_fill

        previous = self.delivery(asset_id, order_id)
        if previous["evidence"] is not None:
            return previous
        # No network calls while holding the database writer lock. The order
        # terms cannot change between preparation, observation and commit.
        observed = confirmed_fill(previous["terms"], rpcs)
        if observed is None:
            return self.delivery(asset_id, order_id)
        with self._transaction() as (db, _):
            config = self._config(db, asset_id)
            return sale_delivery.record_confirmation(db, config, order_id, observed)
