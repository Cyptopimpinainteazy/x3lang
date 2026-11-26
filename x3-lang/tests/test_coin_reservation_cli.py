"""Run the stewardship commands in separate processes against a temporary database."""
import json
import os
import subprocess
import sys
import time
from pathlib import Path

from coin import MAX_SOURCE_BYTES

ROOT = Path(__file__).resolve().parents[1]
EXAMPLE = ROOT / "coin/examples/ten_chain_inventory.json"
CONFIG = json.loads(EXAMPLE.read_text())


def cli(db, command, *args, ok=True):
    proc = subprocess.run(
        [sys.executable, "-m", "coin", command, "--db", str(db), *map(str, args)],
        cwd=db.parent,
        env={**os.environ, "PYTHONPATH": str(ROOT)},
        capture_output=True, text=True, timeout=15,
    )
    if ok:
        assert proc.returncode == 0, proc.stderr
        return json.loads(proc.stdout)
    assert proc.returncode != 0
    assert not proc.stdout
    assert "coin:" in proc.stderr and "Traceback" not in proc.stderr
    return proc.stderr


def test_hold_survives_cli_restart_and_releases_once(tmp_path):
    db = tmp_path / "stock.sqlite"
    asset = CONFIG["asset_id"]
    initial = cli(db, "inventory-init", EXAMPLE)
    assert len(initial["networks"]) == 10
    assert sum(int(n["opening"]) for n in initial["networks"]) == 1_000_000 * 10**6
    order = {
        "asset_id": asset, "order_id": "sale-100", "buyer": "buyer-01",
        "destination": "ethereum", "amount": "100000000", "expires_at": int(time.time()) + 600,
    }
    request = tmp_path / "order.json"
    request.write_text(json.dumps(order))
    hold = cli(db, "reserve", request)
    assert hold["status"] == "reserved" and len(hold["shares"]) == 10
    assert all(row["charge"] == "10000000" for row in hold["shares"])
    assert cli(db, "reservation", asset, order["order_id"]) == hold
    assert cli(db, "reserve", request) == hold
    balance = cli(db, "inventory", asset)
    for row in balance["networks"]:
        assert row["quota_available"] == "99990000000"
        assert row["float_available"] == ("99900000000" if row["id"] == "ethereum" else "100000000000")
    closed = cli(db, "release", asset, order["order_id"])
    assert closed["status"] == "released"
    assert cli(db, "release", asset, order["order_id"]) == closed
    assert cli(db, "reserve", request) == closed
    assert cli(db, "inventory", asset)["networks"] == initial["networks"]


def test_invalid_json_cannot_initialize_or_change_inventory(tmp_path):
    db = tmp_path / "stock.sqlite"
    spec = tmp_path / "input.json"
    spec.write_text('{"decimals":6,"decimals":7}')
    assert "duplicate JSON field" in cli(db, "inventory-init", spec, ok=False)
    assert not db.exists()
    before = cli(db, "inventory-init", EXAMPLE)
    spec.write_bytes(b" " * (MAX_SOURCE_BYTES + 1))
    assert "size limit" in cli(db, "reserve", spec, ok=False)
    assert cli(db, "inventory", CONFIG["asset_id"])["networks"] == before["networks"]


def test_database_typo_fails_without_creating_empty_book(tmp_path):
    db = tmp_path / "typo.sqlite"
    assert "inventory-init" in cli(db, "inventory", CONFIG["asset_id"], ok=False)
    assert not db.exists()
