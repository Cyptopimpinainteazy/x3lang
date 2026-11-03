"""Independent chain evidence: the deposit, the delivery and the refund, proven from the chains.

Solana transactions here have the documented `getTransaction` shape (`encoding: jsonParsed`,
`commitment: finalized`); NEAR reads are `query` / `call_function` results. Both are served by
test doubles through the adapter's injectable transports, so nothing touches the network.
"""
import json

import pytest

from test_near_intents import (  # noqa: E402,F401  (_trust_the_test_key is an autouse fixture)
    EXAMPLE, NOW, NEAR_USDC, SOL_ADDR, FakeNearChain, RecordingTransport, _status, _trust_the_test_key, api, ni,
    near_rpc,
)

ce = ni.chain_evidence
USDC_MINT = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v"
DEPOSIT = "9xQeWvG816bUx9EPjHmaT23yvVM2ZWbrrpZb9PusVFin"
SENDER = "7Np41oeYqPefeNQEHSv1UDhYrehxin3NStELsSKCT4K2"
TOKEN_PROGRAM = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA"
DEPOSIT_SIG = "5Kd3NBUAdUnhyzenEwVLy9pBKxSwXvE9FMPyR4UKZvpe6E3AgLr6PbXCeLRAUQh8M2VeS1kz5Wnw6hcqEfWyqGcW"
REFUND_SIG = "3pGoFxv2D5QBgG7Ckr4dkCcqufWaGyZT6fDZ6N4i3BRpYLcBy96HmYMkVuqoEhKDY4qbtCbP8WxF6wPCvhC1ZhJz"
NEAR_USDC_CONTRACT = NEAR_USDC.split(":", 1)[1]
T_0301 = 1793674860  # 2026-11-03T03:01:00Z, inside the 10-minute quote window


def balance(index, owner, amount, mint=USDC_MINT):
    return {"accountIndex": index, "mint": mint, "owner": owner, "programId": TOKEN_PROGRAM,
            "uiTokenAmount": {"amount": str(amount), "decimals": 6, "uiAmount": amount / 1e6, "uiAmountString": str(amount / 1e6)}}


def spl_tx(signature, *, to, amount, slot=350_000_000, block_time=T_0301, err=None, sender=SENDER, mint=USDC_MINT,
           pre_to=0, create_account=False):
    """A finalized transfer of `amount` of `mint` from `sender` to `to`, as getTransaction returns it."""
    pre = [balance(2, sender, 100_000_000, mint)]
    if not create_account:
        pre.insert(0, balance(1, to, pre_to, mint))
    post = [balance(1, to, pre_to + (0 if err else amount), mint), balance(2, sender, 100_000_000 - (0 if err else amount), mint)]
    return {
        "slot": slot, "blockTime": block_time, "version": 0,
        "meta": {"err": err, "fee": 5000, "preTokenBalances": pre, "postTokenBalances": post,
                 "preBalances": [], "postBalances": [], "innerInstructions": [], "logMessages": []},
        "transaction": {"signatures": [signature], "message": {"accountKeys": [], "instructions": []}},
    }


class FakeSolana:
    """Answers getTransaction per endpoint: {url: {signature: tx-or-None}}."""

    def __init__(self, by_url):
        self.by_url = by_url
        self.calls = []

    def __call__(self, method, url, payload):
        self.calls.append(payload)
        assert payload["method"] == "getTransaction"
        sig, opts = payload["params"]
        assert opts == {"encoding": "jsonParsed", "commitment": "finalized", "maxSupportedTransactionVersion": 0}
        return 200, {"jsonrpc": "2.0", "id": payload["id"], "result": self.by_url[url].get(sig)}


def solana(txs, urls=("https://sol-a.example.invalid",)):
    fake = FakeSolana({u: dict(txs) for u in urls})
    return ce.SolanaRpc(transport=RecordingTransport(fake), urls=list(urls)), fake


def live_receipt(tmp_path, chain=None):
    chain = chain or FakeNearChain(balances={(NEAR_USDC_CONTRACT, "x3-demo.near"): 1_000_000})
    rpc, _ = near_rpc(chain=chain)
    path = tmp_path / "r.json"
    ni.quote(str(EXAMPLE), client=ni.OneClickClient(jwt="k", transport=api()), live=True,
             nonce_store=ni.NonceStore(str(tmp_path / "n.json")), receipt_path=str(path), rpc=rpc, now=NOW)
    return path, chain, rpc


def checks(evidence):
    return {c["check"]: c["ok"] for c in evidence["checks"]}


# --- Solana transfer verdicts --------------------------------------------------------------

def test_a_finalized_deposit_of_the_full_amount_is_verified():
    rpc, fake = solana({DEPOSIT_SIG: spl_tx(DEPOSIT_SIG, to=DEPOSIT, amount=25_000_000)})
    ev = rpc.verify_transfer(signature=DEPOSIT_SIG, mint=USDC_MINT, owner=DEPOSIT, min_amount=25_000_000,
                             not_after_ms=(T_0301 + 60) * 1000, purpose="deposit")
    assert ev["verified"] and ev["observed"] == "25000000" and ev["slot"] == 350_000_000
    assert len(ev["rpcs"][0]["result_sha256"]) == 64
    assert fake.calls[0]["params"][0] == DEPOSIT_SIG


def test_a_deposit_that_creates_the_token_account_still_counts():
    rpc, _ = solana({DEPOSIT_SIG: spl_tx(DEPOSIT_SIG, to=DEPOSIT, amount=25_000_000, create_account=True)})
    assert rpc.verify_transfer(signature=DEPOSIT_SIG, mint=USDC_MINT, owner=DEPOSIT, min_amount=25_000_000,
                               not_after_ms=None, purpose="deposit")["verified"]


@pytest.mark.parametrize("tx,failed", [
    (None, "finalized"),
    (spl_tx(DEPOSIT_SIG, to=DEPOSIT, amount=24_999_999), "amount"),
    (spl_tx(DEPOSIT_SIG, to=DEPOSIT, amount=25_000_000, err={"InstructionError": [0, "Custom"]}), "succeeded"),
    (spl_tx(DEPOSIT_SIG, to=DEPOSIT, amount=25_000_000, block_time=T_0301 + 3600), "before_deadline"),
    (spl_tx(DEPOSIT_SIG, to=SENDER, amount=25_000_000, sender=DEPOSIT), "amount"),
    (spl_tx(DEPOSIT_SIG, to=DEPOSIT, amount=25_000_000, mint="Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB"), "amount"),
    (spl_tx("someOtherSignature", to=DEPOSIT, amount=25_000_000), "signature_matches"),
])
def test_a_deposit_that_is_not_proven_is_refused(tx, failed):
    rpc, _ = solana({DEPOSIT_SIG: tx})
    ev = rpc.verify_transfer(signature=DEPOSIT_SIG, mint=USDC_MINT, owner=DEPOSIT, min_amount=25_000_000,
                             not_after_ms=(T_0301 + 60) * 1000, purpose="deposit")
    assert ev["verified"] is False
    assert checks(ev)[failed] is False


def test_endpoints_that_disagree_prove_nothing():
    a, b = "https://sol-a.example.invalid", "https://sol-b.example.invalid"
    fake = FakeSolana({a: {DEPOSIT_SIG: spl_tx(DEPOSIT_SIG, to=DEPOSIT, amount=25_000_000)},
                       b: {DEPOSIT_SIG: spl_tx(DEPOSIT_SIG, to=DEPOSIT, amount=25_000_000, slot=350_000_001)}})
    rpc = ce.SolanaRpc(transport=RecordingTransport(fake), urls=[a, b])
    ev = rpc.verify_transfer(signature=DEPOSIT_SIG, mint=USDC_MINT, owner=DEPOSIT, min_amount=1, not_after_ms=None, purpose="deposit")
    assert ev["verified"] is False and checks(ev)["endpoints_agree"] is False
    assert [r["url"] for r in ev["rpcs"]] == [a, b]


def test_an_rpc_error_is_raised_not_guessed():
    transport = RecordingTransport(lambda m, u, p: (200, {"jsonrpc": "2.0", "id": p["id"], "error": {"code": -32005, "message": "busy"}}))
    rpc = ce.SolanaRpc(transport=transport, urls=["https://sol.example.invalid"])
    with pytest.raises(ce.ChainEvidenceError) as err:
        rpc.verify_transfer(signature=DEPOSIT_SIG, mint=USDC_MINT, owner=DEPOSIT, min_amount=1, not_after_ms=None, purpose="deposit")
    assert err.value.code == "X3_EVIDENCE_RPC"


def test_malformed_token_balances_are_refused():
    tx = spl_tx(DEPOSIT_SIG, to=DEPOSIT, amount=25_000_000)
    tx["meta"]["postTokenBalances"][0]["uiTokenAmount"]["amount"] = "25.0"
    with pytest.raises(ce.ChainEvidenceError) as err:
        ce.spl_balance_delta(tx, mint=USDC_MINT, owner=DEPOSIT)
    assert err.value.code == "X3_EVIDENCE_FORMAT"


# --- NEAR balance reads -------------------------------------------------------------------

def test_ft_balance_reads_every_endpoint_at_the_same_block():
    chain = FakeNearChain(balances={("usdc.near", "alice.near"): 42})
    transport = RecordingTransport(chain)
    views = ce.NearViews(transport=transport, urls=["https://a.example.invalid", "https://b.example.invalid"])
    read = views.ft_balance("usdc.near", "alice.near")
    assert read["balance"] == "42"
    first, second = (c["body"]["params"] for c in transport.calls)
    assert first["finality"] == "final" and second["block_id"] == read["block_height"]
    assert first["method_name"] == "ft_balance_of"


def test_ft_balance_endpoints_that_disagree_raise():
    calls = {"n": 0}

    def handler(method, url, payload):
        calls["n"] += 1
        value = "1" if calls["n"] == 1 else "2"
        return 200, {"jsonrpc": "2.0", "id": "x", "result": {"result": list(json.dumps(value).encode()), "logs": [], "block_height": 7}}

    views = ce.NearViews(transport=RecordingTransport(handler), urls=["https://a.example.invalid", "https://b.example.invalid"])
    with pytest.raises(ce.ChainEvidenceError) as err:
        views.ft_balance("usdc.near", "alice.near")
    assert err.value.code == "X3_EVIDENCE_RPC_DISAGREE"


def test_delivery_is_the_rise_from_the_baseline():
    base = {"token": "usdc.near", "account": "a.near", "balance": "1000", "block_height": 10, "rpcs": []}
    ok = ce.judge_near_delivery(base, {**base, "balance": "26000", "block_height": 20}, min_amount=25_000)
    assert ok["verified"] and ok["delivered"] == "25000"
    short = ce.judge_near_delivery(base, {**base, "balance": "25999", "block_height": 20}, min_amount=25_000)
    assert short["verified"] is False and checks(short)["amount"] is False
    stale = ce.judge_near_delivery(base, {**base, "balance": "99999", "block_height": 10}, min_amount=1)
    assert checks(stale)["observed_after_baseline"] is False


# --- the adapter, end to end --------------------------------------------------------------

def test_a_live_quote_records_the_recipients_starting_balance(tmp_path):
    path, _, _ = live_receipt(tmp_path)
    receipt = json.loads(path.read_text())
    base = receipt["evidence"]["settlement_baseline"]
    assert (base["token"], base["account"], base["balance"]) == (NEAR_USDC_CONTRACT, "x3-demo.near", "1000000")
    assert receipt["tokens"]["origin"]["contractAddress"] == USDC_MINT


def test_a_live_quote_whose_settlement_could_never_be_measured_is_refused(tmp_path):
    chain = FakeNearChain()
    rpc, _ = near_rpc(chain=chain)
    original = chain.__call__

    def no_views(method, url, payload):
        if payload["params"]["request_type"] == "call_function":
            return 200, {"jsonrpc": "2.0", "id": payload["id"], "error": {"name": "HANDLER_ERROR", "cause": {"name": "UNKNOWN_BLOCK"}}}
        return original(method, url, payload)

    rpc.transport = RecordingTransport(no_views)
    with pytest.raises(ni.NearIntentsError) as err:
        ni.quote(str(EXAMPLE), client=ni.OneClickClient(jwt="k", transport=api()), live=True,
                 nonce_store=ni.NonceStore(str(tmp_path / "n.json")), receipt_path=str(tmp_path / "r.json"), rpc=rpc, now=NOW)
    assert err.value.code == "X3_EVIDENCE_BASELINE"
    assert not (tmp_path / "n.json").exists() and not (tmp_path / "r.json").exists()


def test_settled_end_to_end_through_the_cli(tmp_path, capsys):
    path, chain, rpc = live_receipt(tmp_path)
    client = ni.OneClickClient(jwt="k", transport=api(statuses=[_status("SUCCESS", amountOut="24950000")]))

    assert ni.main(["submit-deposit", "--deposit-address", DEPOSIT, "--tx-hash", DEPOSIT_SIG, "--receipt", str(path)], client=client) == 0
    assert json.loads(path.read_text())["deposit"]["tx_hash"] == DEPOSIT_SIG

    sol, _ = solana({DEPOSIT_SIG: spl_tx(DEPOSIT_SIG, to=DEPOSIT, amount=25_000_000)})
    assert ni.main(["verify-deposit", "--receipt", str(path)], solana=sol) == 0

    # Before the payout lands, settlement is not proven, and the receipt says so.
    views = ni.near_views_for(rpc)
    assert ni.main(["verify-settlement", "--receipt", str(path)], near_views=views) == ni.EXIT_UNVERIFIED
    assert json.loads(path.read_text())["verification"]["status"] == "unverified"

    chain.balances[(NEAR_USDC_CONTRACT, "x3-demo.near")] = 1_000_000 + 24_950_000
    ni.track(client, DEPOSIT, receipt_path=str(path))
    assert ni.main(["verify-settlement", "--receipt", str(path)], near_views=views) == 0
    capsys.readouterr()

    receipt = json.loads(path.read_text())
    assert receipt["verified_outcome"] == "settled"
    assert receipt["verification"] == {"status": "verified", "outcome": "settled", "missing": [],
                                       "1click_reported": "settled", "1click_agrees": True}
    assert receipt["evidence"]["settlement"]["delivered"] == "24950000"


def test_refunded_end_to_end(tmp_path):
    path, _, _ = live_receipt(tmp_path)
    receipt = json.loads(path.read_text())
    sol, _ = solana({
        DEPOSIT_SIG: spl_tx(DEPOSIT_SIG, to=DEPOSIT, amount=25_000_000),
        REFUND_SIG: spl_tx(REFUND_SIG, to=SOL_ADDR, sender=DEPOSIT, amount=25_000_000 - 244_425, slot=350_000_900),
    })
    assert ni.verify_deposit(receipt, tx_signature=DEPOSIT_SIG, solana=sol)["verified"]
    ev = ni.verify_settlement(receipt, refund_tx=REFUND_SIG, solana=sol)
    assert ev["verified"] and ev["required"] == str(25_000_000 - 244_425)
    assert receipt["verified_outcome"] == "refunded"


def test_a_refund_from_before_the_deposit_proves_nothing(tmp_path):
    path, _, _ = live_receipt(tmp_path)
    receipt = json.loads(path.read_text())
    sol, _ = solana({
        DEPOSIT_SIG: spl_tx(DEPOSIT_SIG, to=DEPOSIT, amount=25_000_000, slot=350_000_000),
        REFUND_SIG: spl_tx(REFUND_SIG, to=SOL_ADDR, sender=DEPOSIT, amount=25_000_000, slot=349_999_999),
    })
    ni.verify_deposit(receipt, tx_signature=DEPOSIT_SIG, solana=sol)
    ev = ni.verify_settlement(receipt, refund_tx=REFUND_SIG, solana=sol)
    assert checks(ev)["after_deposit"] is False
    assert "verified_outcome" not in receipt


def test_settlement_without_a_proven_deposit_is_not_an_outcome(tmp_path):
    path, chain, rpc = live_receipt(tmp_path)
    receipt = json.loads(path.read_text())
    chain.balances[(NEAR_USDC_CONTRACT, "x3-demo.near")] = 1_000_000 + 24_950_000
    assert ni.verify_settlement(receipt, near_views=ni.near_views_for(rpc))["verified"]
    assert receipt["verification"]["status"] == "unverified"
    assert receipt["verification"]["missing"] == ["deposit"]


def test_an_rpc_failure_is_recorded_as_unverified_evidence(tmp_path):
    path, _, _ = live_receipt(tmp_path)
    receipt = json.loads(path.read_text())
    broken = ce.SolanaRpc(transport=RecordingTransport(lambda m, u, p: (503, b"")), urls=["https://sol.example.invalid"])
    broken.transport = lambda method, url, headers, body, timeout: (503, b"Service Unavailable")
    ev = ni.verify_deposit(receipt, tx_signature=DEPOSIT_SIG, solana=broken)
    assert ev["verified"] is False and ev["error"]["code"] == "X3_EVIDENCE_RPC"
    assert receipt["evidence"]["deposit"]["verified"] is False


def test_evidence_needs_a_live_receipt_and_a_known_tx(tmp_path):
    path, _, _ = live_receipt(tmp_path)
    receipt = json.loads(path.read_text())
    with pytest.raises(ni.NearIntentsError) as err:
        ni.verify_deposit(receipt, solana=solana({})[0])
    assert err.value.code == "X3_EVIDENCE_TX"
    receipt["quote_request"]["dry"] = True
    with pytest.raises(ni.NearIntentsError) as err:
        ni.verify_deposit(receipt, tx_signature=DEPOSIT_SIG, solana=solana({})[0])
    assert err.value.code == "X3_EVIDENCE_UNSUPPORTED"


def test_submit_deposit_refuses_another_receipts_address(tmp_path):
    path, _, _ = live_receipt(tmp_path)
    client = ni.OneClickClient(jwt="k", transport=api())
    assert ni.main(["submit-deposit", "--deposit-address", "SomeoneElse111111111111111111111111111111111",
                    "--tx-hash", DEPOSIT_SIG, "--receipt", str(path)], client=client) == 1
    assert "tx_hash" not in json.loads(path.read_text())["deposit"]
