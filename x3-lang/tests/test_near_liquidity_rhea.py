"""Issue #40: agreed NEAR chain views, RHEA pool discovery, and the provider support record.

Every RPC answer here is served from `fixtures/near_liquidity/rhea_rpc_constructed.json`, a
CONSTRUCTED fixture shaped after the NEAR and RHEA documentation it cites. None of its
reserves, hashes or outputs were observed on chain. No test contacts a network.
"""
import base64
import copy
import json
import sys
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))

from near_liquidity import chain, providers, rhea  # noqa: E402
from near_liquidity import ranking as agg  # noqa: E402
from near_liquidity.quote import AssetRef, Commitment, LiquidityError, SignatureStatus  # noqa: E402

FIXTURE = json.loads((ROOT / "tests" / "fixtures" / "near_liquidity" / "rhea_rpc_constructed.json").read_text())
NOW = 1793678210
WRAP = "wrap.near"
USDC = "17208628f84f5d6ad33f0da3bbbeb27ffcb398eac501a31bd6ad2011e36133a1"
ONE_NEAR = 10**24
URLS = ("https://rpc-a.example", "https://rpc-b.example")


def test_fixture_is_labelled_constructed_with_sources():
    meta = FIXTURE["_fixture"]
    assert meta["kind"].startswith("CONSTRUCTED")
    assert all(url.startswith("https://") for url in meta["shape_sources"])


class FakeRpc:
    """Serves the constructed fixture per host; individual hosts can be made to diverge."""

    def __init__(self, fixture=FIXTURE, now=NOW):
        self.fixture = copy.deepcopy(fixture)
        header = self.fixture["block_header"]
        self.header = {"height": header["height"], "hash": header["hash"],
                       "timestamp": (now + header["timestamp_offset_s"]) * 1_000_000_000}
        self.overrides = {}
        self.status = {}
        self.calls = []

    def __call__(self, method, url, headers, body, timeout):
        host = url.split("//")[1]
        assert method == "POST" and url.startswith("https://")
        request = json.loads(body)
        self.calls.append((host, request["method"], request["params"]))
        if host in self.status:
            return self.status[host], b'{"error": "rate limited"}'
        params = request["params"]
        if request["method"] == "block":
            header = dict(self.header, **self.overrides.get((host, "block"), {}))
            if "block_id" in params and params["block_id"] != header["hash"]:
                return 200, json.dumps({"error": {"name": "HANDLER_ERROR", "cause": {"name": "UNKNOWN_BLOCK"}}}).encode()
            return 200, json.dumps({"result": {"header": header}}).encode()
        assert request["method"] == "query" and params["request_type"] == "call_function"
        args = base64.b64decode(params["args_base64"]).decode()
        key = f"{params['account_id']}|{params['method_name']}|{args}"
        value = self.overrides.get((host, key), self.fixture["views"].get(key))
        if value is None:
            return 200, json.dumps({"error": {"name": "HANDLER_ERROR", "cause": {"name": "CONTRACT_EXECUTION_ERROR"}}}).encode()
        result = {"result": list(json.dumps(value, separators=(",", ":")).encode()), "logs": [],
                  "block_height": self.header["height"], "block_hash": params["block_id"]}
        return 200, json.dumps({"result": result}).encode()


def make(rpc=None, *, page_size=2, max_pools=100, urls=URLS, max_block_age_s=30):
    rpc = rpc or FakeRpc()
    view = chain.AgreedNearView([chain.RpcEndpoint(u) for u in urls], transport=rpc, max_block_age_s=max_block_age_s)
    return rpc, rhea.RheaPoolDiscovery(view, page_size=page_size, max_pools=max_pools)


# ------------------------------------------------------------ task 3: discovery ----

def test_discovers_every_pool_at_one_agreed_block():
    rpc, discovery = make()
    snapshot = discovery.discover(now=NOW)
    assert [p.pool_id for p in snapshot.pools] == [0, 1, 2]
    assert snapshot.pools[0].amounts == (1000 * ONE_NEAR, 3_000_000_000)
    assert snapshot.pools[2].pool_kind == "STABLE_SWAP" and snapshot.pools[2].amp == 240
    queried_blocks = {p.get("block_id") for _, m, p in rpc.calls if m == "query"}
    assert queried_blocks == {FIXTURE["block_header"]["hash"]}
    assert {h for h, _, _ in rpc.calls} == {"rpc-a.example", "rpc-b.example"}


def test_observation_quote_comes_from_get_return_and_is_never_executable():
    _, discovery = make()
    snapshot = discovery.discover(now=NOW)
    quote = discovery.observe_quote(snapshot, 0, WRAP, USDC, ONE_NEAR, now=NOW)
    assert quote.amount_out == 2988016 and quote.min_amount_out == 0
    assert quote.asset_in == AssetRef("near", "nep141:wrap.near", 24)
    assert quote.asset_out == AssetRef("near", f"nep141:{USDC}", 6)
    assert quote.executable is False and quote.commitment is Commitment.CHAIN_OBSERVATION
    assert quote.signature.status is SignatureStatus.UNSIGNED
    assert quote.guaranteed_net_output() is None
    assert quote.provenance.block_height == FIXTURE["block_header"]["height"]
    assert quote.provenance.reference.endswith(FIXTURE["block_header"]["hash"])
    assert quote.fees[0].provider_value == "30" and quote.fees[0].amount is None
    result = agg.aggregate(agg.AggregationRequest(quote.asset_in, quote.asset_out, ONE_NEAR, NOW, 30, 0), [quote])
    assert not result.ranked and result.observations[0].expected_net_output == 2988016
    with pytest.raises(LiquidityError) as err:
        agg.select_executable(result, now=NOW, ledger=agg.QuoteLedger())
    assert err.value.code == "X3_LIQ_NO_LIQUIDITY"


def test_rpc_disagreement_on_the_block_fails_closed():
    rpc = FakeRpc()
    rpc.overrides[("rpc-b.example", "block")] = {"timestamp": rpc.header["timestamp"] + 1}
    _, discovery = make(rpc)
    with pytest.raises(LiquidityError) as err:
        discovery.discover(now=NOW)
    assert err.value.code == "X3_LIQ_RPC_DISAGREEMENT"


def test_rpc_disagreement_on_reserves_fails_closed():
    rpc = FakeRpc()
    key = 'v2.ref-finance.near|get_pools|{"from_index":0,"limit":2}'
    altered = copy.deepcopy(rpc.fixture["views"][key])
    altered[0]["amounts"][1] = "3000000001"
    rpc.overrides[("rpc-b.example", key)] = altered
    _, discovery = make(rpc)
    with pytest.raises(LiquidityError) as err:
        discovery.discover(now=NOW)
    assert err.value.code == "X3_LIQ_RPC_DISAGREEMENT"


def test_endpoint_that_does_not_know_the_block_is_refused():
    rpc = FakeRpc()
    rpc.overrides[("rpc-b.example", "block")] = {"hash": "11111111111111111111111111111111"}
    _, discovery = make(rpc)
    with pytest.raises(LiquidityError) as err:
        discovery.discover(now=NOW)
    assert err.value.code == "X3_LIQ_RPC_ERROR"


def test_stale_chain_state_is_refused():
    _, discovery = make()
    with pytest.raises(LiquidityError) as err:
        discovery.discover(now=NOW + 40)
    assert err.value.code == "X3_LIQ_STALE"
    snapshot = discovery.discover(now=NOW)
    with pytest.raises(LiquidityError) as err:
        discovery.observe_quote(snapshot, 0, WRAP, USDC, ONE_NEAR, now=NOW + 40)
    assert err.value.code == "X3_LIQ_STALE"


def test_rpc_rate_limit_is_a_named_refusal():
    rpc = FakeRpc()
    rpc.status["rpc-b.example"] = 429
    _, discovery = make(rpc)
    with pytest.raises(LiquidityError) as err:
        discovery.discover(now=NOW)
    assert err.value.code == "X3_LIQ_RATE_LIMITED"


@pytest.mark.parametrize("urls", [("https://rpc-a.example",), ("https://rpc-a.example", "https://rpc-a.example/v2"),
                                  ("https://rpc-a.example", "http://rpc-b.example"),
                                  ("https://rpc-a.example", "https://user:key@rpc-b.example")])
def test_single_same_host_plaintext_or_credentialed_endpoints_are_refused(urls):
    with pytest.raises(LiquidityError) as err:
        make(urls=urls)
    assert err.value.code == "X3_LIQ_RPC_CONFIG"


def test_empty_reserves_zero_return_and_drain_are_no_liquidity():
    _, discovery = make()
    snapshot = discovery.discover(now=NOW)
    with pytest.raises(LiquidityError) as err:
        discovery.observe_quote(snapshot, 1, WRAP, "token.v2.ref-finance.near", ONE_NEAR, now=NOW)
    assert err.value.code == "X3_LIQ_NO_LIQUIDITY"
    key = ('v2.ref-finance.near|get_return|{"amount_in":"1000000000000000000000000","pool_id":0,'
           '"token_in":"wrap.near","token_out":"%s"}' % USDC)
    for value in ("0", "3000000000"):
        rpc = FakeRpc()
        rpc.fixture["views"][key] = value
        _, discovery = make(rpc)
        with pytest.raises(LiquidityError) as err:
            discovery.observe_quote(discovery.discover(now=NOW), 0, WRAP, USDC, ONE_NEAR, now=NOW)
        assert err.value.code == "X3_LIQ_NO_LIQUIDITY"


def test_unknown_pool_and_wrong_pair_are_refused():
    _, discovery = make()
    snapshot = discovery.discover(now=NOW)
    with pytest.raises(LiquidityError) as err:
        discovery.observe_quote(snapshot, 99, WRAP, USDC, ONE_NEAR, now=NOW)
    assert err.value.code == "X3_LIQ_NO_LIQUIDITY"
    with pytest.raises(LiquidityError) as err:
        discovery.observe_quote(snapshot, 0, WRAP, "usdt.tether-token.near", ONE_NEAR, now=NOW)
    assert err.value.code == "X3_LIQ_ASSET_MISMATCH"


@pytest.mark.parametrize("mutate", [
    lambda p: p[0].pop("amounts"),
    lambda p: p[0].update(amounts=[1000, "3000000000"]),
    lambda p: p[0].update(amounts=["-1", "3000000000"]),
    lambda p: p[0].update(amounts=[str(2**128), "3000000000"]),
    lambda p: p[0].update(token_account_ids=["Wrap.Near", USDC]),
    lambda p: p[0].update(total_fee="30"),
    lambda p: p[0].update(amounts=["1"]),
])
def test_malformed_pool_data_is_refused(mutate):
    rpc = FakeRpc()
    key = 'v2.ref-finance.near|get_pools|{"from_index":0,"limit":2}'
    mutate(rpc.fixture["views"][key])
    _, discovery = make(rpc)
    with pytest.raises(LiquidityError) as err:
        discovery.discover(now=NOW)
    assert err.value.code in {"X3_LIQ_MALFORMED", "X3_LIQ_OUT_OF_RANGE"}


def test_pool_listing_a_token_twice_is_duplicate_inventory():
    rpc = FakeRpc()
    key = 'v2.ref-finance.near|get_pools|{"from_index":0,"limit":2}'
    rpc.fixture["views"][key][0]["token_account_ids"] = [WRAP, WRAP]
    _, discovery = make(rpc)
    with pytest.raises(LiquidityError) as err:
        discovery.discover(now=NOW)
    assert err.value.code == "X3_LIQ_DUPLICATE_INVENTORY"


def test_more_pools_than_the_bound_is_refused_not_truncated():
    _, discovery = make(max_pools=2)
    with pytest.raises(LiquidityError) as err:
        discovery.discover(now=NOW)
    assert err.value.code == "X3_LIQ_TOO_MANY_POOLS"


def test_missing_or_invalid_decimals_are_refused():
    rpc = FakeRpc()
    rpc.fixture["views"][f"{USDC}|ft_metadata|{{}}"]["decimals"] = "6"
    _, discovery = make(rpc)
    with pytest.raises(LiquidityError) as err:
        discovery.observe_quote(discovery.discover(now=NOW), 0, WRAP, USDC, ONE_NEAR, now=NOW)
    assert err.value.code == "X3_LIQ_DECIMALS"


def test_no_default_reserves_exist_without_chain_answers():
    rpc = FakeRpc()
    rpc.fixture["views"] = {}
    _, discovery = make(rpc)
    with pytest.raises(LiquidityError) as err:
        discovery.discover(now=NOW)
    assert err.value.code == "X3_LIQ_RPC_ERROR"


# -------------------------------------------------------- task 4: provider record ----

@pytest.mark.parametrize("provider", ["lifi", "rango"])
def test_unconfirmed_providers_are_blocked_with_sources(provider):
    with pytest.raises(LiquidityError) as err:
        providers.require_adapter(provider)
    assert err.value.code == "X3_LIQ_UNSUPPORTED_PROVIDER"
    assert all(u.startswith("https://") for u in err.value.detail["sources"])
    with pytest.raises(LiquidityError):
        providers.require_adapter("some-new-aggregator")


def test_implemented_adapters_are_recorded():
    assert providers.require_adapter("near-intents-1click").status == "adapter"
    assert providers.require_adapter(rhea.PROVIDER_ID).status == "adapter"
