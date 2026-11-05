"""Fail-closed tests for X3 vault source proof; all Solana RPC calls are local doubles.

These tests exercise the verifier's parsing, not a deployed Solana or NEAR network.
"""
import base64
import copy
import importlib.util
import json
from pathlib import Path

import pytest

from test_chain_evidence import (  # noqa: F401  (autouse signing-key fixture)
    DEPOSIT_SIG, USDC_MINT, _trust_the_test_key, live_receipt, ni,
)

PATH = Path(__file__).resolve().parents[1] / "solana" / "vault_evidence.py"
SPEC = importlib.util.spec_from_file_location("x3_vault_evidence", PATH)
ve = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(ve)

PROGRAM = ni.ed25519.b58encode(bytes([11] * 32))
POLICY = ni.ed25519.b58encode(bytes([12] * 32))
SOURCE = ni.ed25519.b58encode(bytes([13] * 32))
DEST = ni.ed25519.b58encode(bytes([14] * 32))
NONCE = 7
TOKEN = ve.TOKEN_PROGRAM


@pytest.fixture(autouse=True)
def trust_existing_test_signer(monkeypatch):
    # The production verifier keeps its pinned 1Click key; only this test module
    # substitutes the existing deterministic quote fixture's Ed25519 public key.
    monkeypatch.setattr(ve.ni, "ONE_CLICK_MANAGER_PUBLIC_KEY", ni.ONE_CLICK_MANAGER_PUBLIC_KEY)


class VaultRpc:
    def __init__(self, *txs):
        self.urls = [f"https://sol-{i}.example.invalid" for i in range(len(txs))]
        self._txs = dict(zip(self.urls, txs))
        self.calls = []

    def get_transaction(self, url, signature):
        self.calls.append((url, signature))
        return copy.deepcopy(self._txs[url])


def b64(raw):
    return base64.b64encode(raw).decode("ascii")


def signed_event(receipt, *, program=PROGRAM, destination=DEST, owner=None, amount=None,
                 nonce=NONCE, digest=None):
    quote_digest, signed_owner, _mint, signed_amount = ve.quote_commitment(receipt)
    owner = owner or signed_owner
    amount = signed_amount if amount is None else amount
    digest = digest or quote_digest
    fields = [
        ve.EVENT_MARKER,
        ve._pubkey(POLICY),
        nonce.to_bytes(8, "little"),
        digest,
        ve._pubkey(destination),
        ve._pubkey(owner),
        amount.to_bytes(8, "little"),
    ]
    return "Program data: " + " ".join(map(b64, fields))


def transfer_tx(receipt):
    _, signed_owner, mint, amount = ve.quote_commitment(receipt)
    return {
        "slot": 350_000_700,
        "blockTime": 1793674950,
        "transaction": {
            "signatures": [DEPOSIT_SIG],
            "message": {"instructions": [
                {"programId": PROGRAM, "accounts": [POLICY, SOURCE, DEST]}
            ]},
        },
        "meta": {
            "err": None,
            "logMessages": [
                f"Program {PROGRAM} invoke [1]",
                f"Program {TOKEN} invoke [2]",
                f"Program {TOKEN} success",
                signed_event(receipt),
                f"Program {PROGRAM} success",
            ],
            "innerInstructions": [
                {"index": 0, "instructions": [
                    {"programId": TOKEN, "parsed": {
                        "type": "transferChecked",
                        "info": {
                            "source": SOURCE,
                            "destination": DEST,
                            "mint": mint,
                            "authority": POLICY,
                            "tokenAmount": {"amount": str(amount), "decimals": 6},
                        },
                    }}
                ]}
            ],
        },
    }


def proof(receipt, rpc):
    return ve.verify_source(
        receipt, solana=rpc, signature=DEPOSIT_SIG, program=PROGRAM,
        policy=POLICY, source_token=SOURCE, destination_token=DEST, nonce=NONCE,
        expected_mint=USDC_MINT,
    )


def test_source_proof_needs_signed_quote_exact_program_event_token_cpi_and_two_rpcs(tmp_path):
    path, _, _ = live_receipt(tmp_path)
    receipt = json.loads(path.read_text())
    tx = transfer_tx(receipt)
    rpc = VaultRpc(tx, tx)
    result = proof(receipt, rpc)
    assert result["source_verified"] is True, result
    assert result["destination_verified"] is False
    assert result["cross_chain_causality_proven"] is False
    assert result["source"]["nonce"] == NONCE
    assert result["source"]["deposit_owner"] == receipt["deposit"]["address"]
    assert len(result["rpcs"]) == 2
    assert len(rpc.calls) == 2


@pytest.mark.parametrize("tamper", [
    "missing_event", "spoofed_program", "modified_quote_digest", "changed_deposit_owner",
    "wrong_token_destination", "wrong_token_mint", "wrong_token_amount",
    "wrong_token_authority", "missing_cpi", "failed_transaction", "wrong_signature",
    "ambiguous_event", "altered_slot",
])
def test_source_proof_rejects_invalid_chain_evidence(tmp_path, tamper):
    path, _, _ = live_receipt(tmp_path)
    receipt = json.loads(path.read_text())
    tx = transfer_tx(receipt)
    logs = tx["meta"]["logMessages"]
    ix = tx["meta"]["innerInstructions"][0]["instructions"][0]
    info = ix["parsed"]["info"]
    if tamper == "missing_event":
        logs.pop(3)
    elif tamper == "spoofed_program":
        logs[3] = "Program NOT_THE_VAULT invoke [2]"
    elif tamper == "modified_quote_digest":
        logs[3] = signed_event(receipt, digest=bytes([45] * 32))
    elif tamper == "changed_deposit_owner":
        logs[3] = signed_event(receipt, owner=SOURCE)
    elif tamper == "wrong_token_destination":
        info["destination"] = SOURCE
    elif tamper == "wrong_token_mint":
        info["mint"] = SOURCE
    elif tamper == "wrong_token_amount":
        info["tokenAmount"]["amount"] = "1"
    elif tamper == "wrong_token_authority":
        info["authority"] = SOURCE
    elif tamper == "missing_cpi":
        tx["meta"]["innerInstructions"] = []
    elif tamper == "failed_transaction":
        tx["meta"]["err"] = {"InstructionError": [0, "Custom"]}
    elif tamper == "wrong_signature":
        tx["transaction"]["signatures"] = ["unrelated"]
    elif tamper == "ambiguous_event":
        logs.insert(4, logs[3])
    elif tamper == "altered_slot":
        tx["slot"] = None
    result = proof(receipt, VaultRpc(tx, tx))
    assert result["source_verified"] is False, (tamper, result)
    assert not result["cross_chain_causality_proven"]


def test_two_solana_rpcs_must_agree_on_finalized_transaction(tmp_path):
    path, _, _ = live_receipt(tmp_path)
    receipt = json.loads(path.read_text())
    tx_a = transfer_tx(receipt)
    tx_b = copy.deepcopy(tx_a)
    tx_b["slot"] += 1
    result = proof(receipt, VaultRpc(tx_a, tx_b))
    assert result["source_verified"] is False
    assert "disagree" in result["failure"]


def test_one_rpc_is_not_enough_for_independent_source_proof(tmp_path):
    path, _, _ = live_receipt(tmp_path)
    receipt = json.loads(path.read_text())
    result = proof(receipt, VaultRpc(transfer_tx(receipt)))
    assert result["source_verified"] is False
    assert "two distinct" in result["failure"]


def test_modified_receipt_does_not_validate_even_with_matching_event(tmp_path):
    path, _, _ = live_receipt(tmp_path)
    receipt = json.loads(path.read_text())
    tx = transfer_tx(receipt)
    receipt["quote_response"]["quote"]["amountIn"] = "1"
    result = proof(receipt, VaultRpc(tx, tx))
    assert result["source_verified"] is False
    assert "signed" in result["failure"] or "quote" in result["failure"]


def test_event_has_seven_strict_binary_fields(tmp_path):
    path, _, _ = live_receipt(tmp_path)
    receipt = json.loads(path.read_text())
    line = signed_event(receipt)
    assert len(ve._event_fields(line)) == 7
    with pytest.raises(ValueError):
        ve._event_fields(line + " " + b64(b"junk"))


@pytest.mark.parametrize("field", ["token_asset_id", "token_mint", "receipt_origin_asset", "receipt_amount"])
def test_untrusted_receipt_metadata_cannot_redefine_signed_quote_asset(tmp_path, field):
    path, _, _ = live_receipt(tmp_path)
    receipt = json.loads(path.read_text())
    tx = transfer_tx(receipt)  # canonical signed quote before tampering local metadata
    if field == "token_asset_id":
        receipt["tokens"]["origin"]["assetId"] = "spl:unrelated-token"
    elif field == "token_mint":
        receipt["tokens"]["origin"]["contractAddress"] = SOURCE
        tx["meta"]["innerInstructions"][0]["instructions"][0]["parsed"]["info"]["mint"] = SOURCE
    elif field == "receipt_origin_asset":
        receipt["quote_request"]["originAsset"] = "spl:unrelated-token"
    elif field == "receipt_amount":
        receipt["quote_request"]["amount"] = "1"
    result = proof(receipt, VaultRpc(tx, tx))
    assert result["source_verified"] is False, (field, result)
