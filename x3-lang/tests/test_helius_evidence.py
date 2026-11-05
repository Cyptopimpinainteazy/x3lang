"""X3 Helius Parsed Events evidence is supplementary to finalized chain RPC proofs."""
import json
import pytest

from test_chain_evidence import (
    DEPOSIT, DEPOSIT_SIG, T_0301, USDC_MINT, live_receipt, solana, spl_tx, ni, _trust_the_test_key,
)


class HeliusReply:
    def __init__(self, body, status=200):
        self.body = body
        self.status = status
        self.calls = []

    def __call__(self, method, url, headers, body, timeout):
        self.calls.append((method, url, json.loads(body), headers))
        return self.status, json.dumps(self.body).encode()


def parsed_reply(*, signature=DEPOSIT_SIG, slot=350_000_000, block_time=T_0301,
                 destination=DEPOSIT, mint=USDC_MINT, amount=25_000_000,
                 status="OK", parser_status="OK"):
    return [{"signature": signature, "parserStatus": parser_status, "parsed": {
        "slot": slot, "blockTime": block_time, "transactionStatus": status,
        "error": None if status == "OK" else {"InstructionError": [0, "Custom"]},
        "tokenTransfers": [{"fromUserAccount": "SolanaSender", "toUserAccount": destination,
                            "mint": mint, "rawTokenAmount": amount, "decimals": 6}]
    }}]


def chain_proof():
    rpc, _ = solana({DEPOSIT_SIG: spl_tx(DEPOSIT_SIG, to=DEPOSIT, amount=25_000_000)})
    return rpc.verify_transfer(
        signature=DEPOSIT_SIG, mint=USDC_MINT, owner=DEPOSIT, min_amount=25_000_000,
        not_after_ms=(T_0301 + 60) * 1000, purpose="deposit",
    )


def test_real_canonical_rpc_and_parsed_helius_must_match_before_verification(tmp_path):
    ParsedEventsWitness = ni.helius_evidence.ParsedEventsWitness
    path, _, _ = live_receipt(tmp_path)
    receipt = json.loads(path.read_text())
    rpc, _ = solana({DEPOSIT_SIG: spl_tx(DEPOSIT_SIG, to=DEPOSIT, amount=25_000_000)})
    transport = HeliusReply(parsed_reply())
    witness = ParsedEventsWitness(api_key="secret-for-tests", transport=transport)
    result = ni.verify_deposit(receipt, tx_signature=DEPOSIT_SIG, solana=rpc, helius=witness)
    assert result["verified"] is True
    assert result["helius"]["verified"] is True
    assert result["helius"]["raw_token_net_amount"] == "25000000"
    assert result["helius"]["slot"] == 350_000_000
    assert "secret-for-tests" not in json.dumps(result)
    method, url, req, headers = transport.calls[0]
    assert method == "POST" and "/v1/parsed-events/transactions" in url
    assert req == {"transactions": [DEPOSIT_SIG], "rawTransaction": "none"}


@pytest.mark.parametrize("bad", [
    {"destination": "OtherRecipient"},
    {"mint": "DifferentMint"},
    {"amount": 24_999_999},
    {"slot": 350_000_001},
    {"status": "FAILED"},
    {"parser_status": "NOT_FOUND"},
    {"signature": "AnotherSignature"},
])
def test_helius_disagreement_cannot_create_verified_evidence(bad, tmp_path):
    ParsedEventsWitness = ni.helius_evidence.ParsedEventsWitness
    path, _, _ = live_receipt(tmp_path)
    receipt = json.loads(path.read_text())
    rpc, _ = solana({DEPOSIT_SIG: spl_tx(DEPOSIT_SIG, to=DEPOSIT, amount=25_000_000)})
    witness = ParsedEventsWitness(api_key="secret-for-tests", transport=HeliusReply(parsed_reply(**bad)))
    result = ni.verify_deposit(receipt, tx_signature=DEPOSIT_SIG, solana=rpc, helius=witness)
    assert result["verified"] is False
    assert receipt["evidence"]["deposit"]["verified"] is False


def test_helius_can_never_override_invalid_finalized_rpc_proof(tmp_path):
    ParsedEventsWitness = ni.helius_evidence.ParsedEventsWitness
    path, _, _ = live_receipt(tmp_path)
    receipt = json.loads(path.read_text())
    rpc, _ = solana({DEPOSIT_SIG: spl_tx(DEPOSIT_SIG, to=DEPOSIT, amount=20_000_000)})
    response = HeliusReply(parsed_reply())
    witness = ParsedEventsWitness(api_key="secret-for-tests", transport=response)
    result = ni.verify_deposit(receipt, tx_signature=DEPOSIT_SIG, solana=rpc, helius=witness)
    assert result["verified"] is False
    assert not response.calls, "Helius cannot promote a failed chain proof"


def test_helius_missing_key_and_http_failure_are_safe(tmp_path):
    ParsedEventsWitness = ni.helius_evidence.ParsedEventsWitness
    with pytest.raises(ni.helius_evidence.HeliusEvidenceError):
        ParsedEventsWitness(api_key=None, transport=HeliusReply([]))
    reply = HeliusReply({"error": "bad credentials"}, status=403)
    witness = ParsedEventsWitness(api_key="unprintable-key", transport=reply)
    with pytest.raises(ni.helius_evidence.HeliusEvidenceError) as exc:
        witness.attest(signature=DEPOSIT_SIG, mint=USDC_MINT, owner=DEPOSIT,
                       min_amount=25_000_000, canonical=chain_proof())
    assert "unprintable-key" not in str(exc.value)


def test_cli_explicit_helius_requires_key_and_never_claims_rpc_only_success(tmp_path, monkeypatch, capsys):
    monkeypatch.delenv("HELIUS_API_KEY", raising=False)
    path, _, _ = live_receipt(tmp_path)
    receipt = json.loads(path.read_text())
    receipt["deposit"]["tx_hash"] = DEPOSIT_SIG
    path.write_text(json.dumps(receipt))
    rpc, _ = solana({DEPOSIT_SIG: spl_tx(DEPOSIT_SIG, to=DEPOSIT, amount=25_000_000)})
    code = ni.main(["verify-deposit", "--receipt", str(path), "--helius"], solana=rpc)
    actual = json.loads(path.read_text())
    assert code != 0
    assert actual["evidence"]["deposit"]["verified"] is False
    assert actual["evidence"]["deposit"]["error"]["code"] == "X3_HELIUS_CONFIG"
    assert "HELIUS_API_KEY" in capsys.readouterr().out


def test_helius_gross_inflow_cannot_hide_equal_outflow(tmp_path):
    ParsedEventsWitness = ni.helius_evidence.ParsedEventsWitness
    reply = parsed_reply()
    transfer = reply[0]["parsed"]["tokenTransfers"][0]
    reply[0]["parsed"]["tokenTransfers"].append({
        **transfer, "fromUserAccount": DEPOSIT, "toUserAccount": "OtherWallet",
        "rawTokenAmount": 25_000_000,
    })
    path, _, _ = live_receipt(tmp_path)
    receipt = json.loads(path.read_text())
    rpc, _ = solana({DEPOSIT_SIG: spl_tx(DEPOSIT_SIG, to=DEPOSIT, amount=25_000_000)})
    witness = ParsedEventsWitness(api_key="secret-for-tests", transport=HeliusReply(reply))
    result = ni.verify_deposit(receipt, tx_signature=DEPOSIT_SIG, solana=rpc, helius=witness)
    assert result["verified"] is False


def test_helius_rejects_float_amounts_not_raw_integer_base_units(tmp_path):
    ParsedEventsWitness = ni.helius_evidence.ParsedEventsWitness
    reply = parsed_reply()
    reply[0]["parsed"]["tokenTransfers"][0]["rawTokenAmount"] = 25.0
    path, _, _ = live_receipt(tmp_path)
    receipt = json.loads(path.read_text())
    rpc, _ = solana({DEPOSIT_SIG: spl_tx(DEPOSIT_SIG, to=DEPOSIT, amount=25_000_000)})
    witness = ParsedEventsWitness(api_key="secret-for-tests", transport=HeliusReply(reply))
    result = ni.verify_deposit(receipt, tx_signature=DEPOSIT_SIG, solana=rpc, helius=witness)
    assert result["verified"] is False
