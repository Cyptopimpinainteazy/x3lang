"""X3 Route chains: the EVM chains NEAR Intents 1Click routes, the ones it does not, and the
deposit/refund evidence for EVM origins (native coins and reviewed ERC-20s)."""
from __future__ import annotations

import json
from pathlib import Path

import pytest

from test_near_intents import (  # noqa: F401  (_trust_the_test_key is an autouse fixture)
    NOW, FakeNearChain, RecordingTransport, _trust_the_test_key, ni, near_rpc, sign,
)

ROOT = Path(__file__).resolve().parents[1]
ce = ni.chain_evidence
ROUTE_TOKENS = ROOT / "tests" / "fixtures" / "x3_route_tokens.json"
BASE_EXAMPLE = ROOT / "examples" / "x3_route_base_usdc_to_near.x3"
REFUND = "0x000000000000000000000000000000000000dEaD"
EVM_DEPOSIT = "0x" + "c" * 40
EVM_TX = "0x" + "a" * 64
BASE_USDC = "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913"
T_0301 = 1793674860  # 2026-11-03T03:01:00Z, inside the 10-minute quote window


def program(tmp_path: Path, *, source: str, dest: str = "Near.USDC", receiver: str = REFUND,
            to_receiver: str = "x3-demo.near", amount: int = 25_000_000) -> str:
    chain, asset = source.split(".")
    body = f"""intent route_test {{
    from {source} amount {amount} receiver {receiver}
    to {dest} receiver {to_receiver}
    route {{
        bridge NearIntents {source} -> {dest};
    }}
    require slippage <= 1%
    require fees <= 100 bps
    require nonce unused route_test_001
    timeout 10m refund {source}
    on_fail rollback
}}
"""
    path = tmp_path / f"{chain.lower()}_{asset.lower()}.x3"
    path.write_text(body)
    return str(path)


def tokens():
    return json.loads(ROUTE_TOKENS.read_text())


# --- routing ------------------------------------------------------------------------------

@pytest.mark.parametrize("source,blockchain,asset_id", [
    ("Base.USDC", "base", "nep141:base-0x833589fcd6edb6e08f4c7c32d4f71b54bda02913.omft.near"),
    ("Optimism.USDC", "op", "nep245:v2_1.omni.hot.tg:10_A2ewyUyDp6qsue1jqZsGypkCxRJ"),
    ("Avalanche.USDC", "avax", "nep141:avax-0xb97ef9ef8734c71904d8002f8b6bc66dd9c48a6e.omdep.near"),
    ("Gnosis.USDC", "gnosis", "nep141:gnosis-0x2a22f9c3b484c3629090feed35f17ff8f88f76f0.omft.near"),
    ("Scroll.USDT", "scroll", "nep245:v2_1.omni.hot.tg:534352_4RG3Q2wFsMQmd45m5m89RjsLfupA"),
    ("Monad.USDC", "monad", "nep245:v2_1.omni.hot.tg:143_2dmLwYWkCQKyTjeUPAsGJuiVLbFx"),
    ("Berachain.USDT0", "bera", "nep141:bera-0x779ded0c9e1022225f8e0630b35a9b54be713736.omft.near"),
    ("Base.ETH", "base", "nep141:base.omft.near"),
])
def test_each_new_chain_compiles_to_a_1click_request(tmp_path, source, blockchain, asset_id):
    intent, leg = ni.compile_program(program(tmp_path, source=source), now=NOW)
    assert ni.CHAIN_TO_BLOCKCHAIN[leg["origin"]["chain"]] == blockchain
    request = ni.build_quote_request(leg, tokens(), dry=True)
    assert request["originAsset"] == asset_id
    assert request["refundTo"] == REFUND and request["refundType"] == "ORIGIN_CHAIN"


def test_aliases_name_the_same_chain(tmp_path):
    for alias in ("Op", "OP_MAINNET", "Avax", "Bera", "xDAI"):
        chain = ni.registry.normalize_chain(alias)
        assert chain in ni.CHAIN_TO_BLOCKCHAIN, alias


def test_evm_destination_needs_a_full_address(tmp_path):
    good = program(tmp_path, source="Near.USDC", dest="Base.USDC", receiver="x3-owner.near",
                   to_receiver="0x" + "1" * 40)
    _, leg = ni.compile_program(good, now=NOW)
    assert ni.build_quote_request(leg, tokens(), dry=True)["destinationAsset"].startswith("nep141:base-")
    short = program(tmp_path, source="Near.USDC", dest="Base.USDC", receiver="x3-owner.near", to_receiver="0xA1")
    with pytest.raises(ni.NearIntentsError) as err:
        ni.compile_program(short, now=NOW)
    assert err.value.code == "X3_NEAR_RECIPIENT"


def test_evm_refund_needs_a_full_address(tmp_path):
    with pytest.raises(ni.NearIntentsError) as err:
        ni.compile_program(program(tmp_path, source="Base.USDC", receiver="0xA1"), now=NOW)
    assert err.value.code == "X3_NEAR_REFUND_TO"


@pytest.mark.parametrize("chain", ["Mantle", "zkSync", "Linea", "Cronos", "Moonbeam", "Celo", "Plume",
                                   "Aurora", "HyperEVM", "zksync_era"])
def test_chains_1click_does_not_route_are_refused_by_name(tmp_path, chain):
    with pytest.raises(ni.NearIntentsError) as err:
        ni.compile_program(program(tmp_path, source=f"{chain}.USDC"), now=NOW)
    assert err.value.code == "X3_NEAR_TYPECHECK"
    codes = {e["code"] for e in err.value.detail}
    assert "X3_CHAIN_NOT_ROUTABLE" in codes, err.value.detail


def test_the_not_routable_list_and_the_routable_map_never_overlap():
    routable = set(ni.CHAIN_TO_BLOCKCHAIN)
    for chain, meta in ni.registry.NOT_ROUTABLE_CHAINS.items():
        assert ni.registry.normalize_chain(chain) is None, chain
        for alias in meta["aliases"]:
            assert ni.registry.normalize_chain(alias) not in routable, alias


def test_every_routable_evm_chain_has_a_chain_id():
    assert ni.EVM_CHAINS <= set(ce.EVM_CHAIN_IDS)
    assert {c for c in ni.CHAIN_TO_BLOCKCHAIN if ni.registry.CHAIN_DOMAINS[c]["address"] == "evm"} == ni.EVM_CHAINS


def test_deprecated_listings_do_not_make_a_symbol_ambiguous():
    token = ni.resolve_asset(tokens(), "avalanche", "USDC")
    assert token["assetId"] == "nep141:avax-0xb97ef9ef8734c71904d8002f8b6bc66dd9c48a6e.omdep.near"


# --- EVM evidence ---------------------------------------------------------------------------

def transfer_log(token, to, value, *, sender="0x" + "e" * 40, removed=False):
    pad = lambda a: "0x" + "0" * 24 + a[2:].lower()
    return {"address": token, "topics": [ce.ERC20_TRANSFER_TOPIC, pad(sender), pad(to)],
            "data": "0x" + format(value, "064x"), "removed": removed}


def evm_tx(*, to=EVM_DEPOSIT, value=0, logs=(), status="0x1", block=30_000_000, ts=T_0301, tx_hash=EVM_TX):
    return {"hash": tx_hash, "from": "0x" + "e" * 40, "to": to, "value": hex(value), "blockNumber": hex(block),
            "blockHash": "0x" + "b" * 64, "status": status, "logs": list(logs), "timestamp": ts}


class FakeEvm:
    def __init__(self, by_url, *, chain_id, finalized=30_000_010):
        self.by_url, self.chain_id, self.finalized, self.calls = by_url, chain_id, finalized, []

    def __call__(self, method, url, payload):
        self.calls.append(payload)
        txs = self.by_url[url]
        call, params = payload["method"], payload["params"]
        if call == "eth_chainId":
            result = hex(self.chain_id)
        elif call == "eth_getTransactionByHash":
            tx = txs.get(params[0])
            result = None if tx is None else {k: tx[k] for k in ("hash", "from", "to", "value", "blockNumber", "blockHash")}
        elif call == "eth_getTransactionReceipt":
            tx = txs.get(params[0])
            result = None if tx is None else {"transactionHash": tx["hash"], "blockHash": tx["blockHash"],
                                              "blockNumber": tx["blockNumber"], "status": tx["status"],
                                              "logs": tx["logs"]}
        elif call == "eth_getBlockByNumber":
            if params[0] == "finalized":
                result = {"number": hex(self.finalized), "hash": "0x" + "f" * 64}
            else:
                tx = next(t for t in txs.values() if t["blockNumber"] == params[0])
                result = {"number": params[0], "hash": tx["blockHash"], "timestamp": hex(tx["timestamp"])}
        else:
            raise AssertionError(call)
        return 200, {"jsonrpc": "2.0", "id": payload["id"], "result": result}


def evm_rpc(chain, txs_by_url, **kw):
    fake = FakeEvm(txs_by_url, chain_id=kw.pop("chain_id", ce.EVM_CHAIN_IDS[chain]), **kw)
    return ce.EvmRpc(chain, transport=RecordingTransport(fake), urls=list(txs_by_url)), fake


def verdict(ev):
    return {c["check"]: c["ok"] for c in ev["checks"]}


def token_deposit(rpc, *, amount=25_000_000):
    return rpc.verify_token_transfer(tx_hash=EVM_TX, token=BASE_USDC, recipient=EVM_DEPOSIT, min_amount=amount,
                                     not_after_ms=(T_0301 + 60) * 1000, purpose="deposit")


def test_a_finalized_erc20_deposit_on_base_is_verified():
    tx = evm_tx(logs=[transfer_log(BASE_USDC, EVM_DEPOSIT, 25_000_000)])
    rpc, _ = evm_rpc("base", {"https://base-a.invalid": {EVM_TX: tx}, "https://base-b.invalid": {EVM_TX: tx}})
    ev = token_deposit(rpc)
    assert ev["verified"] is True, ev["checks"]
    assert ev["kind"] == "evm_token_transfer" and ev["chain"] == "base" and ev["chainId"] == 8453
    assert ev["observed"] == "25000000" and len(ev["rpcs"]) == 2


@pytest.mark.parametrize("logs,failed", [
    ([transfer_log("0x" + "9" * 40, EVM_DEPOSIT, 25_000_000)], "recipient"),        # another contract's Transfer
    ([transfer_log(BASE_USDC, "0x" + "7" * 40, 25_000_000)], "recipient"),          # paid someone else
    ([transfer_log(BASE_USDC, EVM_DEPOSIT, 24_999_999)], "amount"),
    ([transfer_log(BASE_USDC, EVM_DEPOSIT, 25_000_000, removed=True)], "recipient"),  # reorged-out log
])
def test_an_erc20_deposit_that_does_not_pay_the_deposit_address_fails(logs, failed):
    rpc, _ = evm_rpc("base", {"https://base-a.invalid": {EVM_TX: evm_tx(logs=logs)}})
    ev = token_deposit(rpc)
    assert ev["verified"] is False and verdict(ev)[failed] is False


def test_two_transfers_in_one_transaction_add_up():
    logs = [transfer_log(BASE_USDC, EVM_DEPOSIT, 20_000_000), transfer_log(BASE_USDC, EVM_DEPOSIT, 5_000_000)]
    rpc, _ = evm_rpc("base", {"https://base-a.invalid": {EVM_TX: evm_tx(logs=logs)}})
    assert token_deposit(rpc)["verified"] is True


def test_a_reverted_unfinalized_late_or_wrong_chain_deposit_fails():
    ok_logs = [transfer_log(BASE_USDC, EVM_DEPOSIT, 25_000_000)]
    for kw, rpc_kw, failed in (
        ({"status": "0x0"}, {}, "succeeded"),
        ({}, {"finalized": 29_999_999}, "finalized"),
        ({"ts": T_0301 + 3600}, {}, "before_deadline"),
        ({}, {"chain_id": 1}, "chain_id"),
    ):
        rpc, _ = evm_rpc("base", {"https://base-a.invalid": {EVM_TX: evm_tx(logs=ok_logs, **kw)}}, **rpc_kw)
        ev = token_deposit(rpc)
        assert ev["verified"] is False and verdict(ev)[failed] is False, failed


def test_endpoints_that_disagree_fail():
    a = evm_tx(logs=[transfer_log(BASE_USDC, EVM_DEPOSIT, 25_000_000)])
    b = evm_tx(logs=[transfer_log(BASE_USDC, EVM_DEPOSIT, 30_000_000)])
    rpc, _ = evm_rpc("base", {"https://base-a.invalid": {EVM_TX: a}, "https://base-b.invalid": {EVM_TX: b}})
    ev = token_deposit(rpc)
    assert ev["verified"] is False and verdict(ev)["endpoints_agree"] is False


def test_a_missing_transaction_fails_closed():
    rpc, _ = evm_rpc("optimism", {"https://op-a.invalid": {}})
    ev = rpc.verify_native_transfer(tx_hash=EVM_TX, recipient=EVM_DEPOSIT, min_amount=1, not_after_ms=None,
                                    purpose="deposit")
    assert ev["verified"] is False and verdict(ev)["found"] is False


def test_a_native_eth_deposit_on_optimism_is_verified():
    rpc, _ = evm_rpc("optimism", {"https://op-a.invalid": {EVM_TX: evm_tx(value=10**16)}})
    ev = rpc.verify_native_transfer(tx_hash=EVM_TX, recipient=EVM_DEPOSIT, min_amount=10**16,
                                    not_after_ms=None, purpose="deposit")
    assert ev["verified"] is True and ev["kind"] == "evm_native_transfer" and ev["chainId"] == 10


def test_an_unconfigured_chain_has_no_endpoints(monkeypatch):
    monkeypatch.delenv("X3_EVM_RPC_URLS_BASE", raising=False)
    with pytest.raises(ce.ChainEvidenceError) as err:
        ce.EvmRpc("base", transport=lambda *a: (200, b"{}"))
    assert err.value.code == "X3_EVIDENCE_RPC"
    monkeypatch.setenv("X3_EVM_RPC_URLS_BASE", "https://a.invalid, https://b.invalid")
    assert ce.EvmRpc("base", transport=lambda *a: (200, b"{}")).urls == ["https://a.invalid", "https://b.invalid"]


# --- binding the deposit to the signed asset ----------------------------------------------------

def asset_receipt(chain, asset_id, *, blockchain=None, contract=None):
    origin = {"assetId": asset_id, "blockchain": blockchain or ni.CHAIN_TO_BLOCKCHAIN[chain]}
    if contract is not None:
        origin["contractAddress"] = contract
    return {"leg": {"origin": {"chain": chain}}, "quote_response": {"quoteRequest": {"originAsset": asset_id}},
            "tokens": {"origin": origin}}


def test_a_base_usdc_asset_binds_to_the_reviewed_contract():
    asset = ni.evm_origin_asset(asset_receipt(
        "base", "nep141:base-0x833589fcd6edb6e08f4c7c32d4f71b54bda02913.omft.near", contract=BASE_USDC))
    assert asset == {"chain": "base", "kind": "erc20", "token": BASE_USDC}


def test_an_omni_asset_binds_through_its_pinned_id():
    asset = ni.evm_origin_asset(asset_receipt(
        "optimism", "nep245:v2_1.omni.hot.tg:10_A2ewyUyDp6qsue1jqZsGypkCxRJ",
        contract="0x0b2c639c533813f4aa9d7837caf62653d097ff85"))
    assert asset["token"] == "0x0b2c639c533813f4aa9d7837caf62653d097ff85"


@pytest.mark.parametrize("chain,asset_id", [
    ("base", "nep141:base.omft.near"),
    ("optimism", "nep245:v2_1.omni.hot.tg:10_11111111111111111111"),
    ("bsc", "nep245:v2_1.omni.hot.tg:56_11111111111111111111"),
])
def test_native_assets_are_recognised_from_the_signed_id(chain, asset_id):
    assert ni.evm_origin_asset(asset_receipt(chain, asset_id))["kind"] == "native"


def test_an_unreviewed_token_is_routable_but_not_provable():
    with pytest.raises(ni.NearIntentsError) as err:
        ni.evm_origin_asset(asset_receipt("monad", "nep245:v2_1.omni.hot.tg:143_2dmLwYWkCQKyTjeUPAsGJuiVLbFx",
                                          contract="0x754704bc059f8c67012fed69bc8a327a5aafb603"))
    assert err.value.code == "X3_EVIDENCE_UNSUPPORTED"
    with pytest.raises(ni.NearIntentsError) as err:  # a real-looking contract nobody reviewed
        ni.evm_origin_asset(asset_receipt("base", "nep141:base-0x" + "4" * 40 + ".omft.near", contract="0x" + "4" * 40))
    assert err.value.code == "X3_EVIDENCE_UNSUPPORTED"


@pytest.mark.parametrize("receipt", [
    # Token metadata names a different contract than the reviewed one for the signed asset.
    asset_receipt("base", "nep141:base-0x833589fcd6edb6e08f4c7c32d4f71b54bda02913.omft.near", contract="0x" + "9" * 40),
    # The signed asset is on another chain than the receipt's origin.
    asset_receipt("optimism", "nep245:v2_1.omni.hot.tg:56_11111111111111111111"),
    asset_receipt("base", "nep141:arb-0xaf88d065e77c8cc2239327c5edb3a432268e5831.omft.near",
                  contract="0xaf88d065e77c8cc2239327c5edb3a432268e5831"),
    # Metadata names a different blockchain.
    asset_receipt("base", "nep141:base.omft.near", blockchain="eth"),
    # A native asset whose metadata claims a contract.
    asset_receipt("base", "nep141:base.omft.near", contract="0x" + "9" * 40),
])
def test_receipt_metadata_that_disagrees_with_the_signed_asset_is_refused(receipt):
    with pytest.raises(ni.NearIntentsError) as err:
        ni.evm_origin_asset(receipt)
    assert err.value.code == "X3_EVIDENCE_BINDING"


# --- end to end: a signed Base quote, a receipt, and the deposit proven from Base -------------

def base_quote(request):
    quote = {"amountIn": request["amount"], "amountInFormatted": "25.0", "amountInUsd": "25.000",
             "minAmountIn": request["amount"], "amountOut": "24950000", "amountOutFormatted": "24.95",
             "amountOutUsd": "24.950", "minAmountOut": str(24_950_000 * (10_000 - request["slippageTolerance"]) // 10_000),
             "timeEstimate": 30, "refundFee": "50000"}
    if not request["dry"]:
        quote.update({"depositAddress": EVM_DEPOSIT, "deadline": request["deadline"],
                      "timeWhenInactive": request["deadline"]})
    return sign({"correlationId": "base-c0ffee", "timestamp": "2026-11-03T03:00:00.000Z", "signature": "",
                 "quoteRequest": dict(request), "quote": quote})


def route_api(quote_fn=base_quote):
    def handler(method, url, payload):
        if url.endswith("/v0/tokens"):
            return 200, tokens()
        if url.endswith("/v0/quote"):
            return 200, quote_fn(payload)
        return 404, {"message": "not found"}
    return RecordingTransport(handler)


def live_base_receipt(tmp_path):
    rpc, _ = near_rpc(chain=FakeNearChain(balances={}))
    path = tmp_path / "base.json"
    ni.quote(str(BASE_EXAMPLE), client=ni.OneClickClient(jwt="k", transport=route_api()), live=True,
             nonce_store=ni.NonceStore(str(tmp_path / "n.json")), receipt_path=str(path), rpc=rpc, now=NOW)
    return json.loads(path.read_text())


def test_a_base_deposit_is_proven_against_the_signed_quote(tmp_path):
    receipt = live_base_receipt(tmp_path)
    assert receipt["leg"]["origin"]["chain"] == "base"
    tx = evm_tx(logs=[transfer_log(BASE_USDC, EVM_DEPOSIT, 25_000_000)])
    rpc, _ = evm_rpc("base", {"https://base-a.invalid": {EVM_TX: tx}})
    evidence = ni.verify_deposit(receipt, tx_signature=EVM_TX, evm={"base": rpc})
    assert evidence["verified"] is True and evidence["kind"] == "evm_token_transfer", evidence
    assert evidence["recipient"] == EVM_DEPOSIT and evidence["token"] == BASE_USDC


def test_a_base_deposit_to_the_wrong_address_is_not_proven(tmp_path):
    receipt = live_base_receipt(tmp_path)
    tx = evm_tx(logs=[transfer_log(BASE_USDC, "0x" + "7" * 40, 25_000_000)])
    rpc, _ = evm_rpc("base", {"https://base-a.invalid": {EVM_TX: tx}})
    assert ni.verify_deposit(receipt, tx_signature=EVM_TX, evm={"base": rpc})["verified"] is False


def test_a_tampered_base_receipt_is_refused_before_any_rpc(tmp_path):
    receipt = live_base_receipt(tmp_path)
    receipt["tokens"]["origin"]["contractAddress"] = "0x" + "9" * 40
    rpc, fake = evm_rpc("base", {"https://base-a.invalid": {}})
    with pytest.raises(ni.NearIntentsError) as err:
        ni.verify_deposit(receipt, tx_signature=EVM_TX, evm={"base": rpc})
    assert err.value.code == "X3_EVIDENCE_BINDING" and fake.calls == []


REFUND_TX = "0x" + "d" * 64


def test_a_base_refund_is_proven_after_a_proven_deposit(tmp_path):
    receipt = live_base_receipt(tmp_path)
    deposit = evm_tx(logs=[transfer_log(BASE_USDC, EVM_DEPOSIT, 25_000_000)])
    refund = evm_tx(tx_hash=REFUND_TX, block=30_000_005, ts=T_0301 + 900, to=BASE_USDC,
                    logs=[transfer_log(BASE_USDC, REFUND, 25_000_000 - 50_000, sender=EVM_DEPOSIT)])
    refund["blockHash"] = "0x" + "e" * 64
    rpc, _ = evm_rpc("base", {"https://base-a.invalid": {EVM_TX: deposit, REFUND_TX: refund}})
    assert ni.verify_deposit(receipt, tx_signature=EVM_TX, evm={"base": rpc})["verified"] is True
    evidence = ni.verify_settlement(receipt, refund_tx=REFUND_TX, evm={"base": rpc})
    assert evidence["verified"] is True, evidence["checks"]
    assert evidence["purpose"] == "refund" and evidence["recipient"] == REFUND.lower()
    assert ni.conclude(receipt)["outcome"] == "refunded"


def test_a_refund_short_of_the_signed_amount_is_not_proven(tmp_path):
    receipt = live_base_receipt(tmp_path)
    deposit = evm_tx(logs=[transfer_log(BASE_USDC, EVM_DEPOSIT, 25_000_000)])
    refund = evm_tx(tx_hash=REFUND_TX, block=30_000_005, to=BASE_USDC,
                    logs=[transfer_log(BASE_USDC, REFUND, 1_000, sender=EVM_DEPOSIT)])
    refund["blockHash"] = "0x" + "e" * 64
    rpc, _ = evm_rpc("base", {"https://base-a.invalid": {EVM_TX: deposit, REFUND_TX: refund}})
    ni.verify_deposit(receipt, tx_signature=EVM_TX, evm={"base": rpc})
    assert ni.verify_settlement(receipt, refund_tx=REFUND_TX, evm={"base": rpc})["verified"] is False


# --- the three EVM chains added second, and the 1Click sync check -----------------------------

@pytest.mark.parametrize("source,blockchain,asset_id", [
    ("Abstract.ETH", "abs", "nep141:abs.omdep.near"),
    ("Plasma.USDT0", "plasma", "nep141:plasma-0xb8ce59fc3717ada4c02eadf9682a9e934f625ebb.omft.near"),
    ("Plasma.XPL", "plasma", "nep141:plasma.omft.near"),
    ("Xlayer.OKB", "xlayer", "nep245:v2_1.omni.hot.tg:196_11111111111111111111"),
    ("XLayer.USDC", "xlayer", "nep245:v2_1.omni.hot.tg:196_2dK9kLNR7Ekq7su8FxNGiUW3djTw"),
])
def test_abstract_plasma_and_xlayer_compile(tmp_path, source, blockchain, asset_id):
    _, leg = ni.compile_program(program(tmp_path, source=source), now=NOW)
    assert ni.CHAIN_TO_BLOCKCHAIN[leg["origin"]["chain"]] == blockchain
    assert ni.build_quote_request(leg, tokens(), dry=True)["originAsset"] == asset_id


def test_plasma_usdt0_is_provable_and_xlayer_usdc_is_not_yet():
    asset = ni.evm_origin_asset(asset_receipt(
        "plasma", "nep141:plasma-0xb8ce59fc3717ada4c02eadf9682a9e934f625ebb.omft.near",
        contract="0xb8ce59fc3717ada4c02eadf9682a9e934f625ebb"))
    assert asset == {"chain": "plasma", "kind": "erc20", "token": "0xb8ce59fc3717ada4c02eadf9682a9e934f625ebb"}
    assert ni.evm_origin_asset(asset_receipt("xlayer", "nep245:v2_1.omni.hot.tg:196_11111111111111111111"))["kind"] == "native"
    with pytest.raises(ni.NearIntentsError) as err:
        ni.evm_origin_asset(asset_receipt("xlayer", "nep245:v2_1.omni.hot.tg:196_2dK9kLNR7Ekq7su8FxNGiUW3djTw",
                                          contract="0x74b7f16337b8972027f6196a17a631ac6de26d22"))
    assert err.value.code == "X3_EVIDENCE_UNSUPPORTED"


def test_omni_native_on_the_wrong_chain_is_refused():
    with pytest.raises(ni.NearIntentsError) as err:
        ni.evm_origin_asset(asset_receipt("plasma", "nep245:v2_1.omni.hot.tg:196_11111111111111111111"))
    assert err.value.code == "X3_EVIDENCE_BINDING"


def _sync():
    import importlib.util
    spec = importlib.util.spec_from_file_location("oneclick_chain_sync", ROOT / "tools" / "oneclick_chain_sync.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


#: Every blockchain code in 1Click's live GET /v0/tokens on 2026-11-16 (39).
LIVE_20261116 = ["abs", "adi", "aleo", "aptos", "arb", "avax", "base", "bch", "bera", "bsc", "btc", "cardano",
                 "dash", "doge", "eth", "fogo", "gnosis", "hood", "hypercore", "ltc", "miden", "monad", "movement",
                 "near", "op", "plasma", "pol", "prl", "qtc", "scroll", "sol", "starknet", "stellar", "sui", "ton",
                 "tron", "xlayer", "xrp", "zec"]


def test_every_code_1click_listed_is_routed_or_classified():
    report = _sync().compare(LIVE_20261116, ni.CHAIN_TO_BLOCKCHAIN, ni.ONECLICK_UNROUTED)
    assert report["ok"], report
    assert len(report["routed"]) == 28 and report["live_codes"] == 39


def test_the_sync_check_fails_on_a_new_chain_and_on_a_vanished_route():
    sync = _sync()
    new = sync.compare(LIVE_20261116 + ["newchain"], ni.CHAIN_TO_BLOCKCHAIN, ni.ONECLICK_UNROUTED)
    assert not new["ok"] and new["unclassified"] == ["newchain"]
    gone = sync.compare([c for c in LIVE_20261116 if c != "base"], ni.CHAIN_TO_BLOCKCHAIN, ni.ONECLICK_UNROUTED)
    assert not gone["ok"] and gone["missing_routed"] == ["base"]
    stale = sync.compare([c for c in LIVE_20261116 if c != "qtc"], ni.CHAIN_TO_BLOCKCHAIN, ni.ONECLICK_UNROUTED)
    assert stale["ok"] and stale["stale_unrouted"] == ["qtc"]


def test_the_sync_cli_reads_an_archived_list(tmp_path, capsys):
    path = tmp_path / "tokens.json"
    path.write_text(json.dumps([{"assetId": f"x:{c}", "blockchain": c} for c in LIVE_20261116]))
    assert _sync().main(["--tokens", str(path)]) == 0
    assert json.loads(capsys.readouterr().out)["ok"] is True
    path.write_text(json.dumps([{"assetId": "x", "blockchain": c} for c in LIVE_20261116 + ["zz"]]))
    assert _sync().main(["--tokens", str(path)]) == 1
