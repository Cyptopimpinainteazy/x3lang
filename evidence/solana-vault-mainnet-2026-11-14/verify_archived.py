#!/usr/bin/env python3
"""Verify this fixed historical pilot offline. Never uses keys, RPC or signing."""
import argparse
import base64
import hashlib
import json
from pathlib import Path
import sys

REPO = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(REPO / "x3-lang"))
sys.path.insert(0, str(REPO / "x3-lang" / "solana"))
import near_intents as ni
import vault_client as vc
from solders.transaction import Transaction
from solders.message import Message
from solders.instruction import Instruction, AccountMeta
from solders.pubkey import Pubkey

OWNER = "58tbMfadmi8HWzUndLqkX3prBbkiZMEaYMTDYSzhB3cQ"
AGENT = "6R4nioW9tPke3PFuWLaQY3gRAwM5SMi1vbY925WJZtPS"
PROGRAM = "68ssmMccnqQFUHWipQKy1W4DcycjA9jip9ZUi7EJyUZn"
POLICY = "ATXS8Y9cDVAN3BxTDa7CXt7FuHU7Y6LJHDqGDDuQtXGC"
VAULT = "FTToXuCe3RG6L78Ctpno5SL5BpXDNAYc6JnjhjqGWndU"
DEST = "9BCGvA3kLoHCGv93hLUMK4HqSXmi4ghdNJDa7aYGMZvd"
MINT = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v"
SOURCE_TX = "3Bu3eWLwiSLoTNHgLPCzn5JyTAUnfzbXJENQG36oEt7kZm9e1M4rRB5CtUZxG92gRaGmPvuFZUQ1EqBzkL9rdLBf"
NEAR_TX = "CXoXgJ5FwcSZcyPDVu7PrNby7KULfY8bJxUBRT9fu4ia"
DIGEST = "e53cd55f8ef6b923fe312fa213e6f9f43a0b6d4f230b411f59e58c1589b11b3d"
CODE_SHA = "aa96de859a8d29c4e41eece9ff336545f3f99e40c8412f78bc16dd25b562c270"


def require(ok, detail):
    if not ok:
        raise ValueError(detail)


def verify(root):
    def read(name):
        return json.loads((root / name).read_text())

    # Manifest detects accidental byte changes; fixed constants and semantic checks
    # below still apply even if someone regenerates the manifest.
    manifest = read("manifest.json")
    require(bool(manifest["sha256"]), "Empty manifest")
    for name, digest in manifest["sha256"].items():
        require(Path(name).name == name and name not in {"manifest.json", "verify_archived.py"},
                "Invalid manifest entry")
        require(hashlib.sha256((root / name).read_bytes()).hexdigest() == digest, "Checksum: " + name)
    receipt = read("live-quote.near-receipt.json")
    require(hashlib.sha256((root / "payment-proposed.x3").read_bytes()).hexdigest()
            == receipt["program_sha256"], "X3 source identity mismatch")
    _, leg = ni.compile_program(str(root / "payment-proposed.x3"))
    require(leg["amount"] == 5000000 and leg["recipient"] == "x3lang.near"
            and leg["refund_to"] == OWNER, "Compiled X3 payment mismatch")
    valid, reason = ni.check_oneclick_quote_signature(receipt["quote_response"])
    require(valid, "Provider signature: " + reason)
    terms = vc.approval_from_receipt(receipt)
    require(terms["quote_digest"].hex() == DIGEST and terms["amount"] == 5000000
            and vc.pubkey_text(terms["mint"]) == MINT
            and vc.pubkey_text(terms["destination_token"]) == DEST, "Signed quote binding mismatch")
    request = receipt["quote_response"]["quoteRequest"]
    quote = receipt["quote_response"]["quote"]
    require(request["recipient"] == "x3lang.near" and request["refundTo"] == OWNER
            and quote["minAmountOut"] == "4974757" and quote["refundFee"] == "10034",
            "Signed payout or refund mismatch")
    # Fixed historical issue time checks freshness then, not freshness today.
    ni.verify_quote(receipt["leg"], receipt["quote_request"], receipt["quote_response"],
                    now=ni.parse_timestamp(receipt["quote_response"]["timestamp"]))
    ve = vc._vault_evidence()
    source = read("finalized-execution-primary.json")
    tx = source["transaction"]
    facts = ve._facts(tx, tx_signature=SOURCE_TX, program=PROGRAM, policy=POLICY,
                      source_token=VAULT, destination_token=DEST, nonce=1,
                      digest=bytes.fromhex(DIGEST), deposit_owner=quote["depositAddress"],
                      mint=MINT, amount=5000000)
    submission = read("execution-submission.json")
    message = base64.b64decode(submission["messageBase64"], validate=True)
    wire = bytes([1]) + ni.ed25519.b58decode(SOURCE_TX) + message
    signed = Transaction.from_bytes(wire)
    signed.verify()
    require(str(signed.message.account_keys[0]) == AGENT
            and signed.message.header.num_required_signatures == 1
            and tx["meta"]["fee"] == 5000, "Agent signer or fee mismatch")
    ix = vc.ix_execute(vc.pubkey_bytes(PROGRAM), vc.pubkey_bytes(AGENT), vc.pubkey_bytes(POLICY),
                       vc.pubkey_bytes(VAULT), vc.pubkey_bytes(DEST), vc.pubkey_bytes(MINT),
                       nonce=1, quote_digest=bytes.fromhex(DIGEST))
    exact = Instruction(Pubkey.from_bytes(ix.program_id), ix.data,
                        [AccountMeta(Pubkey.from_bytes(m.pubkey), m.signer, m.writable) for m in ix.accounts])
    expected = Message.new_with_blockhash([exact], Pubkey.from_string(AGENT), signed.message.recent_blockhash)
    require(bytes(signed.message) == bytes(expected), "Signed execution instruction mismatch")
    expected_owner_signatures = {
        "deployment": "62a2Ufv9Qtw1HTNniwm2RTmD9Q3J7GfiCUrKXeCQkWULmWoDwPkJhPMwES2HENacjSLFNsX2ZZVD7c2LWvfweqtJ",
        "setup": "32FhzL7fxQKPgTTopxjJnwGXMpTgaGVgAzDDsQkevw4YY1kDvKuHdPmjV773ynWcr9m6dPofuedKRPin8abPh9Un",
        "approval": "4ReKsqwxpwtyx2CNmah6UCkHWgb4GbiJjXGTZLwtHaMMBoWv8vqSki4XXzDrBmqgURJSX7A1fHAKm3jSf3fnWwkV",
    }
    owner_steps = {}
    for label, debit in (("deployment", 838120), ("setup", 9788320), ("approval", 1493440)):
        observed = read(label + "-transaction.json")["result"]
        signed_owner = Transaction.from_bytes(base64.b64decode(observed["transaction"][0], validate=True))
        signed_owner.verify()
        require(str(signed_owner.signatures[0]) == expected_owner_signatures[label],
                "Historical owner transaction substituted: " + label)
        require(str(signed_owner.message.account_keys[0]) == OWNER
                and signed_owner.message.header.num_required_signatures == 1
                and observed["meta"]["err"] is None and observed["meta"]["fee"] == 5000
                and observed["meta"]["preBalances"][0] - observed["meta"]["postBalances"][0] == debit,
                "Owner step mismatch: " + label)
        owner_steps[label] = str(signed_owner.signatures[0])
    approval = read("finalized-approval.json")
    require(owner_steps["approval"] == approval["signature"]
            and approval["policyState"]["pending_quote_digest"] == DIGEST
            and approval["policyState"]["pending_nonce"] == 1
            and approval["policyState"]["pending_amount"] == 5000000
            and approval["policyState"]["pending_destination"] == DEST
            and approval["policyState"]["pending_destination_owner"] == quote["depositAddress"]
            and approval["policyState"]["pending_until"] == approval["validUntil"] == 1794657709
            and approval["policyState"]["expires_at"] == 1794741558,
            "Finalized owner approval mismatch")
    program, data, policy, vault, agent = read("finalized-accounts.json")["result"]["value"]
    code = base64.b64decode(data["data"][0], validate=True)
    require(program["executable"] and program["owner"] == data["owner"] == "BPFLoaderUpgradeab1e11111111111111111111111"
            and not data["executable"] and len(code) == 104104 + 45
            and code[:4] == bytes([3, 0, 0, 0]) and code[12:45] == bytes([1]) + vc.pubkey_bytes(OWNER)
            and hashlib.sha256(code[45:]).hexdigest() == CODE_SHA, "Deployed artifact hash mismatch")
    pol = vc.decode_policy(base64.b64decode(policy["data"][0], validate=True))
    balance = vc.decode_token_account(base64.b64decode(vault["data"][0], validate=True))
    require(policy["owner"] == PROGRAM and pol["owner"] == OWNER and pol["agent"] == AGENT
            and pol["mint"] == MINT and pol["vault_token"] == VAULT
            and pol["last_nonce"] == 1 and not pol["pending"] and pol["expires_at"] == 1794741558
            and pol["max_per_payment"] == pol["max_per_day"] == 5000000
            and len(pol["spends"]) == 1 and pol["spends"][0]["amount"] == 5000000
            and balance["amount"] == 0 and agent["lamports"] == 995000, "Consumed policy mismatch")
    near = read("near-payout-transaction.json")
    token = ni.nep141_contract(request["destinationAsset"])
    require(near["final_execution_status"] == "FINAL" and "SuccessValue" in near["status"]
            and near["transaction"]["hash"] == NEAR_TX, "NEAR payout not finalized")
    transfers = []
    for entry in near["receipts_outcome"]:
        outcome = entry["outcome"]
        require("Failure" not in outcome["status"], "NEAR receipt failed")
        for line in outcome["logs"]:
            if outcome["executor_id"] != token or not line.startswith("EVENT_JSON:"):
                continue
            event = json.loads(line[len("EVENT_JSON:"):])
            if event.get("standard") == "nep141" and event.get("event") == "ft_transfer":
                transfers += [d for d in event["data"] if d.get("old_owner_id") == "intents.near"
                              and d.get("new_owner_id") == "x3lang.near" and d.get("amount") == "4999746"]
    require(len(transfers) == 1, "No unique matching NEAR token transfer")
    provider = read("provider-latest-status.json")
    details = provider["swapDetails"]
    require(provider["status"] == "SUCCESS" and details["amountIn"] == "5000000"
            and details["amountOut"] == "4999746"
            and details["originChainTxHashes"][0]["hash"] == SOURCE_TX
            and details["destinationChainTxHashes"][0]["hash"] == NEAR_TX, "Provider association mismatch")
    before, after = read("near-before-execution.json"), read("near-after-execution.json")
    require(int(after["balance"]) - int(before["balance"]) == 4999746, "Balance delta mismatch")
    return {"archived_checks": "passed", "source_slot": facts["slot"], "input_usdc": "5",
            "output_usdc": "4.999746", "recipient": "x3lang.near",
            "source_rpc_quorum": False, "cryptographic_solver_causality": False,
            "live_state_refreshed": False, "signed_or_sent": False}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--evidence", type=Path, default=Path(__file__).resolve().parent)
    args = parser.parse_args()
    try:
        print(json.dumps(verify(args.evidence), indent=2))
    except Exception as error:
        print(json.dumps({"archived_checks": "failed", "error": str(error)}))
        raise SystemExit(1)
