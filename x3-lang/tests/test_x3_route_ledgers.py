"""X3 Route on XRP, Sui, Aptos and Movement: checksummed or full-length addresses, XRP payment
evidence (validated, delivered_amount, destination tag = signed memo), Sui coin evidence
(checkpointed, mainnet, balance changes), and Aptos/Movement evidence refused by name."""
from __future__ import annotations

import hashlib
import json
from pathlib import Path

import pytest

from test_near_intents import (  # noqa: F401  (_trust_the_test_key is an autouse fixture)
    NOW, FakeNearChain, RecordingTransport, _trust_the_test_key, ni, near_rpc, sign,
)
from test_x3_route_chains import program, route_api, tokens  # noqa: F401

ROOT = Path(__file__).resolve().parents[1]
ce = ni.chain_evidence
reg = ni.registry
XRP_EXAMPLE = ROOT / "examples" / "x3_route_xrp_to_near.x3"
RIPPLE = "rpshnaf39wBUDNEGHJKLM4PQRST7VWXYZ2bcdeCg65jkm8oFqi1tuvAxyz"
XRP_REFUND = "rrrrrrrrrrrrrrrrrrrrBZbvji"
TX = "A" * 64
REFUND_TX = "B" * 64
T_0301 = 1793674860
SUI_USDC = "0xdba34672e30cb065b1f93e3ab55318768fd6fef66c15942c9f7cb846e2f900e7::usdc::USDC"


def xrp_address(fill: int) -> str:
    payload = b"\x00" + bytes([fill]) * 20
    raw = payload + hashlib.sha256(hashlib.sha256(payload).digest()).digest()[:4]
    n, out = int.from_bytes(raw, "big"), ""
    while n:
        n, rem = divmod(n, 58)
        out = RIPPLE[rem] + out
    return "r" * (len(raw) - len(raw.lstrip(b"\0"))) + out


XRP_DEPOSIT = xrp_address(0xCC)
SUI_ME = "0x" + "a1" * 32
SUI_DEPOSIT = "0x" + "cc" * 32


# --- addresses -------------------------------------------------------------------------------

def test_xrp_addresses():
    assert reg.is_valid_xrp_address("rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTh")  # the genesis account
    assert reg.is_valid_xrp_address(XRP_DEPOSIT) and reg.is_valid_xrp_address(XRP_REFUND)
    for bad in ("rHb9CJAWyB4rj91VRWn96DkukG4bwdtyTi",                 # checksum
                "X7AcgcsBL6XDcUb289X4mJ8djcdyKaB5hJDWMArnXr61cqZ",    # X-address: tag folded in
                "rHb9CJAWyB4rj91VRWn96DkukG4bwdtyT0", "", "x3-demo.near"):
        assert not reg.is_valid_xrp_address(bad), bad


def test_move_addresses_must_be_full_length():
    assert reg.is_valid_move32_address(SUI_ME)
    for bad in ("0x1", "0x" + "a" * 63, "0x" + "g" * 64, "a" * 64):
        assert not reg.is_valid_move32_address(bad), bad


# --- routing --------------------------------------------------------------------------------------

@pytest.mark.parametrize("source,receiver,asset_id", [
    ("Xrp.XRP", XRP_REFUND, "nep141:xrp.omft.near"),
    ("Sui.SUI", SUI_ME, "nep141:sui.omft.near"),
    ("Sui.USDC", SUI_ME, "nep141:sui-c1b81ecaf27933252d31a963bc5e9458f13c18ce.omft.near"),
    ("Aptos.APT", SUI_ME, "nep141:aptos.omft.near"),
    ("Aptos.USDC", SUI_ME, "nep141:aptos-34ee497f210c5a511e8d5b53bc56d75b63612bb5.omft.near"),
    ("Movement.MOVE", SUI_ME, "nep141:movement.omft.near"),
])
def test_each_ledger_compiles_to_a_1click_request(tmp_path, source, receiver, asset_id):
    _, leg = ni.compile_program(program(tmp_path, source=source, receiver=receiver, amount=50_000_000), now=NOW)
    request = ni.build_quote_request(leg, tokens(), dry=True)
    assert request["originAsset"] == asset_id and request["refundTo"] == receiver


def test_a_short_aptos_address_is_refused_as_a_refund(tmp_path):
    with pytest.raises(ni.NearIntentsError) as err:
        ni.compile_program(program(tmp_path, source="Aptos.APT", receiver="0x1"), now=NOW)
    assert err.value.code == "X3_NEAR_TYPECHECK"


# --- XRP evidence ------------------------------------------------------------------------------------

def xrp_tx(*, tx_hash=TX, destination=XRP_DEPOSIT, tag=None, delivered="50000000", amount=None, ledger=90_000_000,
           validated=True, result="tesSUCCESS", tx_type="Payment", date=T_0301 - ce.RIPPLE_EPOCH):
    tx = {"hash": tx_hash, "TransactionType": tx_type, "Destination": destination, "Amount": amount or delivered,
          "ledger_index": ledger, "validated": validated, "date": date, "status": "success",
          "meta": {"TransactionResult": result, "delivered_amount": delivered}}
    if tag is not None:
        tx["DestinationTag"] = tag
    return tx


class FakeRippled:
    def __init__(self, txs, network_id=None):
        self.txs, self.network_id = txs, network_id

    def __call__(self, method, url, payload):
        m = payload["method"]
        if m == "server_info":
            info = {} if self.network_id is None else {"network_id": self.network_id}
            return 200, {"result": {"info": info, "status": "success"}}
        tx = self.txs.get(payload["params"][0]["transaction"])
        if tx is None:
            return 200, {"result": {"error": "txnNotFound", "status": "error"}}
        return 200, {"result": tx}


def xrp_rpc(by_url, **kw):
    fakes = {u: FakeRippled(t, **kw) for u, t in by_url.items()}
    transport = RecordingTransport(lambda m, url, p: fakes[url](m, url, p))
    return ce.XrpRpc(transport=transport, urls=list(by_url))


def xrp_deposit(rpc, *, tag=None, amount=50_000_000):
    return rpc.verify_native_transfer(tx_hash=TX, recipient=XRP_DEPOSIT, min_amount=amount,
                                      not_after_ms=(T_0301 + 600) * 1000, purpose="deposit", destination_tag=tag)


def verdict(ev):
    return {c["check"]: c["ok"] for c in ev["checks"]}


def test_a_validated_xrp_payment_with_the_right_tag_is_verified():
    rpc = xrp_rpc({"https://xrp-a.invalid": {TX: xrp_tx(tag=424242)}, "https://xrp-b.invalid": {TX: xrp_tx(tag=424242)}})
    ev = xrp_deposit(rpc, tag=424242)
    assert ev["verified"] is True, ev["checks"]
    assert ev["kind"] == "xrp_payment" and ev["commitment"] == "validated" and ev["destination_tag"] == 424242


@pytest.mark.parametrize("tx,kw,failed", [
    (xrp_tx(tag=424242, delivered="1", amount="50000000"), {}, "amount"),   # partial payment: Amount lies
    (xrp_tx(tag=111), {}, "destination_tag"),                                # someone else's tag
    (xrp_tx(), {}, "destination_tag"),                                       # no tag at all
    (xrp_tx(tag=424242, validated=False), {}, "validated"),
    (xrp_tx(tag=424242, result="tecPATH_PARTIAL"), {}, "succeeded"),
    (xrp_tx(tag=424242, tx_type="EscrowCreate"), {}, "is_payment"),
    (xrp_tx(tag=424242, destination=XRP_REFUND), {}, "recipient"),
    (xrp_tx(tag=424242, delivered={"currency": "USD", "issuer": XRP_REFUND, "value": "100"}), {}, "amount"),
    (xrp_tx(tag=424242), {"network_id": 1}, "mainnet"),                     # testnet server
    (None, {}, "found"),
])
def test_an_xrp_payment_that_is_not_the_signed_deposit_fails(tx, kw, failed):
    rpc = xrp_rpc({"https://xrp-a.invalid": {TX: tx} if tx else {}}, **kw)
    ev = xrp_deposit(rpc, tag=424242)
    assert ev["verified"] is False and verdict(ev)[failed] is False


# --- Sui evidence -------------------------------------------------------------------------------------

def sui_tx(*, digest="SuiDigest111", owner=SUI_DEPOSIT, coin=SUI_USDC, amount="25000000", status="success",
           checkpoint="200000000", ts=str((T_0301) * 1000)):
    return {"digest": digest, "effects": {"status": {"status": status}}, "checkpoint": checkpoint, "timestampMs": ts,
            "balanceChanges": [{"owner": {"AddressOwner": owner}, "coinType": coin, "amount": amount},
                               {"owner": {"AddressOwner": SUI_ME}, "coinType": coin, "amount": "-" + amount}]}


def sui_rpc(by_url, chain_id=ce.SUI_MAINNET_CHAIN_ID):
    def handler(method, url, payload):
        if payload["method"] == "sui_getChainIdentifier":
            result = chain_id
        else:
            result = by_url[url].get(payload["params"][0])
        return 200, {"jsonrpc": "2.0", "id": payload["id"], "result": result}
    return ce.SuiRpc(transport=RecordingTransport(handler), urls=list(by_url))


def sui_deposit(rpc, amount=25_000_000, digest="SuiDigest111"):
    return rpc.verify_token_transfer(tx_hash=digest, token=SUI_USDC, recipient=SUI_DEPOSIT, min_amount=amount,
                                     not_after_ms=(T_0301 + 600) * 1000, purpose="deposit")


def test_a_checkpointed_sui_usdc_deposit_is_verified():
    rpc = sui_rpc({"https://sui-a.invalid": {"SuiDigest111": sui_tx()}})
    ev = sui_deposit(rpc)
    assert ev["verified"] is True and ev["kind"] == "sui_coin_transfer" and ev["commitment"] == "checkpointed"


def test_short_and_long_coin_type_spellings_compare_equal():
    tx = sui_tx(coin="0x2::sui::SUI", amount="1000000000")
    rpc = sui_rpc({"https://sui-a.invalid": {"SuiDigest111": tx}})
    ev = rpc.verify_native_transfer(tx_hash="SuiDigest111", recipient=SUI_DEPOSIT, min_amount=10**9,
                                    not_after_ms=None, purpose="deposit")
    assert ev["verified"] is True and ev["kind"] == "sui_native_transfer"
    assert ce.sui_coin_type("0x2::sui::SUI") == ce.sui_coin_type("0x" + "0" * 63 + "2::sui::SUI")


@pytest.mark.parametrize("tx,chain_id,failed", [
    (sui_tx(checkpoint=None), ce.SUI_MAINNET_CHAIN_ID, "finalized"),
    (sui_tx(status="failure"), ce.SUI_MAINNET_CHAIN_ID, "succeeded"),
    (sui_tx(coin="0x" + "9" * 64 + "::usdc::USDC"), ce.SUI_MAINNET_CHAIN_ID, "recipient"),   # a fake USDC
    (sui_tx(owner=SUI_ME), ce.SUI_MAINNET_CHAIN_ID, "recipient"),
    (sui_tx(amount="24999999"), ce.SUI_MAINNET_CHAIN_ID, "amount"),
    (sui_tx(), "4c78adac", "mainnet"),                                                          # a testnet node
])
def test_a_sui_deposit_that_is_not_a_final_mainnet_payment_fails(tx, chain_id, failed):
    rpc = sui_rpc({"https://sui-a.invalid": {"SuiDigest111": tx}}, chain_id=chain_id)
    ev = sui_deposit(rpc)
    assert ev["verified"] is False and verdict(ev)[failed] is False


# --- binding and memos ---------------------------------------------------------------------------------

def ledger_receipt(chain, asset_id, contract=None, memo=None):
    origin = {"assetId": asset_id, "blockchain": ni.CHAIN_TO_BLOCKCHAIN[chain]}
    if contract:
        origin["contractAddress"] = contract
    quote = {} if memo is None else {"depositMemo": memo}
    return {"leg": {"origin": {"chain": chain}},
            "quote_response": {"quoteRequest": {"originAsset": asset_id}, "quote": quote}, "tokens": {"origin": origin}}


def test_sui_usdc_binds_to_its_reviewed_coin_type():
    asset = ni.ledger_origin_asset(ledger_receipt("sui", "nep141:sui-c1b81ecaf27933252d31a963bc5e9458f13c18ce.omft.near",
                                                  SUI_USDC))
    assert asset == {"chain": "sui", "kind": "coin", "token": SUI_USDC}
    with pytest.raises(ni.NearIntentsError) as err:
        ni.ledger_origin_asset(ledger_receipt("sui", "nep141:sui-c1b81ecaf27933252d31a963bc5e9458f13c18ce.omft.near",
                                              "0x" + "9" * 64 + "::usdc::USDC"))
    assert err.value.code == "X3_EVIDENCE_BINDING"


def test_a_signed_memo_becomes_the_required_xrp_tag_and_bad_memos_are_refused():
    _, asset = ni._block_origin(ledger_receipt("xrp", "nep141:xrp.omft.near", memo="424242"),
                                evm=None, bsc=None, tron=None, xrp=object())
    assert asset["deposit_tag"] == 424242
    for memo in ("not-a-tag", str(2 ** 32)):
        with pytest.raises(ni.NearIntentsError) as err:
            ni._block_origin(ledger_receipt("xrp", "nep141:xrp.omft.near", memo=memo),
                             evm=None, bsc=None, tron=None, xrp=object())
        assert err.value.code == "X3_EVIDENCE_UNSUPPORTED"


def test_a_memo_on_a_chain_that_cannot_check_one_refuses_evidence():
    receipt = ledger_receipt("sui", "nep141:sui.omft.near", memo="123")
    with pytest.raises(ni.NearIntentsError) as err:
        ni._block_origin(receipt, evm=None, bsc=None, tron=None, sui=object())
    assert err.value.code == "X3_EVIDENCE_UNSUPPORTED" and "memo" in err.value.message


@pytest.mark.parametrize("chain,asset_id", [("aptos", "nep141:aptos.omft.near"),
                                            ("movement", "nep141:movement.omft.near")])
def test_aptos_and_movement_evidence_is_refused_by_name(chain, asset_id):
    with pytest.raises(ni.NearIntentsError) as err:
        ni._block_origin(ledger_receipt(chain, asset_id), evm=None, bsc=None, tron=None)
    assert err.value.code == "X3_EVIDENCE_UNSUPPORTED" and "primary store" in err.value.message


# --- end to end: a signed XRP quote with a deposit memo --------------------------------------------------

def xrp_quote(request):
    quote = {"amountIn": request["amount"], "amountInFormatted": "50.0", "amountInUsd": "30.000",
             "minAmountIn": request["amount"], "amountOut": "29940000", "amountOutFormatted": "29.94",
             "amountOutUsd": "29.940", "minAmountOut": str(29_940_000 * (10_000 - request["slippageTolerance"]) // 10_000),
             "timeEstimate": 30, "refundFee": "100000"}
    if not request["dry"]:
        quote.update({"depositAddress": XRP_DEPOSIT, "depositMemo": "424242", "deadline": request["deadline"],
                      "timeWhenInactive": request["deadline"]})
    return sign({"correlationId": "xrp-c0ffee", "timestamp": "2026-11-03T03:00:00.000Z", "signature": "",
                 "quoteRequest": dict(request), "quote": quote})


def live_xrp_receipt(tmp_path):
    rpc, _ = near_rpc(chain=FakeNearChain(balances={}))
    out = tmp_path / "xrp.json"
    ni.quote(str(XRP_EXAMPLE), client=ni.OneClickClient(jwt="k", transport=route_api(xrp_quote)), live=True,
             nonce_store=ni.NonceStore(str(tmp_path / "n.json")), receipt_path=str(out), rpc=rpc, now=NOW)
    return json.loads(out.read_text())


def test_an_xrp_deposit_needs_the_signed_tag_and_the_refund_is_proven(tmp_path):
    receipt = live_xrp_receipt(tmp_path)
    untagged = xrp_rpc({"https://xrp-a.invalid": {TX: xrp_tx()}})
    assert ni.verify_deposit(receipt, tx_signature=TX, xrp=untagged)["verified"] is False
    dep = xrp_tx(tag=424242)
    ref = xrp_tx(tx_hash=REFUND_TX, destination=XRP_REFUND, delivered="49900000", ledger=90_000_200)
    rpc = xrp_rpc({"https://xrp-a.invalid": {TX: dep, REFUND_TX: ref}})
    assert ni.verify_deposit(receipt, tx_signature=TX, xrp=rpc)["verified"] is True
    refund = ni.verify_settlement(receipt, refund_tx=REFUND_TX, xrp=rpc)
    assert refund["verified"] is True, refund["checks"]
    assert ni.conclude(receipt)["outcome"] == "refunded"
