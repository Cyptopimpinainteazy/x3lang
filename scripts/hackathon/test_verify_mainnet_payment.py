import base64
import json
from types import SimpleNamespace

import pytest

import verify_mainnet_payment as proof

FUNDING = "funding"
DEPOSIT = "deposit"
SIGNATURE = "2" * 88
TX_HASH = "3" * 44
SIGNER = "solver.near"
RECIPIENT = "vendor.near"


def solana_tx():
    def balance(index, owner, quantity):
        return {"accountIndex": index, "mint": proof.SOL_MINT, "owner": owner,
                "uiTokenAmount": {"amount": str(quantity), "decimals": 6}}

    return {
        "slot": 123, "blockTime": 200,
        "meta": {"err": None, "preTokenBalances": [balance(0, FUNDING, 12), balance(1, DEPOSIT, 1)],
                 "postTokenBalances": [balance(0, FUNDING, 2), balance(1, DEPOSIT, 11)]},
        "transaction": {"signatures": [SIGNATURE], "message": {
            "accountKeys": [{"pubkey": "from", "signer": False}, {"pubkey": "to", "signer": False},
                            {"pubkey": FUNDING, "signer": True}],
            "instructions": [{"programId": proof.SPL_TOKEN, "parsed": {
                "type": "transfer", "info": {"source": "from", "destination": "to",
                                             "authority": FUNDING, "amount": "10"}}}],
        }},
    }


def near_responses():
    args = base64.b64encode(json.dumps({"receiver_id": RECIPIENT, "amount": "9"}).encode()).decode()
    block = {"header": {"hash": "block", "height": 10, "timestamp_nanosec": "201000000000"}}
    return {
        "network": {"chain_id": "mainnet", "genesis_hash": proof.NEAR_GENESIS},
        "result": {"final_execution_status": "FINAL", "status": {"SuccessValue": ""},
                   "transaction": {"hash": TX_HASH, "signer_id": SIGNER},
                   "transaction_outcome": {"id": TX_HASH, "outcome": {"receipt_ids": ["receipt"]}},
                   "receipts_outcome": [{"id": "receipt", "block_hash": "block",
                                         "outcome": {"receipt_ids": [], "executor_id": proof.NEAR_USDC,
                                                     "status": {"SuccessValue": ""}}}],
                   "receipts": [{"receipt_id": "receipt", "receiver_id": proof.NEAR_USDC,
                                 "predecessor_id": proof.INTENTS_CONTRACT,
                                 "receipt": {"Action": {"actions": [
                                     {"FunctionCall": {"method_name": "ft_transfer", "args": args}},
                                 ]}}}]},
        "block": block, "canonical": block, "final": {"header": {"height": 11}},
    }


def near_rpc(responses):
    def call(method, params):
        if method == "status":
            return responses["network"]
        if method == "EXPERIMENTAL_tx_status":
            assert params == {"tx_hash": TX_HASH, "sender_account_id": SIGNER, "wait_until": "FINAL"}
            return responses["result"]
        if params == {"block_id": "block"}:
            return responses["block"]
        if params == {"block_id": 10}:
            return responses["canonical"]
        assert params == {"finality": "final"}
        return responses["final"]
    return call


@pytest.mark.parametrize("case", ["ok", "wrong_amount", "late", "failed", "delegate", "balance_only"])
def test_solana_deposit(case):
    tx = solana_tx()
    instructions = tx["transaction"]["message"]["instructions"]
    if case == "wrong_amount":
        instructions[0]["parsed"]["info"]["amount"] = "11"
    elif case == "late":
        tx["blockTime"] = 201
    elif case == "failed":
        tx["meta"]["err"] = "instruction error"
    elif case == "delegate":
        instructions[0]["parsed"]["info"]["authority"] = "delegate"
    elif case == "balance_only":
        instructions.clear()
    args = (tx, SIGNATURE, FUNDING, DEPOSIT, 10, 100, 200)
    if case == "ok":
        assert proof.source_transfer(*args)["amount_units"] == 10
    else:
        with pytest.raises(proof.ProofRejected):
            proof.source_transfer(*args)


@pytest.mark.parametrize("case", ["ok", "wrong_network", "wrong_asset", "wrong_recipient",
                                  "short_payout", "failed", "unlinked", "orphaned", "optimistic", "unrelated_sender"])
def test_near_payout(case):
    responses = near_responses()
    result = responses["result"]
    recipient, minimum = RECIPIENT, 8
    if case == "wrong_network":
        responses["network"]["chain_id"] = "testnet"
    elif case == "unrelated_sender":
        result["receipts"][0]["predecessor_id"] = "other.near"
    elif case == "wrong_asset":
        result["receipts"][0]["receiver_id"] = "fake.near"
    elif case == "wrong_recipient":
        recipient = "other.near"
    elif case == "short_payout":
        minimum = 10
    elif case == "failed":
        result["receipts_outcome"][0]["outcome"]["status"] = {"Failure": {}}
    elif case == "unlinked":
        result["transaction_outcome"]["outcome"]["receipt_ids"] = []
    elif case == "orphaned":
        responses["canonical"] = {"header": {"hash": "other"}}
    elif case == "optimistic":
        result["final_execution_status"] = "EXECUTED"
    args = (near_rpc(responses), TX_HASH, SIGNER, recipient, minimum, 9, 200)
    if case == "ok":
        assert proof.destination_transfer(*args)["receipt_id"] == "receipt"
    else:
        with pytest.raises(proof.ProofRejected):
            proof.destination_transfer(*args)


@pytest.mark.parametrize("case", ["ok", "quote_changed", "bad_echo", "unsigned_success_only", "wrong_origin",
                                  "duplicate", "memo_required", "not_settled"])
def test_provider_linkage(monkeypatch, case):
    request = {"originAsset": proof.SOL_ASSET, "destinationAsset": "nep141:" + proof.NEAR_USDC,
               "swapType": "EXACT_INPUT", "recipientType": "DESTINATION_CHAIN", "recipient": RECIPIENT,
               "depositType": "ORIGIN_CHAIN", "refundType": "ORIGIN_CHAIN", "amount": "10"}
    quote = {"amountIn": "10", "depositAddress": DEPOSIT, "minAmountOut": "8",
             "deadline": "1970-01-01T00:04:00Z"}
    response = {"quoteRequest": request, "timestamp": "1970-01-01T00:01:00Z"}
    receipt = {"quote_hash": "reviewed", "quote_response": response, "deposit": {"tx_hash": SIGNATURE}}
    status = {"quoteResponse": response, "status": "SUCCESS",
              "swapDetails": {"originChainTxHashes": [{"hash": SIGNATURE}],
                              "destinationChainTxHashes": [{"hash": TX_HASH}],
                              "amountIn": "10", "amountOut": "9"}}
    approved = "reviewed"
    if case == "quote_changed":
        approved = "different"
    elif case == "wrong_origin":
        status["swapDetails"]["originChainTxHashes"][0]["hash"] = "other"
    elif case == "duplicate":
        status["swapDetails"]["destinationChainTxHashes"].append({"hash": TX_HASH})
    elif case == "memo_required":
        quote["depositMemo"] = "required"
    elif case == "not_settled":
        status["status"] = "PROCESSING"
    elif case == "unsigned_success_only":
        status["swapDetails"]["destinationChainTxHashes"] = []
    monkeypatch.setattr(proof.ni, "_signed_quote", lambda value: quote)
    monkeypatch.setattr(proof.ni, "check_oneclick_quote_signature",
                        lambda value: (case != "bad_echo", "fixture"))
    monkeypatch.setattr(proof.ni, "oneclick_quote_hash", lambda value: "reviewed")
    monkeypatch.setattr(proof.ni.registry, "is_valid_receiver", lambda *_: True)
    client = SimpleNamespace(status=lambda address: status)
    def sol_rpc(method, params):
        return proof.SOL_GENESIS if method == "getGenesisHash" else solana_tx()
    kwargs = {"client": client, "sol_rpc": sol_rpc, "near_rpc": near_rpc(near_responses())}
    if case == "ok":
        result = proof.verify(receipt, approved, FUNDING, SIGNER, **kwargs)
        assert result["status"] == "matching_finalized_transfers"
        assert result["provider_linkage"] == "unsigned_status_association"
        assert result["cross_chain_causality_proven"] is False
        assert result["guard_authorization_proven"] is False
    else:
        with pytest.raises(proof.ProofRejected):
            proof.verify(receipt, approved, FUNDING, SIGNER, **kwargs)


def test_archival_rpc_credentials_are_excluded_from_provenance(monkeypatch):
    monkeypatch.delenv("X3_NEAR_ARCHIVAL_RPC_URL", raising=False)
    assert proof.near_rpc_endpoint() == (proof.NEAR_ARCHIVAL_RPC, proof.NEAR_ARCHIVAL_RPC)
    endpoint = "https://rpc.example.invalid/private-fixture"
    monkeypatch.setenv("X3_NEAR_ARCHIVAL_RPC_URL", endpoint)
    assert proof.near_rpc_endpoint() == (endpoint, "operator_configured_https")


@pytest.mark.parametrize("endpoint", ["http://rpc.example.invalid", "https://user@rpc.example.invalid",
                                     "https://rpc.example.invalid/#fragment"])
def test_invalid_archival_rpc_configuration_fails_closed(monkeypatch, endpoint):
    monkeypatch.setenv("X3_NEAR_ARCHIVAL_RPC_URL", endpoint)
    with pytest.raises(proof.ProofRejected, match="must be an HTTPS RPC URL"):
        proof.near_rpc_endpoint()
