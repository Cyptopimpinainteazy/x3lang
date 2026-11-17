"""X3 Route on Tron and TON: checksummed addresses, Tron TRX/TRC-20 deposit and refund evidence from
the solidity API, and TON routing whose deposit evidence is refused by name until it can be proven."""
from __future__ import annotations

import base64
import binascii
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
TRON_EXAMPLE = ROOT / "examples" / "x3_route_tron_usdt_to_near.x3"
TRON_USDT = "TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t"
TRON_USDT_HEX = "41a614f803b6fd780986a42c78ec9c7f77e6ded13c"
TRON_REFUND = "T9yD14Nj9j7xAB4dbGeiX9h8unkKLxmGkn"
TRON_DEPOSIT = reg.tron_address_from_hex("41" + "cc" * 20)
TRON_TX = "ab" * 32
REFUND_TX = "cd" * 32
DEADLINE_MS = 1793675400000  # 2026-11-03T03:10:00Z, the example's 10-minute timeout
TON_USDT_MASTER = "EQCxE6mUtQJKFnGfaROTKOt1lZbDiiX1kCixRv7Nw2Id_sDs"


def ton_address(byte: int, *, tag: int = 0x51, workchain: int = 0x00) -> str:
    body = bytes([tag, workchain]) + bytes([byte]) * 32
    return base64.urlsafe_b64encode(body + binascii.crc_hqx(body, 0).to_bytes(2, "big")).decode()


# --- addresses ------------------------------------------------------------------------------

def test_tron_addresses_need_a_correct_checksum():
    assert reg.is_valid_tron_address(TRON_USDT)
    assert reg.tron_address_bytes(TRON_USDT).hex() == TRON_USDT_HEX
    assert reg.tron_address_from_hex(TRON_USDT_HEX) == TRON_USDT
    for bad in (TRON_USDT[:-1] + "u", "A" + TRON_USDT[1:], TRON_USDT[:-1], "0x" + "a" * 40, ""):
        assert not reg.is_valid_tron_address(bad), bad


def test_ton_addresses_need_a_correct_crc_and_mainnet():
    assert reg.is_valid_ton_address(TON_USDT_MASTER)
    assert reg.is_valid_ton_address(ton_address(0xCC))
    assert reg.is_valid_ton_address(ton_address(0xCC, tag=0x11, workchain=0xFF))
    assert reg.is_valid_ton_address("0:" + "ab" * 32) and reg.is_valid_ton_address("-1:" + "ab" * 32)
    assert not reg.is_valid_ton_address(TON_USDT_MASTER[:-1] + "t")         # CRC
    assert not reg.is_valid_ton_address(ton_address(0xCC, tag=0x51 | 0x80))  # testnet flag
    assert not reg.is_valid_ton_address(ton_address(0xCC, workchain=0x05))
    assert not reg.is_valid_ton_address("2:" + "ab" * 32)
    assert not reg.is_valid_ton_address("x3-demo.near")


# --- routing --------------------------------------------------------------------------------

@pytest.mark.parametrize("source,receiver,asset_id", [
    ("Tron.USDT", TRON_REFUND, "nep141:tron-d28a265909efecdcee7c5028585214ea0b96f015.omft.near"),
    ("Tron.TRX", TRON_REFUND, "nep141:tron.omft.near"),
    ("Ton.USDT", ton_address(0x11), "nep245:v2_1.omni.hot.tg:1117_3tsdfyziyc7EJbP2aULWSKU4toBaAcN4FdTgfm5W1mC4ouR"),
    ("Ton.TON", ton_address(0x11), "nep245:v2_1.omni.hot.tg:1117_"),
])
def test_tron_and_ton_compile_to_1click_requests(tmp_path, source, receiver, asset_id):
    _, leg = ni.compile_program(program(tmp_path, source=source, receiver=receiver), now=NOW)
    request = ni.build_quote_request(leg, tokens(), dry=True)
    assert request["originAsset"] == asset_id and request["refundTo"] == receiver


def test_payouts_to_tron_and_ton_need_valid_addresses(tmp_path):
    for dest, good in (("Tron.USDT", TRON_REFUND), ("Ton.USDT", ton_address(0x22))):
        _, leg = ni.compile_program(program(tmp_path, source="Near.USDC", dest=dest, receiver="x3-owner.near",
                                            to_receiver=good), now=NOW)
        assert ni.build_quote_request(leg, tokens(), dry=True)["recipient"] == good
    for dest, bad in (("Tron.USDT", TRON_REFUND[:-1] + "x"), ("Ton.USDT", ton_address(0x22)[:-1] + "A")):
        with pytest.raises(ni.NearIntentsError) as err:
            ni.compile_program(program(tmp_path, source="Near.USDC", dest=dest, receiver="x3-owner.near",
                                       to_receiver=bad), now=NOW)
        assert err.value.code == "X3_NEAR_TYPECHECK"
        assert "X3_INVALID_RECEIVER" in {e["code"] for e in err.value.detail}


def test_a_tron_refund_with_a_bad_checksum_is_refused(tmp_path):
    with pytest.raises(ni.NearIntentsError) as err:
        ni.compile_program(program(tmp_path, source="Tron.USDT", receiver=TRON_REFUND[:-1] + "x"), now=NOW)
    assert err.value.code == "X3_NEAR_TYPECHECK"


# --- Tron evidence ----------------------------------------------------------------------------

def trc20_log(to_base58, value, token_hex=TRON_USDT_HEX):
    to_hex = reg.tron_address_bytes(to_base58).hex()[2:]
    return {"address": token_hex[2:], "topics": [ce.TRANSFER_TOPIC_HEX, "0" * 24 + "e" * 40, "0" * 24 + to_hex],
            "data": format(value, "064x")}


def tron_tx(*, tx_id=TRON_TX, logs=(), native_to=None, native_amount=0, ret="SUCCESS", block=70_000_000,
            ts_ms=DEADLINE_MS - 300_000, receipt_result="SUCCESS"):
    contract = ({"type": "TransferContract", "parameter": {"value": {
        "amount": native_amount, "owner_address": "41" + "e" * 40,
        "to_address": reg.tron_address_bytes(native_to).hex()}}} if native_to else
        {"type": "TriggerSmartContract", "parameter": {"value": {"contract_address": TRON_USDT_HEX}}})
    tx = {"txID": tx_id, "raw_data": {"contract": [contract]}, "ret": [{"contractRet": ret}]}
    info = {"id": tx_id, "blockNumber": block, "blockTimeStamp": ts_ms, "receipt": {"result": receipt_result},
            "log": list(logs)}
    return tx, info


class FakeTron:
    def __init__(self, by_url):
        self.by_url, self.calls = by_url, []

    def __call__(self, method, url, payload):
        self.calls.append((url, payload))
        base, path = url.split("/wallet", 1)
        entry = self.by_url[base].get(payload["value"])
        if entry is None:  # not solidified (or unknown): the solidity API answers {}
            return 200, {}
        tx, info = entry
        return 200, tx if path == "solidity/gettransactionbyid" else info


def tron_rpc(by_url):
    fake = FakeTron(by_url)
    return ce.TronRpc(transport=RecordingTransport(fake), urls=list(by_url)), fake


def verdict(ev):
    return {c["check"]: c["ok"] for c in ev["checks"]}


def usdt_deposit(rpc, amount=25_000_000):
    return rpc.verify_token_transfer(tx_hash=TRON_TX, token=TRON_USDT, recipient=TRON_DEPOSIT, min_amount=amount,
                                     not_after_ms=DEADLINE_MS, purpose="deposit")


def test_a_solidified_usdt_deposit_on_tron_is_verified():
    tx = tron_tx(logs=[trc20_log(TRON_DEPOSIT, 25_000_000)])
    rpc, _ = tron_rpc({"https://tron-a.invalid": {TRON_TX: tx}, "https://tron-b.invalid": {TRON_TX: tx}})
    ev = usdt_deposit(rpc)
    assert ev["verified"] is True, ev["checks"]
    assert ev["kind"] == "tron_token_transfer" and ev["commitment"] == "solidified" and len(ev["rpcs"]) == 2


@pytest.mark.parametrize("entry,failed", [
    (None, "solidified"),                                                                  # not yet irreversible
    (tron_tx(logs=[trc20_log(TRON_DEPOSIT, 25_000_000, token_hex="41" + "9" * 40)]), "recipient"),  # fake USDT
    (tron_tx(logs=[trc20_log(TRON_REFUND, 25_000_000)]), "recipient"),                     # paid someone else
    (tron_tx(logs=[trc20_log(TRON_DEPOSIT, 24_999_999)]), "amount"),
    (tron_tx(logs=[trc20_log(TRON_DEPOSIT, 25_000_000)], ret="REVERT"), "succeeded"),
    (tron_tx(logs=[trc20_log(TRON_DEPOSIT, 25_000_000)], receipt_result="OUT_OF_ENERGY"), "succeeded"),
    (tron_tx(logs=[trc20_log(TRON_DEPOSIT, 25_000_000)], ts_ms=DEADLINE_MS + 1), "before_deadline"),
])
def test_a_tron_deposit_that_is_not_a_final_usdt_payment_fails(entry, failed):
    rpc, _ = tron_rpc({"https://tron-a.invalid": {TRON_TX: entry} if entry else {}})
    ev = usdt_deposit(rpc)
    assert ev["verified"] is False and verdict(ev)[failed] is False


def test_tron_endpoints_that_disagree_fail():
    a = tron_tx(logs=[trc20_log(TRON_DEPOSIT, 25_000_000)])
    b = tron_tx(logs=[trc20_log(TRON_DEPOSIT, 26_000_000)])
    rpc, _ = tron_rpc({"https://tron-a.invalid": {TRON_TX: a}, "https://tron-b.invalid": {TRON_TX: b}})
    assert verdict(usdt_deposit(rpc))["endpoints_agree"] is False


def test_a_native_trx_deposit_is_verified():
    rpc, _ = tron_rpc({"https://tron-a.invalid": {TRON_TX: tron_tx(native_to=TRON_DEPOSIT, native_amount=10**8)}})
    ev = rpc.verify_native_transfer(tx_hash=TRON_TX, recipient=TRON_DEPOSIT, min_amount=10**8, not_after_ms=None,
                                    purpose="deposit")
    assert ev["verified"] is True and ev["kind"] == "tron_native_transfer"


def test_tron_needs_configured_endpoints(monkeypatch):
    monkeypatch.delenv("X3_TRON_RPC_URLS", raising=False)
    with pytest.raises(ce.ChainEvidenceError) as err:
        ce.TronRpc(transport=lambda *a: (200, b"{}"))
    assert err.value.code == "X3_EVIDENCE_RPC"


# --- binding --------------------------------------------------------------------------------------

def tron_receipt(asset_id, contract=None):
    origin = {"assetId": asset_id, "blockchain": "tron"}
    if contract:
        origin["contractAddress"] = contract
    return {"leg": {"origin": {"chain": "tron"}}, "quote_response": {"quoteRequest": {"originAsset": asset_id}},
            "tokens": {"origin": origin}}


def test_tron_usdt_binds_to_the_reviewed_contract_not_to_the_id_shape():
    asset_id = "nep141:tron-d28a265909efecdcee7c5028585214ea0b96f015.omft.near"
    assert ni.tron_origin_asset(tron_receipt(asset_id, TRON_USDT)) == {"chain": "tron", "kind": "trc20",
                                                                       "token": TRON_USDT}
    # The hex in the asset ID is not the contract: claiming it is must be refused.
    impostor = reg.tron_address_from_hex("41d28a265909efecdcee7c5028585214ea0b96f015")
    with pytest.raises(ni.NearIntentsError) as err:
        ni.tron_origin_asset(tron_receipt(asset_id, impostor))
    assert err.value.code == "X3_EVIDENCE_BINDING"
    with pytest.raises(ni.NearIntentsError) as err:
        ni.tron_origin_asset(tron_receipt("nep141:tron-" + "1" * 40 + ".omft.near", TRON_USDT))
    assert err.value.code == "X3_EVIDENCE_UNSUPPORTED"
    assert ni.tron_origin_asset(tron_receipt("nep141:tron.omft.near"))["kind"] == "native"


# --- end to end ---------------------------------------------------------------------------------

def tron_quote(request):
    quote = {"amountIn": request["amount"], "amountInFormatted": "25.0", "amountInUsd": "25.000",
             "minAmountIn": request["amount"], "amountOut": "24950000", "amountOutFormatted": "24.95",
             "amountOutUsd": "24.950", "minAmountOut": str(24_950_000 * (10_000 - request["slippageTolerance"]) // 10_000),
             "timeEstimate": 60, "refundFee": "200000"}
    if not request["dry"]:
        deposit = TRON_DEPOSIT if request["originAsset"].startswith("nep141:tron") else ton_address(0xDD)
        quote.update({"depositAddress": deposit, "deadline": request["deadline"],
                      "timeWhenInactive": request["deadline"]})
    return sign({"correlationId": "tron-c0ffee", "timestamp": "2026-11-03T03:00:00.000Z", "signature": "",
                 "quoteRequest": dict(request), "quote": quote})


def live_receipt(tmp_path, path):
    rpc, _ = near_rpc(chain=FakeNearChain(balances={}))
    out = tmp_path / "r.json"
    ni.quote(str(path), client=ni.OneClickClient(jwt="k", transport=route_api(tron_quote)), live=True,
             nonce_store=ni.NonceStore(str(tmp_path / "n.json")), receipt_path=str(out), rpc=rpc, now=NOW)
    return json.loads(out.read_text())


def test_a_tron_deposit_and_refund_are_proven_against_the_signed_quote(tmp_path):
    receipt = live_receipt(tmp_path, TRON_EXAMPLE)
    deposit = tron_tx(logs=[trc20_log(TRON_DEPOSIT, 25_000_000)])
    refund = tron_tx(tx_id=REFUND_TX, block=70_000_100, ts_ms=DEADLINE_MS + 60_000,
                     logs=[trc20_log(TRON_REFUND, 24_800_000)])
    rpc, _ = tron_rpc({"https://tron-a.invalid": {TRON_TX: deposit, REFUND_TX: refund}})
    dep = ni.verify_deposit(receipt, tx_signature=TRON_TX, tron=rpc)
    assert dep["verified"] is True and dep["kind"] == "tron_token_transfer" and dep["token"] == TRON_USDT
    ref = ni.verify_settlement(receipt, refund_tx=REFUND_TX, tron=rpc)
    assert ref["verified"] is True, ref["checks"]
    assert ref["recipient"] == TRON_REFUND and ni.conclude(receipt)["outcome"] == "refunded"


def test_a_tron_refund_before_the_deposit_block_is_not_proven(tmp_path):
    receipt = live_receipt(tmp_path, TRON_EXAMPLE)
    deposit = tron_tx(logs=[trc20_log(TRON_DEPOSIT, 25_000_000)])
    early = tron_tx(tx_id=REFUND_TX, block=69_999_999, logs=[trc20_log(TRON_REFUND, 24_800_000)])
    rpc, _ = tron_rpc({"https://tron-a.invalid": {TRON_TX: deposit, REFUND_TX: early}})
    ni.verify_deposit(receipt, tx_signature=TRON_TX, tron=rpc)
    ev = ni.verify_settlement(receipt, refund_tx=REFUND_TX, tron=rpc)
    assert ev["verified"] is False and verdict(ev)["after_deposit"] is False


def test_ton_routes_but_its_deposit_evidence_is_refused_by_name(tmp_path):
    path = program(tmp_path, source="Ton.USDT", receiver=ton_address(0x11))
    receipt = live_receipt(tmp_path, Path(path))
    assert receipt["leg"]["origin"]["chain"] == "ton"
    with pytest.raises(ni.NearIntentsError) as err:
        ni.verify_deposit(receipt, tx_signature="ton-tx")
    assert err.value.code == "X3_EVIDENCE_UNSUPPORTED" and "jetton wallet" in err.value.message


def test_a_tron_refund_short_of_the_signed_amount_is_not_proven(tmp_path):
    receipt = live_receipt(tmp_path, TRON_EXAMPLE)
    deposit = tron_tx(logs=[trc20_log(TRON_DEPOSIT, 25_000_000)])
    short = tron_tx(tx_id=REFUND_TX, block=70_000_100, logs=[trc20_log(TRON_REFUND, 24_799_999)])
    rpc, _ = tron_rpc({"https://tron-a.invalid": {TRON_TX: deposit, REFUND_TX: short}})
    ni.verify_deposit(receipt, tx_signature=TRON_TX, tron=rpc)
    ev = ni.verify_settlement(receipt, refund_tx=REFUND_TX, tron=rpc)
    assert ev["verified"] is False and verdict(ev)["amount"] is False
