"""Operator-facing delivery commands and their bounded read-only transport."""
import io
import json
import time
from pathlib import Path
from urllib.error import URLError

import pytest

from coin import CoinError
from coin import __main__ as cli
from coin.reservations import ReservationBook

FIXTURE = Path(__file__).resolve().parents[1] / "coin/examples/ten_chain_inventory.json"
ASSET = json.loads(FIXTURE.read_text())["asset_id"]
ORDER = "buy-100"


def policy(chain_id):
    return {
        "chain_id": chain_id,
        "genesis_hash": cli.SEPOLIA_GENESIS if chain_id == cli.SEPOLIA_CHAIN_ID else "0x" + "ab" * 32,
        "sale_contract": "0x" + "11" * 20, "sale_code_hash": "0x" + "aa" * 32,
        "token_contract": "0x" + "22" * 20, "token_code_hash": "0x" + "bb" * 32,
        "treasury": "0x" + "33" * 20, "from_block": 1, "destination": "ethereum",
        "payment_wei": "100000000000000",
    }


def prepare(tmp_path, capsys, chain_id):
    db = tmp_path / "stock.sqlite"
    config = json.loads(FIXTURE.read_text())
    book = ReservationBook(db)
    book.initialize(config)
    book.reserve({
        "asset_id": ASSET, "order_id": ORDER, "buyer": "0x" + "44" * 20, "destination": "ethereum",
        "amount": "100000000", "expires_at": int(time.time()) + 600,
    })
    source = tmp_path / "policy.json"
    source.write_text(json.dumps(policy(chain_id)))
    args = ["--db", str(db), ASSET, ORDER]
    assert cli.main(["sale-prepare", *args, str(source)]) == 0, capsys.readouterr().err
    return args, json.loads(capsys.readouterr().out)


def test_prepare_status_and_refuse_remote_read_for_local_vm(tmp_path, monkeypatch, capsys):
    args, prepared = prepare(tmp_path, capsys, 31337)
    assert prepared["status"] == "awaiting_confirmation" and prepared["evidence"] is None
    assert set(prepared["wallet_calls"]) == {"authorize", "buy"}
    assert prepared["wallet_calls"]["buy"]["from"] == "0x" + "44" * 20
    assert cli.main(["sale-status", *args]) == 0
    assert json.loads(capsys.readouterr().out) == prepared
    monkeypatch.setattr(cli, "_reader", lambda _: pytest.fail("local chain reached remote transport"))
    assert cli.main(["sale-confirm", *args, "--rpc-env", "UNSET_X3_RPC"]) == 1
    assert "Sepolia" in capsys.readouterr().err


def test_quorum_receives_two_distinct_readers(tmp_path, monkeypatch, capsys):
    args, _ = prepare(tmp_path, capsys, cli.SEPOLIA_CHAIN_ID)
    urls = ["https://rpc.one.example", "https://rpc.two.example"]
    monkeypatch.setenv("RPC_A", urls[0])
    monkeypatch.setenv("RPC_B", urls[1])
    calls = []

    def reconcile(self, asset, order, readers):
        assert asset == ASSET and order == ORDER
        calls.extend((endpoint, callable(reader)) for endpoint, reader in readers)
        return self.delivery(asset, order)

    monkeypatch.setattr(ReservationBook, "reconcile_delivery", reconcile)
    assert cli.main(["sale-confirm", *args, "--rpc-env", "RPC_A", "--rpc-env", "RPC_B"]) == 0
    assert calls == [(url, True) for url in urls]
    assert json.loads(capsys.readouterr().out)["status"] == "awaiting_confirmation"


@pytest.mark.parametrize("other", [None, "http://host.invalid", "https://user:secret@host.invalid",
                                    "https://host.invalid/?token=secret", "https://host.invalid/#secret", "https://API.example.:443/key"])
def test_invalid_quorum_urls_refused_without_contact(other):
    with pytest.raises(CoinError) as failure:
        cli._sale_readers(["https://api.example", other])
    assert "secret" not in str(failure.value)


class Response(io.BytesIO):
    status = 200


def test_transport_only_reads_and_bounds_the_rpc_response(monkeypatch):
    seen = []
    payload = b'{"jsonrpc":"2.0","id":"x3-sale-read","result":"0xaa36a7"}'

    class Opener:
        def open(self, req, timeout):
            seen.append((json.loads(req.data), timeout))
            return Response(payload)

    monkeypatch.setattr(cli, "build_opener", lambda handler: Opener())
    read = cli._reader("https://rpc.one.example")
    assert read("eth_chainId", []) == "0xaa36a7"
    with pytest.raises(CoinError):
        read("eth_sendRawTransaction", [])
    assert len(seen) == 1 and seen[0][1] == 15
    for invalid in (b'{"jsonrpc":"2.0","id":"wrong","result":1}', b"x" * (cli.MAX_RPC_BYTES + 1),
                    b'{"jsonrpc":"2.0","id":"x3-sale-read","error":{"message":"sensitive"}}'):
        payload = invalid
        with pytest.raises(CoinError, match="RPC read failed"):
            read("eth_chainId", [])


def test_redirects_and_network_errors_exclude_endpoint_secrets(monkeypatch):
    with pytest.raises(CoinError, match="redirects"):
        cli._NoRedirect().redirect_request(None, None, 302, "", {}, "https://other.invalid")
    class Opener:
        def open(self, req, timeout):
            raise URLError(req.full_url)
    monkeypatch.setattr(cli, "build_opener", lambda handler: Opener())
    with pytest.raises(CoinError, match="RPC read failed") as exc:
        cli._reader("https://rpc.example/private-secret")("eth_getLogs", [{}])
    assert "private-secret" not in str(exc.value)
