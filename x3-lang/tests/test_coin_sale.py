"""Equal-share treasury quotes are arithmetic only. All amounts are base units."""
from copy import deepcopy
import json
from pathlib import Path

import pytest

from coin import CoinError, U64_MAX
from coin.sale import plan_sale


EXAMPLE = Path(__file__).resolve().parents[1] / "coin" / "examples" / "ten_chain_sale.json"


def example():
    return json.loads(EXAMPLE.read_text(encoding="utf-8"))


def test_ten_chain_sale_100_tokens_into_ethereum():
    spec = example()
    before = deepcopy(spec)
    units = 10 ** spec["decimals"]
    assert sum(int(n["available"]) for n in spec["networks"]) == 1_000_000 * units
    quote = plan_sale(spec)
    assert spec == before
    assert quote["status"] == "quote_only"
    assert quote["destination"] == "ethereum"
    assert quote["amount"] == str(100 * units)
    assert quote["destination_local_share"] == str(10 * units)
    assert quote["destination_receivable"] == str(90 * units)
    assert quote["destination_float_after_delivery"] == str(99_900 * units)
    assert len(quote["shares"]) == 10
    assert all(s["charge"] == str(10 * units) for s in quote["shares"])
    for share in quote["shares"]:
        assert share["remote_payable"] == ("0" if share["network"] == "ethereum" else str(10 * units))
    assert sum(int(s["remote_payable"]) for s in quote["shares"]) == int(quote["destination_receivable"])
    assert sum(int(s["charge"]) for s in quote["shares"]) == int(quote["amount"])
    assert plan_sale({**spec, "networks": spec["networks"][::-1]}) == quote


@pytest.mark.parametrize(("field", "value", "message"), [
    ("amount", "100000001", "whole base units"),
    ("amount", "0", "outside the supported range"),
    ("amount", 100000000, "decimal string"),
    ("destination_float", "99999999", "insufficient destination float"),
    ("destination", "unknown", "participating networks"),
    ("destination", {}, "participating networks"),
    ("decimals", True, "integer"),
    ("networks", [], "nonempty list"),
    ("amount", str(U64_MAX + 1), "outside the supported range"),
])
def test_refuses_invalid_sale(field, value, message):
    with pytest.raises(CoinError, match=message):
        plan_sale({**example(), field: value})


@pytest.mark.parametrize(("index", "field", "value", "message"), [
    (0, "available", "9999999", "insufficient uncommitted quota"),
    (1, "available", "9999999", "insufficient uncommitted quota"),
    (1, "available", "010000000", "decimal string"),
    (1, "id", "ethereum", "duplicate network id"),
    (1, "id", "Ethereum", "lowercase ASCII"),
])
def test_refuses_invalid_networks(index, field, value, message):
    spec = example()
    spec["networks"][index][field] = value
    with pytest.raises(CoinError, match=message):
        plan_sale(spec)


def test_large_values_and_exact_exhaustion():
    half = U64_MAX // 2
    spec = {
        "amount": str(2 * half), "decimals": 0,
        "networks": [{"id": network, "available": str(half)} for network in ("x3", "ethereum")],
        "destination": "x3", "destination_float": str(2 * half),
    }
    quote = plan_sale(spec)
    assert quote["destination_float_after_delivery"] == "0"
    assert quote["destination_receivable"] == str(half)
    assert quote["destination_local_share"] == str(half)


def test_single_destination_treasury_has_no_remote_receivable():
    spec = example()
    quote = plan_sale({**spec, "networks": [spec["networks"][0]]})
    assert quote["destination_receivable"] == "0"
    assert quote["shares"] == [{"network": "ethereum", "charge": spec["amount"], "remote_payable": "0"}]
