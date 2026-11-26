"""Durable local holds over shared network quotas and spendable destination inventory."""
import json
import sqlite3
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
from threading import Barrier

import pytest

from coin import CoinError, U64_MAX
from coin.reservations import ReservationBook


EXAMPLE = Path(__file__).resolve().parents[1] / "coin/examples/ten_chain_sale.json"
ASSET = "a1" * 32
OTHER_ASSET = "b2" * 32
BUYER = "0x" + "14" * 20
NOW = 1_800_000_000
UNITS = 10**6


def config():
    quote = json.loads(EXAMPLE.read_text())
    return {
        "asset_id": ASSET,
        "decimals": quote["decimals"],
        "total_inventory": str(1_000_000 * UNITS),
        "networks": [{"id": n["id"], "opening": n["available"]} for n in quote["networks"]],
    }


def order(order_id="order-1", **changes):
    return {
        "asset_id": ASSET, "order_id": order_id, "buyer": BUYER, "destination": "ethereum",
        "amount": str(100 * UNITS), "expires_at": NOW + 60, **changes,
    }


def balances(inventory):
    return {n["id"]: (n["quota_available"], n["float_available"]) for n in inventory["networks"]}


def test_holds_all_ten_quotas_and_full_destination_float_after_restart(tmp_path):
    path = tmp_path / "treasury.db"
    spec = config()
    book = ReservationBook(path, clock=lambda: NOW)
    book.initialize(spec)
    receipt = book.reserve(order())
    assert receipt["status"] == "reserved"
    assert {s["network"]: s["charge"] for s in receipt["shares"]} == {
        n["id"]: str(10 * UNITS) for n in spec["networks"]
    }

    reopened = ReservationBook(path, clock=lambda: NOW + 1)
    assert reopened.get(ASSET, "order-1") == receipt
    assert reopened.reserve(order()) == receipt
    inventory = reopened.inventory(ASSET)
    assert balances(inventory) == {
        n["id"]: (str(99_990 * UNITS), str((99_900 if n["id"] == "ethereum" else 100_000) * UNITS))
        for n in spec["networks"]
    }
    assert all(row["opening"] == str(100_000 * UNITS) for row in inventory["networks"])
    assert reopened.initialize({**spec, "networks": spec["networks"][::-1]}) == inventory
    with pytest.raises(CoinError):
        reopened.initialize({**spec, "decimals": 5})
    assert reopened.inventory(ASSET) == inventory


@pytest.mark.parametrize("change", [
    {"buyer": "0x" + "33" * 20}, {"destination": "base"}, {"amount": str(200 * UNITS)},
    {"expires_at": NOW + 59},
])
def test_idempotency_key_binds_terms_even_after_release(tmp_path, change):
    book = ReservationBook(tmp_path / "treasury.db", clock=lambda: NOW)
    opening = book.initialize(config())
    book.reserve(order())
    for state in ("reserved", "released"):
        if state == "released":
            released = book.release(ASSET, "order-1")
            assert released["status"] == state
            assert book.release(ASSET, "order-1") == released
            assert book.reserve(order()) == released
            assert balances(book.inventory(ASSET)) == balances(opening)
        before = book.inventory(ASSET)
        with pytest.raises(CoinError):
            book.reserve(order(**change))
        assert book.inventory(ASSET) == before


def test_expiry_at_deadline_survives_restart_and_never_revives_old_order(tmp_path):
    path = tmp_path / "treasury.db"
    book = ReservationBook(path, clock=lambda: NOW)
    opening = book.initialize(config())
    book.reserve(order())
    almost_due = ReservationBook(path, clock=lambda: NOW + 59)
    assert almost_due.get(ASSET, "order-1")["status"] == "reserved"
    due = ReservationBook(path, clock=lambda: NOW + 60)
    assert balances(due.inventory(ASSET)) == balances(opening)
    expired = due.get(ASSET, "order-1")
    assert expired["status"] == "expired"
    assert due.reserve(order()) == expired
    assert due.release(ASSET, "order-1") == expired
    assert balances(due.inventory(ASSET)) == balances(opening)
    replacement = due.reserve(order("order-2", expires_at=NOW + 120))
    assert replacement["status"] == "reserved"
    assert balances(due.inventory(ASSET))["ethereum"][1] == str(99_900 * UNITS)


@pytest.mark.parametrize("change", [
    {"asset_id": OTHER_ASSET}, {"destination": "missing"}, {"amount": "1"},
    {"amount": "0100000000"}, {"amount": 100 * UNITS}, {"expires_at": NOW},
    {"expires_at": True}, {"destination_float": str(U64_MAX)}, {"networks": []},
])
def test_refusals_leave_all_stored_inventory_intact(tmp_path, change):
    book = ReservationBook(tmp_path / "treasury.db", clock=lambda: NOW)
    opening = book.initialize(config())
    with pytest.raises(CoinError):
        book.reserve(order(**change))
    assert balances(book.inventory(ASSET)) == balances(opening)
    assert book.reserve(order())["status"] == "reserved"


@pytest.mark.parametrize("mutation", ["total", "duplicate", "numeric", "noncanonical", "asset", "overflow"])
def test_invalid_openings_cannot_initialize_book(tmp_path, mutation):
    book = ReservationBook(tmp_path / "treasury.db", clock=lambda: NOW)
    spec = config()
    if mutation == "total":
        spec["total_inventory"] = "1"
    elif mutation == "duplicate":
        spec["networks"][0]["id"] = spec["networks"][1]["id"]
    elif mutation == "numeric":
        spec["networks"][0]["opening"] = 100_000 * UNITS
    elif mutation == "noncanonical":
        spec["networks"][0]["opening"] = "0" + spec["networks"][0]["opening"]
    elif mutation == "asset":
        spec["asset_id"] = "unbound-ticker"
    else:
        spec["networks"] = [{"id": "ethereum", "opening": str(U64_MAX)}, {"id": "base", "opening": "1"}]
        spec["total_inventory"] = str(U64_MAX + 1)
    with pytest.raises(CoinError):
        book.initialize(spec)
    assert len(book.initialize(config())["networks"]) == 10


def test_unsigned_64bit_round_trip_with_exact_exhaustion(tmp_path):
    path = tmp_path / "treasury.db"
    book = ReservationBook(path, clock=lambda: NOW)
    full = str(U64_MAX)
    spec = {**config(), "total_inventory": full, "networks": [{"id": "ethereum", "opening": full}]}
    book.initialize(spec)
    receipt = book.reserve(order(amount=full))
    assert receipt["shares"][0]["charge"] == full
    assert balances(book.inventory(ASSET)) == {"ethereum": ("0", "0")}
    with pytest.raises(CoinError):
        book.reserve(order("order-2", amount="1"))
    reopened = ReservationBook(path, clock=lambda: NOW)
    reopened.release(ASSET, "order-1")
    assert balances(reopened.inventory(ASSET)) == {"ethereum": (full, full)}


def test_simultaneous_buyers_cannot_share_last_deliverable_inventory(tmp_path):
    path = tmp_path / "treasury.db"
    spec = config()
    for n in spec["networks"]:
        n["opening"] = str(100 * UNITS)
    spec["total_inventory"] = str(1_000 * UNITS)
    master = ReservationBook(path, clock=lambda: NOW)
    master.initialize(spec)
    ready = Barrier(2)

    def compete(key):
        participant = ReservationBook(path, clock=lambda: NOW)
        ready.wait(timeout=10)
        try:
            return participant.reserve(order(key))["status"]
        except CoinError:
            return "refused"

    with ThreadPoolExecutor(max_workers=2) as workers:
        outcomes = list(workers.map(compete, ("buyer-a", "buyer-b")))
    assert sorted(outcomes) == ["refused", "reserved"]
    assert balances(master.inventory(ASSET)) == {
        n["id"]: (str(90 * UNITS), "0" if n["id"] == "ethereum" else str(100 * UNITS))
        for n in spec["networks"]
    }


def test_remote_quota_shortage_preserves_destination_and_other_nine_quotas(tmp_path):
    book = ReservationBook(tmp_path / "treasury.db", clock=lambda: NOW)
    spec = config()
    spec["networks"][-1]["opening"] = "1"
    spec["total_inventory"] = str(900_000 * UNITS + 1)
    opening = book.initialize(spec)
    with pytest.raises(CoinError):
        book.reserve(order())
    assert balances(book.inventory(ASSET)) == balances(opening)


def test_interrupted_database_write_rolls_back_and_same_order_can_retry(tmp_path):
    path = tmp_path / "treasury.db"
    book = ReservationBook(path, clock=lambda: NOW)
    opening = book.initialize(config())
    with sqlite3.connect(path) as db:
        db.execute("""
            CREATE TRIGGER simulate_interruption AFTER INSERT ON sale_holds
            BEGIN SELECT RAISE(ABORT, 'simulated interruption'); END
        """)
    with pytest.raises((sqlite3.DatabaseError, CoinError)):
        book.reserve(order())
    with sqlite3.connect(path) as db:
        assert db.execute("SELECT COUNT(*) FROM sale_holds").fetchone() == (0,)
        db.execute("DROP TRIGGER simulate_interruption")
    recovered = ReservationBook(path, clock=lambda: NOW + 1)
    assert balances(recovered.inventory(ASSET)) == balances(opening)
    assert recovered.reserve(order())["status"] == "reserved"


def test_simultaneous_retry_returns_one_receipt_and_charges_once(tmp_path):
    path = tmp_path / "treasury.db"
    master = ReservationBook(path, clock=lambda: NOW)
    master.initialize(config())
    ready = Barrier(2)

    def repeat(_client):
        participant = ReservationBook(path, clock=lambda: NOW)
        ready.wait(timeout=10)
        return participant.reserve(order())

    with ThreadPoolExecutor(max_workers=2) as workers:
        first, second = workers.map(repeat, (1, 2))
    assert first == second
    assert balances(master.inventory(ASSET))["ethereum"] == (str(99_990 * UNITS), str(99_900 * UNITS))


def test_failed_order_does_not_revive_an_elapsed_hold_after_clock_rollback(tmp_path):
    path = tmp_path / "treasury.db"
    initial = ReservationBook(path, clock=lambda: NOW)
    initial.initialize(config())
    initial.reserve(order(expires_at=NOW + 10))
    later = ReservationBook(path, clock=lambda: NOW + 11)
    with pytest.raises(CoinError, match="insufficient destination float"):
        later.reserve(order("too-large", amount=str(100_010 * UNITS)))
    after_clock_adjustment = ReservationBook(path, clock=lambda: NOW + 9)
    assert after_clock_adjustment.get(ASSET, "order-1")["status"] == "expired"
    with pytest.raises(CoinError, match="unknown reservation"):
        after_clock_adjustment.get(ASSET, "too-large")
