"""Offline regressions for the mainnet demo's evidence and outcome boundaries."""
import json

import pytest

from test_chain_evidence import (
    DEPOSIT, DEPOSIT_SIG, USDC_MINT, live_receipt, ni, solana, spl_tx,
    _trust_the_test_key,
)
from test_near_intents import NEAR_USDC, _status


def test_receipt_mint_substitution_is_rejected_before_rpc(tmp_path):
    path, _, _ = live_receipt(tmp_path)
    receipt = json.loads(path.read_text())
    receipt["tokens"]["origin"]["contractAddress"] = "So11111111111111111111111111111111111111112"
    rpc, fake = solana({DEPOSIT_SIG: spl_tx(DEPOSIT_SIG, to=DEPOSIT, amount=25_000_000)})
    with pytest.raises(ni.NearIntentsError) as exc:
        ni.verify_deposit(receipt, tx_signature=DEPOSIT_SIG, solana=rpc)
    assert exc.value.code == "X3_EVIDENCE_BINDING"
    assert fake.calls == []


@pytest.mark.parametrize("field,value", [
    ("account", "attacker.near"),
    ("token", "attacker-token.near"),
])
def test_balance_observation_is_bound_to_signed_destination(tmp_path, field, value):
    path, _, rpc = live_receipt(tmp_path)
    receipt = json.loads(path.read_text())
    receipt["evidence"]["settlement_baseline"][field] = value
    with pytest.raises(ni.NearIntentsError) as exc:
        ni.verify_settlement(receipt, near_views=ni.near_views_for(rpc))
    assert exc.value.code == "X3_EVIDENCE_BINDING"


def test_balance_increase_and_provider_success_cannot_prove_settlement(tmp_path):
    path, chain, rpc = live_receipt(tmp_path)
    receipt = json.loads(path.read_text())
    sol, _ = solana({DEPOSIT_SIG: spl_tx(DEPOSIT_SIG, to=DEPOSIT, amount=25_000_000)})
    assert ni.verify_deposit(receipt, tx_signature=DEPOSIT_SIG, solana=sol)["verified"]
    # This could be an unrelated incoming transfer; no payout transaction is supplied.
    chain.balances[(NEAR_USDC.split(":", 1)[1], "x3-demo.near")] += 25_000_000
    receipt["status_history"] = [ni.interpret_status(_status("SUCCESS", amountOut="25000000"))]
    evidence = ni.verify_settlement(receipt, near_views=ni.near_views_for(rpc))
    assert evidence["verified"] is False
    assert evidence["balance_delta_observed"] is True
    assert evidence["baseline_authenticated"] is False
    assert evidence["cross_chain_causality_proven"] is False
    assert receipt["verification"]["status"] == "unverified"
    assert "verified_outcome" not in receipt
    with pytest.raises(ni.NearIntentsError) as exc:
        ni.guard_args(receipt, "outcome")
    assert exc.value.code == "X3_EVIDENCE_UNSUPPORTED"


@pytest.mark.parametrize("kind", ["near_ft_balance_delta", "invented_transaction_proof"])
def test_old_or_invented_saved_flags_cannot_reenable_settled(kind):
    receipt = {
        "evidence": {"deposit": {"verified": True},
                     "settlement": {"verified": True, "kind": kind,
                                    "cross_chain_causality_proven": True}},
        "verified_outcome": "settled",
    }
    assert ni.conclude(receipt)["outcome"] is None
    assert "verified_outcome" not in receipt


def test_provider_success_alone_never_builds_guard_outcome(tmp_path):
    path, _, _ = live_receipt(tmp_path)
    receipt = json.loads(path.read_text())
    receipt["status_history"] = [ni.interpret_status(_status("SUCCESS", amountOut="25000000"))]
    with pytest.raises(ni.NearIntentsError) as exc:
        ni.guard_args(receipt, "outcome")
    assert exc.value.code == "X3_EVIDENCE_UNSUPPORTED"

def test_unknown_signed_solana_asset_has_no_trusted_mint(tmp_path):
    path, _, _ = live_receipt(tmp_path)
    receipt = json.loads(path.read_text())
    # The mapping function must not guess based on an editable symbol/mint,
    # even when a future provider-signed asset ID is supplied.
    receipt["quote_response"]["quoteRequest"]["originAsset"] = "nep141:unknown.omft.near"
    with pytest.raises(ni.NearIntentsError) as exc:
        ni._solana_mint(receipt)
    assert exc.value.code == "X3_EVIDENCE_UNSUPPORTED"


@pytest.mark.parametrize("field,value", [("balance", "0"), ("block_height", 1)])
def test_edited_baseline_numbers_cannot_upgrade_observation_to_settlement(tmp_path, field, value):
    path, chain, rpc = live_receipt(tmp_path)
    receipt = json.loads(path.read_text())
    receipt["evidence"]["settlement_baseline"][field] = value
    chain.balances[(NEAR_USDC.split(":", 1)[1], "x3-demo.near")] += 25_000_000
    evidence = ni.verify_settlement(receipt, near_views=ni.near_views_for(rpc))
    assert evidence["verified"] is False
    assert evidence["baseline_authenticated"] is False
    assert "verified_outcome" not in receipt
