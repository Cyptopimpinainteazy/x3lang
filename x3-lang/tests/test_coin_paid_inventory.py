"""Accounting across preparation, failures, older reservation clients and restart."""
from concurrent.futures import ThreadPoolExecutor
import json
from pathlib import Path
import sqlite3
from threading import Barrier

import pytest

from coin import CoinError, delivery_evidence
from coin.reservations import ReservationBook

FIXTURE = Path(__file__).resolve().parents[1] / "coin/examples/ten_chain_inventory.json"
OPENING = json.loads(FIXTURE.read_text())
ASSET = OPENING["asset_id"]
POLICY = {
    "chain_id": 31337, "genesis_hash": "0x" + "aa"*32, "sale_contract": "0x" + "11"*20,
    "sale_code_hash": "0x" + "22"*32, "token_contract": "0x" + "33"*20, "token_code_hash": "0x" + "44"*32,
    "treasury": "0x" + "55"*20, "from_block": 25, "destination": "ethereum", "payment_wei": "100",
}
REQUEST = {"asset_id": ASSET, "order_id": "A", "amount": "100000000", "destination": "ethereum",
           "buyer": "0x" + "66"*20, "expires_at": 100}
PROJECTION = {"tx_hash": "0x" + "ab"*32, "block_hash": "0x" + "bc"*32, "block_number": 50}


def seed(path):
    book = ReservationBook(path, clock=lambda: 80)
    book.initialize(OPENING)
    book.reserve(REQUEST)
    return book


def test_upgrade_preserves_existing_holds_and_protects_paid_orders_from_old_clients(tmp_path):
    path = tmp_path / "book.sqlite"
    book = seed(path)
    book.reserve({**REQUEST, "order_id": "B"})
    # Recreate the schema on #289 from a book with two outstanding holds.
    with sqlite3.connect(path) as db:
        db.executescript("""
            DROP TRIGGER sale_keep_delivery_hold;
            DROP TABLE sale_obligations;
            DROP TABLE sale_deliveries;
            DROP TABLE sale_delivery_routes;
        """)
    prepared = book.prepare_delivery(ASSET, "A", POLICY)
    assert book.prepare_delivery(ASSET, "A", POLICY) == prepared
    assert prepared["wallet_calls"]["buy"]["data"].endswith(prepared["terms"]["order_key"][2:])
    assert prepared["status"] == "awaiting_confirmation"
    # The #289 CLI executes precisely these writes. It can still expire ordinary holds.
    with sqlite3.connect(path) as legacy:
        legacy.execute("UPDATE sale_holds SET status='expired' WHERE status='reserved' AND expires_at<=?", (101,))
        legacy.execute("UPDATE sale_holds SET status='released' WHERE order_id=? AND status='reserved'", ("A",))
        assert dict(legacy.execute("SELECT order_id,status FROM sale_holds")) == {"A": "reserved", "B": "expired"}
    reopened = ReservationBook(path, clock=lambda: 120)
    assert reopened.reserve(REQUEST)["status"] == "awaiting_confirmation"
    assert reopened.delivery(ASSET, "A") == prepared
    with pytest.raises(CoinError, match="cannot release"):
        reopened.release(ASSET, "A")
    assert reopened.get(ASSET, "B")["status"] == "expired"


@pytest.mark.parametrize("changed", [
    {"payment_wei": "101"}, {"genesis_hash": "0x" + "bb"*32}, {"from_block": 50},
    {"token_contract": "0x" + "bb"*20}, {"treasury": "0x" + "bb"*20},
    {"token_code_hash": "0x" + "bb"*32}, {"sale_code_hash": "0x" + "bb"*32},
    {"chain_id": 1}, {"sale_contract": POLICY["treasury"]}, {"payment_wei": "0"},
])
def test_prepared_order_cannot_change_its_terms(tmp_path, changed):
    book = seed(tmp_path / "book.sqlite")
    original = book.prepare_delivery(ASSET, "A", POLICY)
    with pytest.raises(CoinError):
        book.prepare_delivery(ASSET, "A", {**POLICY, **changed})
    assert book.delivery(ASSET, "A") == original


def test_new_orders_cannot_replace_route_and_failed_preparation_does_not_freeze_them(tmp_path):
    book = seed(tmp_path / "book.sqlite")
    book.prepare_delivery(ASSET, "A", POLICY)
    book.reserve({**REQUEST, "order_id": "B"})
    with pytest.raises(CoinError, match="route differs"):
        book.prepare_delivery(ASSET, "B", {**POLICY, "treasury": "0x" + "ab"*20})
    assert book.release(ASSET, "B")["status"] == "released"
    book.reserve({**REQUEST, "order_id": "C", "buyer": "opaque-name"})
    with pytest.raises(CoinError):
        book.prepare_delivery(ASSET, "C", POLICY)
    assert book.release(ASSET, "C")["status"] == "released"
    late = ReservationBook(book.path, clock=lambda: 101)
    with pytest.raises(CoinError, match="active reservation"):
        late.prepare_delivery(ASSET, "B", POLICY)


def test_two_reconcilers_post_obligations_once_and_keep_inventory_consumed(tmp_path, monkeypatch):
    path = tmp_path / "book.sqlite"
    book = seed(path)
    book.prepare_delivery(ASSET, "A", POLICY)
    barrier = Barrier(2)

    def observe(terms, nodes):
        barrier.wait(timeout=5)
        return PROJECTION
    # This test isolates the database race. Signed execution and the observer
    # itself are covered by the integration suite and evidence unit tests.
    monkeypatch.setattr(delivery_evidence, "confirmed_fill", observe)
    def reconcile(_):
        return ReservationBook(path, clock=lambda: 110).reconcile_delivery(ASSET, "A", [])
    with ThreadPoolExecutor(2) as pool:
        first, second = pool.map(reconcile, range(2))
    assert first == second and first["status"] == "delivered"
    assert len(first["obligations"]) == 9
    assert sum(int(n["amount"]) for n in first["obligations"]) == 90000000
    assert book.inventory(ASSET)["networks"] == ReservationBook(path, clock=lambda: 150).inventory(ASSET)["networks"]
    available = {n["id"]: n["float_available"] for n in book.inventory(ASSET)["networks"]}
    assert available["ethereum"] == "99900000000"
    with pytest.raises(CoinError, match="cannot release"):
        book.release(ASSET, "A")
    assert book.get(ASSET, "A")["status"] == "delivered"
