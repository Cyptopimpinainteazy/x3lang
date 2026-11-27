"""Offline regression against public invoice_06 RPC responses, not mock payments.

Replays transfer parsing only. The archive is RPC evidence, not authenticated
consensus, fresh network availability, signed quote verification or causality.
"""
import copy
import hashlib
import json
from pathlib import Path
import sys

import pytest

import verify_mainnet_payment as proof

ROOT = Path(__file__).resolve().parents[2]
ARCHIVE = Path(__file__).with_name("fixtures") / "invoice06-mainnet-rpc.json"
BUNDLE = ROOT / "evidence/near-guard-mainnet-invoice06-2026-11-12"


def replay():
    raw = ARCHIVE.read_bytes()
    assert hashlib.sha256(raw).hexdigest() == "6aa00f559a5a1a5d30226f985622d4d88bd7808f152a097dc07bf8c048f239d3"
    observations = json.loads(raw)["observations"]
    recorded = json.loads((BUNDLE / "outcome-recording/settlement-reverification.json").read_text())
    facts = json.loads((BUNDLE / "public-rpc-observation.json").read_text())["intent"]["value"]["quote"]["facts"]

    def rpc(method, params):
        for row in observations.values():
            if row["method"] == method and row["params"] == params:
                return copy.deepcopy(row["response"]["result"])
        raise AssertionError("Unrecorded RPC request; replay must stay offline")

    return observations, recorded, facts, rpc


def payout(recorded, facts, rpc):
    return proof.destination_transfer(
        rpc, recorded["payout"]["transaction_hash"], "intents.near", facts["recipient"],
        int(facts["min_amount_out"]), int(facts["amount_out"]), recorded["source"]["timestamp"],
    )


def test_archived_real_mainnet_transfers_match_recorded_amounts():
    observations, recorded, facts, rpc = replay()
    source = proof.source_transfer(
        observations["deposit"]["response"]["result"], recorded["source"]["signature"],
        recorded["source"]["source_owner"], facts["deposit_address"], int(facts["amount_in"]),
        facts["issued_at_ms"] / 1000, facts["quote_deadline_ms"] / 1000,
    )
    assert source["amount_units"] == 25_000_000
    assert source["slot"] == recorded["source"]["slot"]
    received = payout(recorded, facts, rpc)
    assert received["amount_units"] == 24_995_200
    assert received["receipt_id"] == recorded["payout"]["receipt_id"]
    assert received["block_hash"] == recorded["payout"]["block_hash"]


@pytest.mark.parametrize("mutation", ["null_success", "malformed_success", "optimistic",
                                      "wrong_network", "noncanonical", "duplicate_outcome",
                                      "wrong_recipient", "short_payout"])
def test_corrupted_real_payout_cannot_pass(mutation):
    observations, recorded, facts, rpc = replay()
    tx = observations["payout"]["response"]["result"]
    if mutation == "null_success":
        tx["status"] = {"SuccessValue": None}
    elif mutation == "malformed_success":
        tx["status"] = {"SuccessValue": "not base64"}
    elif mutation == "optimistic":
        tx["final_execution_status"] = "EXECUTED_OPTIMISTIC"
    elif mutation == "wrong_network":
        observations["network"]["response"]["result"]["chain_id"] = "testnet"
    elif mutation == "noncanonical":
        observations["canonical"]["response"]["result"]["header"]["hash"] = "wrong-block"
    elif mutation == "duplicate_outcome":
        tx["receipts_outcome"].append(copy.deepcopy(tx["receipts_outcome"][0]))
    elif mutation == "wrong_recipient":
        facts["recipient"] = "attacker.near"
    elif mutation == "short_payout":
        facts["min_amount_out"] = "25000000"
    with pytest.raises(proof.ProofRejected):
        payout(recorded, facts, rpc)


@pytest.mark.parametrize("failure", [TimeoutError("RPC unavailable"), OSError("RPC unavailable")])
def test_unavailable_rpc_exits_unverified_without_payment_claim(monkeypatch, capsys, failure):
    monkeypatch.setattr(sys, "argv", ["verify_mainnet_payment.py", "--receipt", str(ARCHIVE),
                                     "--approved-quote-hash", "unused", "--funding-owner", "unused",
                                     "--payout-signer", "unused"])

    def unavailable(*args, **kwargs):
        _, recorded, facts, rpc = replay()

        def interrupted(method, params):
            if method == "EXPERIMENTAL_tx_status":
                raise failure
            return rpc(method, params)

        # Interrupt the real payout verifier after network identity succeeds.
        # The CLI must not emit a partially verified payment as success.
        return payout(recorded, facts, interrupted)

    monkeypatch.setattr(proof, "verify", unavailable)
    assert proof.main() == 5
    result = json.loads(capsys.readouterr().out)
    assert result["status"] == "unverified"
    assert "source" not in result and "payout" not in result
