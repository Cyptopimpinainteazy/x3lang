"""Independent chain evidence: the deposit, the delivery and the refund, proven from the chains.

Solana transactions here have the documented `getTransaction` shape (`encoding: jsonParsed`,
`commitment: finalized`); NEAR reads are `query` / `call_function` results. Both are served by
test doubles through the adapter's injectable transports, so nothing touches the network.
"""
import json

import pytest

from test_near_intents import (  # noqa: E402,F401  (_trust_the_test_key is an autouse fixture)
    BSC_DEPOSIT, BSC_EXAMPLE, EXAMPLE, NOW, NEAR_USDC, SOL_ADDR, TOKENS, FakeNearChain, RecordingTransport, _status,
    _trust_the_test_key, api, good_bsc_quote, ni, near_rpc, sign,
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
BSC_TX = "0x" + "a" * 64
BSC_REFUND_TX = "0x" + "1" * 64
BSC_DEPOSIT = "0x" + "c" * 40
BSC_REFUND_TO = "0x000000000000000000000000000000000000dEaD"


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


class FakeBsc:
    def __init__(self, by_url, *, finalized):
        self.by_url = by_url
        self.finalized = finalized
        self.calls = []

    def __call__(self, method, url, payload):
        self.calls.append(payload)
        entry = self.by_url[url]
        call = payload["method"]
        if call == "eth_getTransactionByHash":
            tx = entry[payload["params"][0]]
            result = tx
        elif call == "eth_getTransactionReceipt":
            tx = entry[payload["params"][0]]
            result = {
                "transactionHash": tx["hash"],
                "blockHash": tx["blockHash"],
                "blockNumber": tx["blockNumber"],
                "status": tx["status"],
                "to": tx["to"],
                "from": tx["from"],
            }
        elif call == "eth_getBlockByNumber":
            tag = payload["params"][0]
            if tag == "finalized":
                result = {"number": hex(self.finalized), "hash": "0x" + "f" * 64}
            else:
                tx = next(tx for tx in entry.values() if tx["blockNumber"] == tag)
                result = {"number": tx["blockNumber"], "hash": tx["blockHash"],
                          "timestamp": hex(tx["blockTimestamp"])}
        else:
            raise AssertionError(call)
        return 200, {"jsonrpc": "2.0", "id": payload["id"], "result": result}


def bsc_tx(*, tx_hash=BSC_TX, to=BSC_DEPOSIT, value=1_000_000_000_000_000, status="0x1",
           block=45_000_000, block_timestamp=T_0301):
    return {
        "hash": tx_hash,
        "from": "0x" + "e" * 40,
        "to": to,
        "value": hex(value),
        "blockNumber": hex(block),
        "blockHash": "0x" + "b" * 64,
        "blockTimestamp": block_timestamp,
        "status": status,
    }


def bsc_rpc(tx=None, *, urls=("https://bsc-a.example.invalid",), finalized=45_000_020):
    tx = tx or bsc_tx()
    fake = FakeBsc({url: {tx["hash"]: tx} for url in urls}, finalized=finalized)
    return ce.BscRpc(transport=RecordingTransport(fake), urls=list(urls)), fake


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


def live_bsc_receipt(tmp_path):
    chain = FakeNearChain(balances={("wrap.near", "wrap.near"): 1_000_000})
    rpc, _ = near_rpc(chain=chain)
    path = tmp_path / "bsc.json"
    ni.quote(str(BSC_EXAMPLE), client=ni.OneClickClient(jwt="k", transport=api(quote_fn=good_bsc_quote)),
             tokens_path=str(TOKENS), live=True, nonce_store=ni.NonceStore(str(tmp_path / "bsc-nonces.json")),
             receipt_path=str(path), rpc=rpc, now=NOW)
    return path, chain, rpc


def checks(evidence):
    return {c["check"]: c["ok"] for c in evidence["checks"]}


def test_json_rpc_identifies_the_evidence_client():
    transport = RecordingTransport(lambda m, u, p: (200, {"jsonrpc": "2.0", "id": p["id"], "result": "0x1"}))
    assert ce.json_rpc(transport, "https://rpc.example.invalid", "eth_blockNumber", []) == "0x1"
    assert transport.calls[0]["headers"]["User-Agent"] == "x3-lang-evidence/1"


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


def test_a_finalized_native_bnb_deposit_is_verified_from_chain_data():
    rpc, fake = bsc_rpc()
    evidence = rpc.verify_native_transfer(
        tx_hash=BSC_TX,
        recipient=BSC_DEPOSIT,
        min_amount=1_000_000_000_000_000,
        not_after_ms=(T_0301 + 60) * 1000,
        purpose="deposit",
    )
    assert evidence["verified"] is True
    assert evidence["observed"] == "1000000000000000"
    assert evidence["blockNumber"] == 45_000_000
    assert evidence["finalizedBlock"] == 45_000_020
    assert [call["method"] for call in fake.calls][:2] == [
        "eth_getTransactionByHash",
        "eth_getTransactionReceipt",
    ]


@pytest.mark.parametrize("tx,failed", [
    (bsc_tx(status="0x0"), "succeeded"),
    (bsc_tx(to="0x" + "9" * 40), "recipient"),
    (bsc_tx(value=999_999_999_999_999), "amount"),
    (bsc_tx(block_timestamp=T_0301 + 3_600), "before_deadline"),
])
def test_an_unproven_native_bnb_deposit_is_refused(tx, failed):
    rpc, _ = bsc_rpc(tx)
    evidence = rpc.verify_native_transfer(
        tx_hash=BSC_TX,
        recipient=BSC_DEPOSIT,
        min_amount=1_000_000_000_000_000,
        not_after_ms=(T_0301 + 60) * 1000,
        purpose="deposit",
    )
    assert evidence["verified"] is False
    assert checks(evidence)[failed] is False


def test_an_unfinalized_native_bnb_deposit_is_refused():
    rpc, _ = bsc_rpc(finalized=44_999_999)
    evidence = rpc.verify_native_transfer(
        tx_hash=BSC_TX,
        recipient=BSC_DEPOSIT,
        min_amount=1,
        not_after_ms=None,
        purpose="deposit",
    )
    assert evidence["verified"] is False
    assert checks(evidence)["finalized"] is False


def test_bsc_rpc_endpoints_that_disagree_prove_nothing():
    a, b = "https://bsc-a.example.invalid", "https://bsc-b.example.invalid"
    fake = FakeBsc({
        a: {BSC_TX: bsc_tx()},
        b: {BSC_TX: bsc_tx(value=1_000_000_000_000_001)},
    }, finalized=45_000_020)
    rpc = ce.BscRpc(transport=RecordingTransport(fake), urls=[a, b])
    evidence = rpc.verify_native_transfer(
        tx_hash=BSC_TX,
        recipient=BSC_DEPOSIT,
        min_amount=1,
        not_after_ms=None,
        purpose="deposit",
    )
    assert evidence["verified"] is False
    assert checks(evidence)["endpoints_agree"] is False


def test_a_native_bnb_refund_is_verified_after_the_deposit():
    refund = bsc_tx(to=BSC_REFUND_TO, value=900_000_000_000_000, block=45_000_050)
    rpc, _ = bsc_rpc(refund, finalized=45_000_070)
    evidence = rpc.verify_native_transfer(
        tx_hash=BSC_TX,
        recipient=BSC_REFUND_TO,
        min_amount=900_000_000_000_000,
        not_after_ms=None,
        purpose="refund",
        after_block=45_000_000,
    )
    assert evidence["verified"] is True


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


def test_a_bsc_live_quote_records_evm_deposit_and_near_baseline(tmp_path):
    path, _, _ = live_bsc_receipt(tmp_path)
    receipt = json.loads(path.read_text())
    assert receipt["deposit"]["address"] == BSC_DEPOSIT
    assert receipt["deposit"]["amount"] == "1000000000000000"
    assert receipt["tokens"]["origin"]["symbol"] == "BNB"
    assert receipt["tokens"]["destination"]["symbol"] == "wNEAR"
    assert receipt["evidence"]["settlement_baseline"]["token"] == "wrap.near"
    assert receipt["evidence"]["settlement_baseline"]["account"] == "wrap.near"


def test_verify_deposit_dispatches_bsc_native_bnb(tmp_path):
    path, _, _ = live_bsc_receipt(tmp_path)
    receipt = json.loads(path.read_text())
    rpc, _ = bsc_rpc()
    evidence = ni.verify_deposit(receipt, tx_signature=BSC_TX, bsc=rpc)
    assert evidence["verified"] is True
    assert evidence["kind"] == "bsc_native_transfer"
    assert evidence["purpose"] == "deposit"
    assert receipt["verification"]["missing"] == ["settlement or refund"]


def test_verify_deposit_cli_dispatches_bsc_native_bnb(tmp_path, capsys):
    path, _, _ = live_bsc_receipt(tmp_path)
    rpc, _ = bsc_rpc()
    assert ni.main([
        "verify-deposit", "--receipt", str(path), "--tx", BSC_TX,
    ], bsc=rpc) == 0
    out = json.loads(capsys.readouterr().out)
    assert out["evidence"]["kind"] == "bsc_native_transfer"
    assert out["evidence"]["verified"] is True


def test_bsc_receipt_prepares_guard_and_tracking_without_authorizing(tmp_path):
    path, _, _ = live_bsc_receipt(tmp_path)
    receipt = json.loads(path.read_text())
    handoff = ni.prepare_handoff(
        receipt,
        receipt_path=str(path),
        program_path=str(BSC_EXAMPLE),
        contract="guard.near",
        signer="agent.near",
        key_file="/owner-only/agent.json",
    )
    assert handoff["status"] == "prepared_not_authorized"
    assert handoff["broadcasted"] is False
    assert handoff["wallet_signature_required"] is True
    assert [step["step"] for step in handoff["guard_steps"]] == ["register", "quote"]
    assert handoff["guard_steps"][0]["method"] == "register_intent"
    assert handoff["guard_steps"][1]["method"] == "accept_quote"
    assert handoff["guard_steps"][0]["command"].startswith("near contract call-function as-transaction guard.near")
    assert handoff["deposit"]["address"] == BSC_DEPOSIT
    assert handoff["deposit"]["asset"].startswith("nep245:")
    assert handoff["deposit"]["amount"] == "1000000000000000"
    assert handoff["deposit"]["deadline"] == receipt["deposit"]["deadline"]
    assert "submit-deposit" in handoff["tracking_commands"][0]
    assert "status" in handoff["tracking_commands"][1]
    assert "verify-deposit" in handoff["tracking_commands"][2]
    assert "verify-settlement" in handoff["tracking_commands"][3]


def test_prepare_cli_never_claims_broadcast_or_authorization(tmp_path, capsys):
    path, _, _ = live_bsc_receipt(tmp_path)
    code = ni.main([
        "prepare", "--receipt", str(path), "--program", str(BSC_EXAMPLE), "--contract", "guard.near",
        "--signer", "agent.near", "--key-file", "/owner-only/agent.json",
    ])
    assert code == 0
    out = json.loads(capsys.readouterr().out)
    assert out["status"] == "prepared_not_authorized"
    assert out["broadcasted"] is False
    assert out["wallet_signature_required"] is True
    assert all(step["command"] for step in out["guard_steps"])


def test_prepare_rejects_a_receipt_not_bound_to_the_supplied_program(tmp_path):
    path, _, _ = live_bsc_receipt(tmp_path)
    receipt = json.loads(path.read_text())
    receipt["program_sha256"] = "0" * 64
    with pytest.raises(ni.NearIntentsError) as error:
        ni.prepare_handoff(receipt, receipt_path=str(path), program_path=str(BSC_EXAMPLE))
    assert error.value.code == "X3_EVIDENCE_BINDING"


def test_bsc_refund_requires_and_reuses_finalized_deposit_evidence(tmp_path):
    path, _, _ = live_bsc_receipt(tmp_path)
    receipt = json.loads(path.read_text())
    url = "https://bsc-a.example.invalid"
    fake = FakeBsc({
        url: {
            BSC_TX: bsc_tx(),
            BSC_REFUND_TX: bsc_tx(
                tx_hash=BSC_REFUND_TX,
                to=BSC_REFUND_TO,
                value=995_000_000_000_000,
                block=45_000_050,
            ),
        },
    }, finalized=45_000_070)
    rpc = ce.BscRpc(transport=RecordingTransport(fake), urls=[url])
    assert ni.verify_deposit(receipt, tx_signature=BSC_TX, bsc=rpc)["verified"]
    refund = ni.verify_settlement(receipt, refund_tx=BSC_REFUND_TX, bsc=rpc)
    assert refund["verified"] is True
    assert refund["purpose"] == "refund"
    assert refund["observed"] == "995000000000000"
    assert receipt["verified_outcome"] == "refunded"


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
    # The contract records the amount the chain proved, not 1Click's unsigned status.
    chain.balances[(NEAR_USDC_CONTRACT, "x3-demo.near")] = 1_000_000 + 24_960_000
    assert ni.verify_settlement(receipt, near_views=views)["delivered"] == "24960000"
    assert ni.guard_args(receipt, "outcome")["amount_out"] == "24960000"


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


def test_refund_cannot_use_mutated_unsigned_recipient(tmp_path):
    path, _, _ = live_receipt(tmp_path)
    receipt = json.loads(path.read_text())
    # The signed 1Click quote still refunds to SOL_ADDR.
    receipt["leg"]["refund_to"] = SENDER
    sol, fake = solana({
        REFUND_SIG: spl_tx(REFUND_SIG, to=SENDER, sender=DEPOSIT, amount=25_000_000 - 244_425),
    })
    with pytest.raises(ni.NearIntentsError) as error:
        ni.verify_settlement(receipt, refund_tx=REFUND_SIG, solana=sol)
    assert error.value.code == "X3_EVIDENCE_BINDING"
    assert fake.calls == []


def test_refund_cannot_use_mutated_saved_quote_refund_target(tmp_path):
    path, _, _ = live_receipt(tmp_path)
    receipt = json.loads(path.read_text())
    receipt["quote_request"]["refundTo"] = SENDER
    sol, fake = solana({REFUND_SIG: spl_tx(
        REFUND_SIG, to=SOL_ADDR, sender=DEPOSIT, amount=25_000_000 - 244_425,
    )})
    with pytest.raises(ni.NearIntentsError) as error:
        ni.verify_settlement(receipt, refund_tx=REFUND_SIG, solana=sol)
    assert error.value.code == "X3_EVIDENCE_BINDING"
    assert fake.calls == []


def _reseal_live_quote(receipt, *, fee=None, deposit_address=None):
    response = json.loads(json.dumps(receipt["quote_response"]))
    if fee is not None:
        response["quote"]["refundFee"] = fee
    if deposit_address is not None:
        response["quote"]["depositAddress"] = deposit_address
        receipt["deposit"]["address"] = deposit_address
    sign(response)  # test-only deterministic Ed25519 signer, not a live key
    receipt["quote_response"] = response
    receipt["quote_hash"] = ni.oneclick_quote_hash(response)
    receipt["signed_payload"] = ni.signed_quote_payload(response)
    receipt["quote_response_sha256"] = ni._sha256_json(response)


def test_refund_reverifies_original_deposit_against_current_signed_quote(tmp_path):
    path, _, _ = live_receipt(tmp_path)
    receipt = json.loads(path.read_text())
    receipt["deposit"]["tx_hash"] = DEPOSIT_SIG
    sol, fake = solana({
        DEPOSIT_SIG: spl_tx(DEPOSIT_SIG, to=DEPOSIT, amount=25_000_000),
        REFUND_SIG: spl_tx(REFUND_SIG, to=SOL_ADDR, sender=DEPOSIT,
                           amount=25_000_000 - 244_425, slot=350_000_900),
    })
    assert ni.verify_deposit(receipt, solana=sol)["verified"]
    # Swap to another *validly signed* quote after the original deposit was proven.
    # Its destination is a different address, so old evidence must not be reused.
    _reseal_live_quote(receipt, deposit_address=SENDER)
    with pytest.raises(ni.NearIntentsError) as err:
        ni.verify_settlement(receipt, refund_tx=REFUND_SIG, solana=sol)
    assert err.value.code == "X3_EVIDENCE_BINDING"
    assert len(fake.calls) >= 2  # source deposit was fetched again before refund acceptance


@pytest.mark.parametrize("fee", [0, "-1", "25000000", "9999999999"])
def test_refund_rejects_unsigned_zero_or_invalid_signed_fee(tmp_path, fee):
    path, _, _ = live_receipt(tmp_path)
    receipt = json.loads(path.read_text())
    receipt["deposit"]["tx_hash"] = DEPOSIT_SIG
    _reseal_live_quote(receipt, fee=fee)
    sol, fake = solana({})
    with pytest.raises(ni.NearIntentsError) as err:
        ni.verify_settlement(receipt, refund_tx=REFUND_SIG, solana=sol)
    assert err.value.code == "X3_EVIDENCE_BINDING"
    assert fake.calls == []


def test_refund_rejects_deposit_recheck_without_real_slot(tmp_path):
    path, _, _ = live_receipt(tmp_path)
    receipt = json.loads(path.read_text())
    receipt["deposit"]["tx_hash"] = DEPOSIT_SIG
    rpc, _ = solana({DEPOSIT_SIG: spl_tx(DEPOSIT_SIG, to=DEPOSIT, amount=25_000_000)})
    assert ni.verify_deposit(receipt, solana=rpc)["verified"]
    # The old caller blindly reused an unsigned source-evidence slot.
    # Mimic an incomplete/corrupt RPC result that claims success without a slot.
    receipt["evidence"]["deposit"]["slot"] = None

    class MissingSlotSolana:
        def __init__(self):
            self.calls = []

        def verify_transfer(self, **kwargs):
            self.calls.append(kwargs)
            if kwargs["purpose"] == "deposit":
                return {**receipt["evidence"]["deposit"], "verified": True, "slot": None}
            return {"verified": True, "observed": str(25_000_000 - 244_425)}

    malformed_rpc = MissingSlotSolana()
    with pytest.raises(ni.NearIntentsError) as error:
        ni.verify_settlement(receipt, refund_tx=REFUND_SIG, solana=malformed_rpc)
    assert error.value.code == "X3_EVIDENCE_BINDING"
    assert len(malformed_rpc.calls) == 1  # no refund RPC when deposit slot is absent


def test_guard_refund_amount_must_come_from_verified_chain_evidence(tmp_path):
    path, _, _ = live_receipt(tmp_path)
    receipt = json.loads(path.read_text())
    receipt["deposit"]["tx_hash"] = DEPOSIT_SIG
    receipt["status_history"] = [{"status": "REFUNDED", "outcome": "refunded", "terminal": True,
                                  "refunded_amount": "999999999999"}]
    with pytest.raises(ni.NearIntentsError):
        ni.guard_args(receipt, "outcome")
    sol, _ = solana({
        DEPOSIT_SIG: spl_tx(DEPOSIT_SIG, to=DEPOSIT, amount=25_000_000),
        REFUND_SIG: spl_tx(REFUND_SIG, to=SOL_ADDR, sender=DEPOSIT,
                           amount=25_000_000 - 244_425, slot=350_000_900),
    })
    assert ni.verify_deposit(receipt, solana=sol)["verified"]
    assert ni.verify_settlement(receipt, refund_tx=REFUND_SIG, solana=sol)["verified"]
    args = ni.guard_args(receipt, "outcome")
    assert args["refunded_amount"] == str(25_000_000 - 244_425)
    assert args["refunded_amount"] != "999999999999"


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
