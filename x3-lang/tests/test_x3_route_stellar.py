"""X3 Route on Stellar: checksummed `G…` addresses, the pinned 1Click listings (native XLM and
Circle's USDC through HOT omni chain 1100), and Horizon deposit evidence: closed ledger,
successful, public network, credited amount from effects, issuer pinned, memo = signed memo."""
from __future__ import annotations

import base64
import binascii
import json
from pathlib import Path
from urllib.parse import urlsplit

import pytest

from test_near_intents import (  # noqa: F401  (_trust_the_test_key is an autouse fixture)
    NOW, FakeNearChain, RecordingTransport, _trust_the_test_key, ni, near_rpc, sign,
)
from test_x3_route_chains import program, route_api, tokens  # noqa: F401

ROOT = Path(__file__).resolve().parents[1]
ce = ni.chain_evidence
reg = ni.registry
EXAMPLE = ROOT / "examples" / "x3_route_stellar_to_near.x3"
CIRCLE = "GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN"
XLM_ID = "nep245:v2_1.omni.hot.tg:1100_111bzQBB5v7AhLyPMDwS8uJgQV24KaAPXtwyVWu2KXbbfQU6NXRCz"
USDC_ID = "nep245:v2_1.omni.hot.tg:1100_111bzQBB65GxAPAVoxqmMcgYo5oS3txhqs1Uh1cgahKQUeTUq1TJu"
REFUND = "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF"
DEPOSIT = reg.stellar_address(bytes([0xCC]) * 32)
OTHER = reg.stellar_address(bytes([0x5A]) * 32)
FAKE_ISSUER = reg.stellar_address(bytes([0x99]) * 32)
TX = "a" * 64
REFUND_TX = "b" * 64
MEMO = "8812345"


def strkey(version: int, payload: bytes) -> str:
    body = bytes([version]) + payload
    return base64.b32encode(body + binascii.crc_hqx(body, 0).to_bytes(2, "little")).decode()


# --- addresses -------------------------------------------------------------------------------

def test_stellar_addresses_carry_a_version_and_checksum():
    assert reg.is_valid_stellar_address(CIRCLE)          # a real account, as 1Click lists it
    assert reg.stellar_address(bytes(32)) == REFUND
    assert reg.stellar_address(reg.stellar_account_bytes(CIRCLE)) == CIRCLE
    tampered = CIRCLE[:-1] + ("A" if CIRCLE[-1] != "A" else "B")
    for bad in (tampered, CIRCLE.lower(), CIRCLE[:-1], CIRCLE + "A",
                strkey(18 << 3, bytes(32)),               # a secret seed (S…)
                strkey(0x31, bytes(32)),                  # starts with G, but not the account version
                strkey(12 << 3, bytes(32) + bytes(8)),    # a muxed account (M…)
                "", "G" + "A" * 55):
        assert not reg.is_valid_stellar_address(bad), bad
    assert reg.is_valid_receiver("stellar", CIRCLE) and not reg.is_valid_receiver("stellar", tampered)


# --- routing --------------------------------------------------------------------------------------

@pytest.mark.parametrize("source,asset_id", [("Stellar.XLM", XLM_ID), ("Stellar.USDC", USDC_ID)])
def test_stellar_compiles_to_the_pinned_1click_asset(tmp_path, source, asset_id):
    _, leg = ni.compile_program(program(tmp_path, source=source, receiver=REFUND, amount=1_000_000_000), now=NOW)
    request = ni.build_quote_request(leg, tokens(), dry=True)
    assert request["originAsset"] == asset_id and request["refundTo"] == REFUND
    assert ni.CHAIN_TO_BLOCKCHAIN["stellar"] == "stellar" and "stellar" not in ni.ONECLICK_UNROUTED


def test_a_bad_stellar_refund_address_is_refused(tmp_path):
    with pytest.raises(ni.NearIntentsError) as err:
        ni.compile_program(program(tmp_path, source="Stellar.XLM", receiver=REFUND[:-1] + "A"), now=NOW)
    assert err.value.code == "X3_NEAR_TYPECHECK"


# --- Horizon evidence -----------------------------------------------------------------------------

def credit(account=DEPOSIT, amount="100.0000000", issuer=CIRCLE, code="USDC", muxed=False):
    effect = {"type": "account_credited", "account": account, "amount": amount}
    if issuer is None:
        effect["asset_type"] = "native"
    else:
        effect.update({"asset_type": "credit_alphanum4", "asset_code": code, "asset_issuer": issuer})
    if muxed:
        effect["account_muxed"] = "M" + "A" * 68
    return effect


def stellar_tx(*, tx_hash=TX, successful=True, ledger=55_000_000, memo=MEMO, memo_type="id",
               created="2026-11-03T03:01:00Z", effects=None):
    tx = {"id": tx_hash, "hash": tx_hash, "successful": successful, "ledger": ledger, "created_at": created,
          "memo_type": memo_type if memo is not None else "none"}
    if memo is not None:
        tx["memo"] = memo
    payer = {"type": "account_debited", "account": OTHER, "asset_type": "credit_alphanum4", "asset_code": "USDC",
             "asset_issuer": CIRCLE, "amount": "100.0000000"}
    return tx, [payer] + (effects if effects is not None else [credit()])


class FakeHorizon:
    def __init__(self, txs, passphrase=ce.STELLAR_PUBLIC_PASSPHRASE):
        self.txs, self.passphrase = txs, passphrase

    def __call__(self, method, url, payload):
        assert method == "GET" and payload is None
        path = urlsplit(url).path
        if path == "/":
            return 200, {"network_passphrase": self.passphrase, "horizon_version": "22.0.0"}
        parts = path.strip("/").split("/")
        if parts[0] != "transactions" or parts[1] not in self.txs:
            return 404, {"status": 404, "title": "Resource Missing"}
        tx, effects = self.txs[parts[1]]
        if len(parts) == 3 and parts[2] == "effects":
            return 200, {"_embedded": {"records": effects}}
        return 200, tx


def horizon(by_url, **kw):
    transports = {url: FakeHorizon(txs, **kw) for url, txs in by_url.items()}

    def handler(method, url, payload):
        base = next(u for u in transports if url.startswith(u))
        return transports[base](method, url, payload)
    return ce.StellarRpc(transport=RecordingTransport(handler), urls=list(by_url))


def usdc_deposit(rpc, *, memo=MEMO, amount=1_000_000_000, tx_hash=TX):
    return rpc.verify_token_transfer(tx_hash=tx_hash, token=CIRCLE, recipient=DEPOSIT, min_amount=amount,
                                     not_after_ms=(1793674860 + 600) * 1000, purpose="deposit", memo=memo)


def verdict(ev):
    return {c["check"]: c["ok"] for c in ev["checks"]}


def test_a_successful_usdc_deposit_with_the_signed_memo_is_verified():
    rpc = horizon({"https://horizon-a.invalid": {TX: stellar_tx()}, "https://horizon-b.invalid": {TX: stellar_tx()}})
    ev = usdc_deposit(rpc)
    assert ev["verified"] is True, ev["checks"]
    assert ev["kind"] == "stellar_asset_transfer" and ev["commitment"] == "ledger_closed"
    assert ev["observed"] == "1000000000" and ev["memo"] == MEMO and ev["blockNumber"] == 55_000_000


def test_a_text_memo_matches_too_and_native_xlm_is_read_from_native_credits():
    tx = stellar_tx(memo_type="text", effects=[credit(issuer=None, amount="250.5")])
    rpc = horizon({"https://horizon-a.invalid": {TX: tx}})
    ev = rpc.verify_native_transfer(tx_hash=TX, recipient=DEPOSIT, min_amount=2_505_000_000, not_after_ms=None,
                                    purpose="deposit", memo=MEMO)
    assert ev["verified"] is True and ev["kind"] == "stellar_native_transfer"


@pytest.mark.parametrize("tx,failed", [
    (stellar_tx(memo="999"), "memo"),                                              # someone else's memo
    (stellar_tx(memo=None), "memo"),                                               # no memo
    (stellar_tx(memo_type="hash"), "memo"),                                        # a hash memo is not the memo
    (stellar_tx(successful=False), "succeeded"),
    (stellar_tx(ledger=None), "finalized"),
    (stellar_tx(effects=[credit(issuer=FAKE_ISSUER)]), "recipient"),               # "USDC" from a fake issuer
    (stellar_tx(effects=[credit(code="USDT")]), "recipient"),
    (stellar_tx(effects=[credit(muxed=True)]), "recipient"),                       # a muxed sub-account
    (stellar_tx(effects=[credit(account=OTHER)]), "recipient"),
    (stellar_tx(effects=[credit(issuer=None)]), "recipient"),                      # XLM, not USDC
    (stellar_tx(effects=[credit(amount="99.9999999")]), "amount"),                 # a path payment that fell short
    (stellar_tx(created="2026-11-03T03:11:01Z"), "before_deadline"),
])
def test_a_stellar_deposit_that_is_not_the_signed_one_fails(tx, failed):
    ev = usdc_deposit(horizon({"https://horizon-a.invalid": {TX: tx}}))
    assert ev["verified"] is False and verdict(ev)[failed] is False


def test_credits_split_across_effects_add_up():
    tx = stellar_tx(effects=[credit(amount="60"), credit(amount="40")])
    assert usdc_deposit(horizon({"https://horizon-a.invalid": {TX: tx}}))["verified"] is True


def test_a_testnet_horizon_or_a_missing_or_disputed_transaction_fails():
    testnet = horizon({"https://horizon-a.invalid": {TX: stellar_tx()}},
                      passphrase="Test SDF Network ; September 2015")
    assert verdict(usdc_deposit(testnet))["mainnet"] is False
    assert verdict(usdc_deposit(horizon({"https://horizon-a.invalid": {}})))["found"] is False
    split = horizon({"https://horizon-a.invalid": {TX: stellar_tx()},
                     "https://horizon-b.invalid": {TX: stellar_tx(effects=[credit(amount="200")])}})
    assert verdict(usdc_deposit(split))["endpoints_agree"] is False


def test_a_full_effects_page_or_a_malformed_amount_or_hash_is_refused():
    full = stellar_tx(effects=[credit(amount="0.0000001")] * 199)
    with pytest.raises(ce.ChainEvidenceError) as err:
        usdc_deposit(horizon({"https://horizon-a.invalid": {TX: full}}))
    assert err.value.code == "X3_EVIDENCE_FORMAT"
    for amount in ("1.00000001", "-1", "1e9", ""):
        with pytest.raises(ce.ChainEvidenceError):
            usdc_deposit(horizon({"https://horizon-a.invalid": {TX: stellar_tx(effects=[credit(amount=amount)])}}))
    with pytest.raises(ce.ChainEvidenceError) as err:
        usdc_deposit(horizon({"https://horizon-a.invalid": {}}), tx_hash="../../accounts/" + "a" * 50)
    assert err.value.code == "X3_EVIDENCE_FORMAT"


def test_an_unreviewed_issuer_is_refused_before_any_credit_counts():
    with pytest.raises(ce.ChainEvidenceError) as err:
        horizon({"https://horizon-a.invalid": {TX: stellar_tx()}}).verify_token_transfer(
            tx_hash=TX, token=FAKE_ISSUER, recipient=DEPOSIT, min_amount=1, not_after_ms=None, purpose="deposit")
    assert err.value.code == "X3_EVIDENCE_UNSUPPORTED"


def test_stellar_endpoints_have_no_defaults(monkeypatch):
    monkeypatch.delenv("X3_STELLAR_HORIZON_URLS", raising=False)
    with pytest.raises(ce.ChainEvidenceError):
        ce.StellarRpc(transport=RecordingTransport(lambda *a: (200, {})))


# --- binding and memos ---------------------------------------------------------------------------------

def receipt_for(asset_id, contract=None, memo=None):
    origin = {"assetId": asset_id, "blockchain": "stellar"}
    if contract:
        origin["contractAddress"] = contract
    quote = {} if memo is None else {"depositMemo": memo}
    return {"leg": {"origin": {"chain": "stellar"}},
            "quote_response": {"quoteRequest": {"originAsset": asset_id}, "quote": quote}, "tokens": {"origin": origin}}


def test_stellar_assets_bind_to_their_pinned_issuer():
    assert ni.ledger_origin_asset(receipt_for(USDC_ID, CIRCLE)) == {"chain": "stellar", "kind": "coin", "token": CIRCLE}
    assert ni.ledger_origin_asset(receipt_for(XLM_ID)) == {"chain": "stellar", "kind": "native", "token": None}
    for receipt, code in ((receipt_for(USDC_ID, FAKE_ISSUER), "X3_EVIDENCE_BINDING"),
                          (receipt_for(XLM_ID, CIRCLE), "X3_EVIDENCE_BINDING"),
                          (receipt_for("nep245:v2_1.omni.hot.tg:1100_111unreviewed", None), "X3_EVIDENCE_UNSUPPORTED")):
        with pytest.raises(ni.NearIntentsError) as err:
            ni.ledger_origin_asset(receipt)
        assert err.value.code == code


def test_the_signed_memo_is_required_on_the_deposit_and_unusable_memos_are_refused():
    _, asset = ni._block_origin(receipt_for(USDC_ID, CIRCLE, memo=MEMO), evm=None, bsc=None, tron=None,
                                stellar=object())
    assert asset["deposit_memo"] == MEMO
    _, asset = ni._block_origin(receipt_for(USDC_ID, CIRCLE, memo=str(2 ** 64 - 1)), evm=None, bsc=None, tron=None,
                                stellar=object())
    assert asset["deposit_memo"] == str(2 ** 64 - 1)
    with pytest.raises(ni.NearIntentsError) as err:
        ni._block_origin(receipt_for(USDC_ID, CIRCLE, memo="m" * 29), evm=None, bsc=None, tron=None, stellar=object())
    assert err.value.code == "X3_EVIDENCE_UNSUPPORTED"


# --- end to end: a signed Stellar quote with a deposit memo ----------------------------------------------

def stellar_quote(request):
    quote = {"amountIn": request["amount"], "amountInFormatted": "100.0", "amountInUsd": "99.970",
             "minAmountIn": request["amount"], "amountOut": "99800000", "amountOutFormatted": "99.8",
             "amountOutUsd": "99.800", "minAmountOut": str(99_800_000 * (10_000 - request["slippageTolerance"]) // 10_000),
             "timeEstimate": 20, "refundFee": "1000000"}
    if not request["dry"]:
        quote.update({"depositAddress": DEPOSIT, "depositMemo": MEMO, "deadline": request["deadline"],
                      "timeWhenInactive": request["deadline"]})
    return sign({"correlationId": "xlm-c0ffee", "timestamp": "2026-11-03T03:00:00.000Z", "signature": "",
                 "quoteRequest": dict(request), "quote": quote})


def live_receipt(tmp_path):
    rpc, _ = near_rpc(chain=FakeNearChain(balances={}))
    out = tmp_path / "stellar.json"
    ni.quote(str(EXAMPLE), client=ni.OneClickClient(jwt="k", transport=route_api(stellar_quote)), live=True,
             nonce_store=ni.NonceStore(str(tmp_path / "n.json")), receipt_path=str(out), rpc=rpc, now=NOW)
    return json.loads(out.read_text())


def test_a_stellar_deposit_needs_the_signed_memo_and_the_refund_is_proven(tmp_path):
    receipt = live_receipt(tmp_path)
    assert receipt["quote_response"]["quote"]["depositMemo"] == MEMO
    no_memo = horizon({"https://horizon-a.invalid": {TX: stellar_tx(memo=None)}})
    assert ni.verify_deposit(receipt, tx_signature=TX, stellar=no_memo)["verified"] is False
    refund = stellar_tx(tx_hash=REFUND_TX, memo=None, ledger=55_000_100,
                        effects=[credit(account=REFUND, amount="99.9")])
    rpc = horizon({"https://horizon-a.invalid": {TX: stellar_tx(), REFUND_TX: refund}})
    assert ni.verify_deposit(receipt, tx_signature=TX, stellar=rpc)["verified"] is True
    proven = ni.verify_settlement(receipt, refund_tx=REFUND_TX, stellar=rpc)
    assert proven["verified"] is True, proven["checks"]
    assert ni.conclude(receipt)["outcome"] == "refunded"
