"""X3 Route on UTXO chains: Bitcoin, Litecoin, Dogecoin, Bitcoin Cash, Zcash and Dash. Checksummed
addresses per chain, and native-coin deposit/refund evidence from each chain's node RPC."""
from __future__ import annotations

import hashlib
import json
from pathlib import Path

import pytest

from test_near_intents import (  # noqa: F401  (_trust_the_test_key is an autouse fixture)
    NOW, FakeNearChain, _trust_the_test_key, ni, near_rpc, sign,
)
from test_x3_route_chains import program, route_api, tokens  # noqa: F401

ROOT = Path(__file__).resolve().parents[1]
ce = ni.chain_evidence
reg = ni.registry
DOGE_EXAMPLE = ROOT / "examples" / "x3_route_doge_to_near.x3"
TX = "aa" * 32
REFUND_TX = "bb" * 32
T_0301 = 1793674860  # 2026-11-03T03:01:00Z


def b58c(payload: bytes) -> str:
    return reg.base58_encode(payload + hashlib.sha256(hashlib.sha256(payload).digest()).digest()[:4])


def addr(chain: str, fill: int) -> str:
    h = bytes([fill]) * 20
    return {"dogecoin": b58c(b"\x1e" + h), "litecoin": reg.segwit_address("ltc", 0, h),
            "bitcoincash": reg.cashaddr("p2pkh", h), "zcash": b58c(b"\x1c\xb8" + h),
            "dash": b58c(b"\x4c" + h), "bitcoin": reg.segwit_address("bc", 0, h)}[chain]


DOGE_REFUND = "D596YFweJQuHY1BbjazZYmAbt8jJXaDhSF"
DOGE_DEPOSIT = addr("dogecoin", 0xCC)


# --- addresses --------------------------------------------------------------------------------

@pytest.mark.parametrize("kind,address", [
    ("btc", "1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa"),
    ("btc", "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4"),
    ("btc", "bc1p0xlxvlhemja6c4dqv22uapctqupfhlxm9h8z3k2e72q4k9hcz7vqzk5jj0"),
    ("bch", "bitcoincash:qpm2qsznhks23z7629mms6s4cwef74vcwvy22gdx6a"),
    ("bch", "qpm2qsznhks23z7629mms6s4cwef74vcwvy22gdx6a"),
    ("doge", "DH5yaieqoZN36fDVciNyRueRGvGLR3mr7L"),
    ("ltc", "LaMT348PWRnrqeeWArpwQPbuanpXDZGEUz"),
    ("ltc", "ltc1qg42tkwuuxefutzxezdkdel39gfstuap288mfea"),
    ("dash", "XpESxaUmonkq8RaLLp46Brx2K39ggQe226"),
    ("zec", addr("zcash", 0x11)),
    ("zec", b58c(b"\x1c\xbd" + b"\x11" * 20)),
])
def test_valid_addresses(kind, address):
    assert reg.utxo_address_kind(kind, address) is not None


@pytest.mark.parametrize("kind,address", [
    ("btc", "1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNb"),                         # checksum
    ("btc", "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t5"),                 # bech32 checksum
    ("btc", "BC1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4"),                 # mixed case
    ("ltc", "1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa"),                         # a Bitcoin address
    ("doge", "LaMT348PWRnrqeeWArpwQPbuanpXDZGEUz"),                        # a Litecoin address
    ("bch", "1BpEi6DfDAUFd7GtittLSdBeYJvcoaVggu"),                         # legacy BCH looks like BTC
    ("bch", "bitcoincash:qpm2qsznhks23z7629mms6s4cwef74vcwvy22gdx6b"),     # cashaddr checksum
    ("bch", "bchtest:qpm2qsznhks23z7629mms6s4cwef74vcwvy22gdx6a"),         # testnet prefix
    ("zec", "zs1" + "q" * 75),                                             # shielded: unprovable
    ("dash", "DH5yaieqoZN36fDVciNyRueRGvGLR3mr7L"),                        # a Dogecoin address
])
def test_invalid_or_wrong_chain_addresses(kind, address):
    assert reg.utxo_address_kind(kind, address) is None


def test_canonical_forms_compare_equal():
    assert reg.canonical_utxo_address("bch", "qpm2qsznhks23z7629mms6s4cwef74vcwvy22gdx6a") == \
        reg.canonical_utxo_address("bch", "BITCOINCASH:QPM2QSZNHKS23Z7629MMS6S4CWEF74VCWVY22GDX6A")
    assert reg.canonical_utxo_address("ltc", "LTC1QG42TKWUUXEFUTZXEZDKDEL39GFSTUAP288MFEA") == \
        "ltc1qg42tkwuuxefutzxezdkdel39gfstuap288mfea"


# --- routing ---------------------------------------------------------------------------------------

@pytest.mark.parametrize("source,chain,asset_id", [
    ("Dogecoin.DOGE", "dogecoin", "nep141:doge.omft.near"),
    ("Litecoin.LTC", "litecoin", "nep141:ltc.omft.near"),
    ("Bitcoincash.BCH", "bitcoincash", "nep141:bch.omft.near"),
    ("Zcash.ZEC", "zcash", "nep141:zec.omft.near"),
    ("Dash.DASH", "dash", "nep141:dash.omft.near"),
    ("Bitcoin.BTC", "bitcoin", "nep141:btc.omft.near"),
])
def test_each_utxo_chain_compiles_to_a_1click_request(tmp_path, source, chain, asset_id):
    refund = addr(chain, 0x21)
    _, leg = ni.compile_program(program(tmp_path, source=source, receiver=refund, amount=100_000_000), now=NOW)
    request = ni.build_quote_request(leg, tokens(), dry=True)
    assert request["originAsset"] == asset_id and request["refundTo"] == refund


def test_a_bitcoin_address_is_refused_as_a_litecoin_refund(tmp_path):
    with pytest.raises(ni.NearIntentsError) as err:
        ni.compile_program(program(tmp_path, source="Litecoin.LTC", receiver="1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa",
                                   amount=100_000_000), now=NOW)
    assert err.value.code == "X3_NEAR_TYPECHECK"


def test_the_1click_path_demands_a_checksummed_bitcoin_address(tmp_path):
    # Shaped like an address, so the language's loose check accepts it; its checksum is wrong.
    bogus = "1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNb"
    assert reg.is_valid_receiver("bitcoin", bogus)
    with pytest.raises(ni.NearIntentsError) as err:
        ni.compile_program(program(tmp_path, source="Bitcoin.BTC", receiver=bogus, amount=100_000), now=NOW)
    assert err.value.code == "X3_NEAR_REFUND_TO"


# --- evidence ---------------------------------------------------------------------------------------

class FakeNode:
    """A bitcoind-style JSON-RPC node. Values are written as JSON text, exactly as a node sends them."""

    def __init__(self, txs, *, network="main", tip=1_000_100):
        self.txs, self.network, self.tip, self.calls = txs, network, tip, []

    def __call__(self, method, url, headers, body, timeout):
        payload = json.loads(body)
        self.calls.append(payload["method"])
        m, params = payload["method"], payload["params"]
        if m == "getblockchaininfo":
            result = json.dumps({"chain": self.network, "blocks": self.tip})
        elif m == "getrawtransaction":
            tx = self.txs.get(params[0])
            if tx is None:
                return 200, b'{"result":null,"error":{"code":-5,"message":"No such mempool or blockchain transaction"},"id":"x"}'
            outs = ",".join(f'{{"value":{v},"n":{i},"scriptPubKey":{{"address":"{a}"}}}}'
                            for i, (a, v) in enumerate(tx["outputs"]))
            block = f',"blockhash":"{tx["block"]}"' if tx.get("height") is not None else ""
            result = f'{{"txid":"{tx["txid"]}","vout":[{outs}]{block}}}'
        elif m == "getblockheader":
            tx = next(t for t in self.txs.values() if t["block"] == params[0])
            result = json.dumps({"hash": tx["block"], "height": tx["height"], "time": tx["time"],
                                 "confirmations": self.tip - tx["height"] + 1})
        else:
            raise AssertionError(m)
        return 200, f'{{"result":{result},"error":null,"id":"x"}}'.encode()


def utxo_tx(txid=TX, outputs=((DOGE_DEPOSIT, "100.00000000"),), height=1_000_000, time=T_0301):
    return {"txid": txid, "outputs": list(outputs), "height": height, "time": time, "block": f"{height:064x}"}


def node_rpc(chain, by_url, **kw):
    nodes = {url: FakeNode(txs, **kw) for url, txs in by_url.items()}
    transport = lambda method, url, headers, body, timeout: nodes[url](method, url, headers, body, timeout)  # noqa: E731
    return ce.UtxoRpc(chain, transport=transport, urls=list(by_url)), nodes


def deposit(rpc, amount=10_000_000_000, recipient=DOGE_DEPOSIT):
    return rpc.verify_native_transfer(tx_hash=TX, recipient=recipient, min_amount=amount,
                                      not_after_ms=(T_0301 + 600) * 1000, purpose="deposit")


def verdict(ev):
    return {c["check"]: c["ok"] for c in ev["checks"]}


def test_a_deep_enough_doge_deposit_is_verified_to_the_satoshi():
    # Change output to someone else is ignored; the deposit output is exact.
    tx = utxo_tx(outputs=[(DOGE_DEPOSIT, "100.00000000"), (addr("dogecoin", 0x99), "3.14159265")])
    rpc, _ = node_rpc("dogecoin", {"https://doge-a.invalid": {TX: tx}, "https://doge-b.invalid": {TX: tx}})
    ev = deposit(rpc)
    assert ev["verified"] is True, ev["checks"]
    assert ev["observed"] == "10000000000" and ev["commitment"] == "40 confirmations" and len(ev["rpcs"]) == 2


def test_a_value_that_is_not_whole_satoshis_is_a_format_error():
    rpc, _ = node_rpc("dogecoin", {"https://doge-a.invalid": {TX: utxo_tx(outputs=[(DOGE_DEPOSIT, "100.000000001")])}})
    with pytest.raises(ce.ChainEvidenceError) as err:
        deposit(rpc)
    assert err.value.code == "X3_EVIDENCE_FORMAT"


@pytest.mark.parametrize("tx,kw,failed", [
    (None, {}, "found"),                                                             # node has never seen it
    (utxo_tx(height=1_000_090), {}, "confirmed"),                                     # 11 deep, needs 40
    (utxo_tx(), {"network": "test"}, "mainnet"),                                      # a testnet node
    (utxo_tx(outputs=[(addr("dogecoin", 0x77), "100.0")]), {}, "recipient"),          # paid someone else
    (utxo_tx(outputs=[(DOGE_DEPOSIT, "99.99999999")]), {}, "amount"),
    (utxo_tx(time=T_0301 + 3600), {}, "before_deadline"),
])
def test_a_doge_deposit_that_is_not_a_final_payment_fails(tx, kw, failed):
    rpc, _ = node_rpc("dogecoin", {"https://doge-a.invalid": {TX: tx} if tx else {}}, **kw)
    ev = deposit(rpc)
    assert ev["verified"] is False and verdict(ev)[failed] is False


def test_utxo_endpoints_that_disagree_fail():
    a = utxo_tx()
    b = utxo_tx(outputs=[(DOGE_DEPOSIT, "101.0")])
    rpc, _ = node_rpc("dogecoin", {"https://doge-a.invalid": {TX: a}, "https://doge-b.invalid": {TX: b}})
    assert verdict(deposit(rpc))["endpoints_agree"] is False


def test_a_bch_payment_matches_whichever_cashaddr_spelling_the_node_uses():
    to = addr("bitcoincash", 0x33)
    bare = to.split(":", 1)[1]
    rpc, _ = node_rpc("bitcoincash", {"https://bch-a.invalid": {TX: utxo_tx(outputs=[(bare, "1.5")])}})
    ev = rpc.verify_native_transfer(tx_hash=TX, recipient=to.upper(), min_amount=150_000_000, not_after_ms=None,
                                    purpose="deposit")
    assert ev["verified"] is True, ev["checks"]


def test_utxo_chains_need_configured_endpoints(monkeypatch):
    monkeypatch.delenv("X3_UTXO_RPC_URLS_DOGECOIN", raising=False)
    with pytest.raises(ce.ChainEvidenceError) as err:
        ce.UtxoRpc("dogecoin", transport=lambda *a: (200, b"{}"))
    assert err.value.code == "X3_EVIDENCE_RPC"


# --- binding -----------------------------------------------------------------------------------------

def utxo_receipt(chain, asset_id, blockchain=None, contract=None):
    origin = {"assetId": asset_id, "blockchain": blockchain or ni.CHAIN_TO_BLOCKCHAIN[chain]}
    if contract:
        origin["contractAddress"] = contract
    return {"leg": {"origin": {"chain": chain}}, "quote_response": {"quoteRequest": {"originAsset": asset_id}},
            "tokens": {"origin": origin}}


def test_utxo_assets_bind_only_to_their_pinned_native_listing():
    assert ni.utxo_origin_asset(utxo_receipt("dogecoin", "nep141:doge.omft.near"))["kind"] == "native"
    with pytest.raises(ni.NearIntentsError) as err:   # BTC's listing presented as a Dogecoin deposit
        ni.utxo_origin_asset(utxo_receipt("dogecoin", "nep141:btc.omft.near", blockchain="doge"))
    assert err.value.code == "X3_EVIDENCE_UNSUPPORTED"
    with pytest.raises(ni.NearIntentsError) as err:   # 1Click's second BTC listing is not reviewed
        ni.utxo_origin_asset(utxo_receipt("bitcoin", "1cs_v1:btc:native:coin"))
    assert err.value.code == "X3_EVIDENCE_UNSUPPORTED"
    with pytest.raises(ni.NearIntentsError) as err:
        ni.utxo_origin_asset(utxo_receipt("dogecoin", "nep141:doge.omft.near", contract="0x" + "9" * 40))
    assert err.value.code == "X3_EVIDENCE_BINDING"


# --- end to end -------------------------------------------------------------------------------------

def doge_quote(request):
    quote = {"amountIn": request["amount"], "amountInFormatted": "100.0", "amountInUsd": "20.000",
             "minAmountIn": request["amount"], "amountOut": "19950000", "amountOutFormatted": "19.95",
             "amountOutUsd": "19.950", "minAmountOut": str(19_950_000 * (10_000 - request["slippageTolerance"]) // 10_000),
             "timeEstimate": 900, "refundFee": "10000000"}
    if not request["dry"]:
        quote.update({"depositAddress": DOGE_DEPOSIT, "deadline": request["deadline"],
                      "timeWhenInactive": request["deadline"]})
    return sign({"correlationId": "doge-c0ffee", "timestamp": "2026-11-03T03:00:00.000Z", "signature": "",
                 "quoteRequest": dict(request), "quote": quote})


def live_doge_receipt(tmp_path):
    rpc, _ = near_rpc(chain=FakeNearChain(balances={}))
    out = tmp_path / "doge.json"
    ni.quote(str(DOGE_EXAMPLE), client=ni.OneClickClient(jwt="k", transport=route_api(doge_quote)), live=True,
             nonce_store=ni.NonceStore(str(tmp_path / "n.json")), receipt_path=str(out), rpc=rpc, now=NOW)
    return json.loads(out.read_text())


def test_a_doge_deposit_and_refund_are_proven_against_the_signed_quote(tmp_path):
    receipt = live_doge_receipt(tmp_path)
    dep = utxo_tx()
    ref = utxo_tx(txid=REFUND_TX, outputs=[(DOGE_REFUND, "99.90000000")], height=1_000_050, time=T_0301 + 2400)
    rpc, _ = node_rpc("dogecoin", {"https://doge-a.invalid": {TX: dep, REFUND_TX: ref}}, tip=1_000_200)
    evidence = ni.verify_deposit(receipt, tx_signature=TX, utxo={"dogecoin": rpc})
    assert evidence["verified"] is True and evidence["kind"] == "utxo_native_transfer", [c for c in evidence["checks"] if not c["ok"]]
    refund = ni.verify_settlement(receipt, refund_tx=REFUND_TX, utxo={"dogecoin": rpc})
    assert refund["verified"] is True, refund["checks"]
    assert refund["recipient"] == DOGE_REFUND and ni.conclude(receipt)["outcome"] == "refunded"


def test_a_doge_refund_in_an_earlier_block_than_the_deposit_is_not_proven(tmp_path):
    receipt = live_doge_receipt(tmp_path)
    dep = utxo_tx()
    ref = utxo_tx(txid=REFUND_TX, outputs=[(DOGE_REFUND, "99.9")], height=999_999)
    rpc, _ = node_rpc("dogecoin", {"https://doge-a.invalid": {TX: dep, REFUND_TX: ref}}, tip=1_000_200)
    ni.verify_deposit(receipt, tx_signature=TX, utxo={"dogecoin": rpc})
    ev = ni.verify_settlement(receipt, refund_tx=REFUND_TX, utxo={"dogecoin": rpc})
    assert ev["verified"] is False and verdict(ev)["after_deposit"] is False
