"""Read-only evidence collection and integrity tests; no chain authorization."""
import copy
import hashlib
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from near_evidence import (
    EvidenceError,
    EvidenceStore,
    RpcUnavailable,
    NEGATIVE_CASES,
    collect_checkpoint,
    collect_receipt,
    collect_transaction,
    generate_negative_corpus,
    rpc_request,
    strict_json,
    validate_endpoints,
    verify_store,
)
from near_receipt_proof import ReceiptProofError, get_candidate


ALPHABET = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"


def b58(label):
    number = int.from_bytes(hashlib.sha256(label.encode()).digest(), "big")
    result = ""
    while number:
        number, remainder = divmod(number, 58)
        result = ALPHABET[remainder] + result
    return result


HEAD = b58("head")
BLOCK = b58("block")
EPOCH = b58("epoch")
NEXT_EPOCH = b58("next-epoch")
ROOT = b58("root")
BP_HASH = b58("bp-hash")
RECEIPT = b58("receipt")
RECEIVER = "league.testnet"
TRANSACTION = b58("transaction")
SENDER = "sender.testnet"


def response(request_id, result):
    return {"jsonrpc": "2.0", "id": request_id, "result": result}


def status_result():
    return {
        "chain_id": "testnet",
        "genesis_hash": HEAD,
        "protocol_version": 88,
    }


def header(height, block_hash, *, epoch=EPOCH):
    return {
        "height": height,
        "hash": block_hash,
        "prev_hash": b58(f"prev-{height}"),
        "epoch_id": epoch,
        "next_epoch_id": NEXT_EPOCH,
        "prev_state_root": ROOT,
        "outcome_root": ROOT,
        "timestamp": 1_800_000_000_000_000_000 + height,
        "next_bp_hash": BP_HASH,
        "block_merkle_root": ROOT,
    }


def proof_result(identifier):
    return {
        "outcome_proof": {
            "id": identifier,
            "block_hash": BLOCK,
            "proof": [{"hash": b58("outcome-sibling"), "direction": "Left"}],
            "outcome": {
                "executor_id": RECEIVER,
                "logs": [],
                "receipt_ids": [],
                "gas_burnt": 1,
                "tokens_burnt": "0",
                "status": {"SuccessValue": ""},
            },
        },
        "outcome_root_proof": [
            {"hash": b58("root-sibling"), "direction": "Right"}
        ],
        "block_header_lite": {
            "prev_block_hash": b58("proof-prev"),
            "inner_rest_hash": b58("proof-inner-rest"),
            "inner_lite": {
                "height": 103,
                "epoch_id": EPOCH,
                "next_epoch_id": NEXT_EPOCH,
                "prev_state_root": ROOT,
                "outcome_root": ROOT,
                "timestamp": 1_800_000_000_000_000_103,
                "next_bp_hash": BP_HASH,
                "block_merkle_root": ROOT,
            },
        },
        "block_proof": [
            {"hash": b58("block-sibling"), "direction": "Right"}
        ],
    }


def checkpoint_responses():
    head_block = header(100, HEAD)
    candidate_block = header(103, BLOCK)
    light = {
        "prev_block_hash": b58("light-prev"),
        "next_block_inner_hash": b58("next-light-inner"),
        "inner_rest_hash": b58("light-inner-rest"),
        "inner_lite": {
            "height": 103,
            "epoch_id": EPOCH,
            "next_epoch_id": NEXT_EPOCH,
            "prev_state_root": ROOT,
            "outcome_root": ROOT,
            "timestamp": candidate_block["timestamp"],
            "next_bp_hash": BP_HASH,
            "block_merkle_root": ROOT,
        },
        "next_bps": [
            {
                "account_id": "validator.testnet",
                "public_key": "ed25519:11111111111111111111111111111111",
                "stake": "1000",
            }
        ],
        "approvals_after_next": ["signature"],
    }
    return {
        ("status", ()): response("x3-evidence-status", status_result()),
        ("block", (("block_id", HEAD),)): response(
            "x3-evidence-head-block", {"header": head_block}
        ),
        ("next_light_client_block", (HEAD,)): response(
            "x3-evidence-next-light-client-block", light
        ),
        ("block", (("block_id", 103),)): response(
            "x3-evidence-candidate-block", {"header": candidate_block}
        ),
    }


def receipt_responses():
    tx = {
        "transaction_outcome": {
            "id": TRANSACTION,
            "outcome": {"status": {"SuccessValue": ""}},
        },
        "receipts_outcome": [
            {
                "id": RECEIPT,
                "outcome": {
                    "executor_id": RECEIVER,
                    "status": {"SuccessValue": ""},
                },
            }
        ],
    }
    return {
        ("status", ()): response("x3-evidence-status", status_result()),
        (
            "light_client_proof",
            (
                ("type", "receipt"),
                ("receipt_id", RECEIPT),
                ("receiver_id", RECEIVER),
                ("light_client_head", HEAD),
            ),
        ): response("x3-near-light-proof", proof_result(RECEIPT)),
        (
            "tx",
            (
                ("tx_hash", TRANSACTION),
                ("sender_account_id", SENDER),
                ("wait_until", "FINAL"),
            ),
        ): response("x3-evidence-transaction", tx),
    }


def transaction_responses():
    proof = proof_result(TRANSACTION)
    proof["outcome_proof"]["outcome"]["executor_id"] = SENDER
    proof["outcome_proof"]["outcome"]["status"] = {
        "SuccessReceiptId": RECEIPT
    }
    return {
        ("status", ()): response("x3-evidence-status", status_result()),
        (
            "light_client_proof",
            (
                ("type", "transaction"),
                ("transaction_hash", TRANSACTION),
                ("sender_id", SENDER),
                ("light_client_head", HEAD),
            ),
        ): response("x3-near-light-proof", proof),
    }


def freezable(value):
    if isinstance(value, dict):
        return tuple((key, freezable(item)) for key, item in value.items())
    if isinstance(value, list):
        return tuple(freezable(item) for item in value)
    return value


def fake_transport(*response_sets):
    response_sets = response_sets or (checkpoint_responses(),)

    def transport(endpoint, method, params, request_id, timeout_sec=20):
        host = endpoint.split("://", 1)[1]
        selected = response_sets[0] if len(response_sets) == 1 else response_sets[
            int(host.endswith("b.example"))
        ]
        key = (method, freezable(params))
        value = copy.deepcopy(selected[key])
        if value["id"] != request_id:
            raise AssertionError("test fixture request ID mismatch")
        return value, json.dumps(value, separators=(",", ":")).encode()

    return transport


class EvidenceStoreTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="x3-near-evidence-")
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)

    def test_endpoints_require_two_distinct_https_hosts(self):
        self.assertEqual(
            validate_endpoints(["https://rpc-a.example", "https://rpc-b.example"]),
            ["https://rpc-a.example", "https://rpc-b.example"],
        )
        for endpoints in (
            ["http://rpc-a.example", "https://rpc-b.example"],
            ["https://rpc-a.example", "https://rpc-a.example"],
            ["https://rpc-a.example"],
        ):
            with self.subTest(endpoints=endpoints):
                with self.assertRaises(EvidenceError):
                    validate_endpoints(endpoints)
        with self.assertRaises(EvidenceError):
            rpc_request(
                "https://rpc-a.example",
                "status",
                [],
                request_id="x3",
                timeout_sec=0,
            )

    def test_strict_json_rejects_duplicate_keys_and_nan(self):
        with self.assertRaises(EvidenceError):
            strict_json(b'{"id":1,"id":2}')
        with self.assertRaises(EvidenceError):
            strict_json(b'{"value":NaN}')

    def test_checkpoint_storage_is_content_addressed_and_unauthenticated(self):
        store = EvidenceStore(self.root, "testnet")
        rpc = fake_transport()
        manifest, path, duplicate = collect_checkpoint(
            ["https://rpc-a.example", "https://rpc-b.example"],
            network="testnet",
            head_hash=HEAD,
            store=store,
            rpc=rpc,
            retrieved_at="2026-11-06T00:00:00Z",
        )
        self.assertFalse(duplicate)
        self.assertTrue(path.is_file())
        self.assertEqual(manifest["status"], "complete_candidate")
        self.assertFalse(manifest["authenticated"])
        self.assertFalse(manifest["authorized_to_sign"])
        self.assertFalse(manifest["candidate"]["validator_set"]["commitment_verified"])
        self.assertEqual(
            manifest["candidate"]["classification"], "candidate_checkpoint_untrusted"
        )
        summary = verify_store(self.root)
        self.assertGreater(summary["artifacts"], 0)
        self.assertFalse(summary["authenticated"])

    def test_receipt_evidence_is_identical_across_providers_but_not_authenticated(self):
        store = EvidenceStore(self.root, "testnet")
        manifest, _, _ = collect_receipt(
            ["https://rpc-a.example", "https://rpc-b.example"],
            network="testnet",
            receipt_id=RECEIPT,
            receiver_id=RECEIVER,
            head_hash=HEAD,
            transaction_hash=TRANSACTION,
            sender_id=SENDER,
            store=store,
            rpc=fake_transport(receipt_responses()),
            retrieved_at="2026-11-06T00:00:00Z",
        )
        self.assertEqual(manifest["status"], "complete_candidate")
        self.assertFalse(manifest["authenticated"])
        self.assertFalse(manifest["authorized_to_sign"])
        self.assertEqual(manifest["candidate"]["receipt_id"], RECEIPT)
        self.assertEqual(
            manifest["candidate"]["classification"],
            "syntactically_valid_untrusted_receipt_proof",
        )
        self.assertEqual(manifest["candidate"]["transaction_hash"], TRANSACTION)
        verify_store(self.root)

    def test_transaction_success_receipt_id_is_not_misclassified(self):
        store = EvidenceStore(self.root, "testnet")
        manifest, _, _ = collect_transaction(
            ["https://rpc-a.example", "https://rpc-b.example"],
            network="testnet",
            transaction_hash=TRANSACTION,
            sender_id=SENDER,
            head_hash=HEAD,
            store=store,
            rpc=fake_transport(transaction_responses()),
            retrieved_at="2026-11-06T00:00:00Z",
        )
        self.assertEqual(manifest["status"], "complete_candidate")
        self.assertFalse(manifest["authenticated"])
        self.assertEqual(
            manifest["candidate"]["classification"],
            "syntactically_valid_untrusted_transaction_proof",
        )
        verify_store(self.root)

    def test_transaction_proof_for_another_signer_is_not_substituted(self):
        responses = transaction_responses()
        proof_key = (
            "light_client_proof",
            (
                ("type", "transaction"),
                ("transaction_hash", TRANSACTION),
                ("sender_id", SENDER),
                ("light_client_head", HEAD),
            ),
        )
        responses[proof_key] = copy.deepcopy(responses[proof_key])
        responses[proof_key]["result"]["outcome_proof"]["outcome"][
            "executor_id"
        ] = "attacker.testnet"
        store = EvidenceStore(self.root, "testnet")
        manifest, _, _ = collect_transaction(
            ["https://rpc-a.example", "https://rpc-b.example"],
            network="testnet",
            transaction_hash=TRANSACTION,
            sender_id=SENDER,
            head_hash=HEAD,
            store=store,
            rpc=fake_transport(responses),
            retrieved_at="2026-11-06T00:00:00Z",
        )
        self.assertEqual(manifest["status"], "incomplete")
        self.assertTrue(
            all(
                item["error"] == "transaction proof signer does not match sender"
                for item in manifest["providers"]
            )
        )

    def test_reopen_after_interrupted_index_append_recovers_orphan_manifest(self):
        # The process can die after durable manifest creation, but before
        # adding its index record. Reopening the store must repair the index,
        # not silently classify the unindexed manifest as a complete duplicate.
        import os

        root = self.root
        store = EvidenceStore(root, "testnet")
        first_args = dict(
            network="testnet",
            head_hash=HEAD,
            store=store,
            rpc=fake_transport(),
            retrieved_at="2026-11-06T00:00:00Z",
        )
        original_open = os.open
        index_path = root / "testnet" / "index.jsonl"

        def fail_index_open(path, *args, **kwargs):
            if Path(path) == index_path:
                raise OSError("simulated crash just before index append")
            return original_open(path, *args, **kwargs)

        with patch("near_evidence.os.open", side_effect=fail_index_open):
            with self.assertRaisesRegex(OSError, "simulated crash"):
                collect_checkpoint(
                    ["https://rpc-a.example", "https://rpc-b.example"], **first_args
                )
        self.assertEqual(
            len(list((root / "testnet" / "manifests").glob("*.json"))), 1
        )
        self.assertFalse(index_path.exists())
        with self.assertRaises(EvidenceError):
            verify_store(root)

        # New EvidenceStore instance models a new process reusing the same
        # content-addressed directory after an interrupted collector.
        resumed = EvidenceStore(root, "testnet")
        manifest, _, duplicate = collect_checkpoint(
            ["https://rpc-a.example", "https://rpc-b.example"],
            network="testnet",
            head_hash=HEAD,
            store=resumed,
            rpc=fake_transport(),
            retrieved_at="2026-11-06T01:00:00Z",
        )
        self.assertTrue(duplicate)
        self.assertEqual(manifest["retrieved_at"], "2026-11-06T00:00:00Z")
        self.assertTrue(index_path.exists(), "crash orphan was not recovered into index")
        lines = index_path.read_bytes().splitlines()
        self.assertEqual(len(lines), 1, "interrupted manifest was not indexed on retry")
        self.assertEqual(json.loads(lines[0])["manifest_id"], manifest["manifest_id"])
        verify_store(root)

    def test_hard_killed_collector_process_recovers_manifest_without_downgrading(self):
        # This is a REAL isolated subprocess terminated with os._exit before
        # the index append, not a simulated exception in the test process.
        # No validator or public RPC is started or stopped.
        child = """
import os
import sys
from pathlib import Path
from unittest.mock import patch
from near_evidence import EvidenceStore, collect_checkpoint
from test_near_evidence import HEAD, fake_transport
root = Path(sys.argv[1])
index = root / "testnet" / "index.jsonl"
raw_open = os.open
def terminate_before_index(path, *args, **kwargs):
    if Path(path) == index:
        os._exit(73)
    return raw_open(path, *args, **kwargs)
with patch("near_evidence.os.open", side_effect=terminate_before_index):
    collect_checkpoint(
        ["https://rpc-a.example", "https://rpc-b.example"],
        network="testnet", head_hash=HEAD,
        store=EvidenceStore(root, "testnet"),
        rpc=fake_transport(), retrieved_at="2026-11-06T00:00:00Z"
    )
"""
        env = dict(os.environ)
        env["PYTHONPATH"] = str(Path(__file__).resolve().parent)
        crashed = subprocess.run(
            [sys.executable, "-c", child, str(self.root)],
            env=env, capture_output=True, text=True, timeout=15,
        )
        self.assertEqual(crashed.returncode, 73, crashed.stderr)
        self.assertEqual(
            len(list((self.root / "testnet" / "manifests").glob("*.json"))), 1
        )
        with self.assertRaises(EvidenceError):
            verify_store(self.root)

        recovered, _, duplicate = collect_checkpoint(
            ["https://rpc-a.example", "https://rpc-b.example"],
            network="testnet",
            head_hash=HEAD,
            store=EvidenceStore(self.root, "testnet"),
            rpc=fake_transport(),
            retrieved_at="2026-11-06T01:00:00Z",
        )
        self.assertTrue(duplicate)
        self.assertEqual(recovered["status"], "complete_candidate")
        self.assertFalse(recovered["authenticated"])
        self.assertFalse(recovered["authorized_to_sign"])
        self.assertEqual(
            len((self.root / "testnet" / "index.jsonl").read_bytes().splitlines()), 1
        )
        verify_store(self.root)

    def test_tampered_crash_orphan_is_refused_not_reindexed(self):
        # No committed index can vouch for a manifest altered between an
        # interrupted write and a later recovery attempt.
        original_open = os.open
        index = self.root / "testnet" / "index.jsonl"

        def crash_before_index(path, *args, **kwargs):
            if Path(path) == index:
                raise OSError("interrupted before index")
            return original_open(path, *args, **kwargs)

        with patch("near_evidence.os.open", side_effect=crash_before_index):
            with self.assertRaises(OSError):
                collect_checkpoint(
                    ["https://rpc-a.example", "https://rpc-b.example"],
                    network="testnet", head_hash=HEAD,
                    store=EvidenceStore(self.root, "testnet"),
                    rpc=fake_transport(), retrieved_at="2026-11-06T00:00:00Z",
                )
        orphan = next((self.root / "testnet" / "manifests").glob("*.json"))
        changed = json.loads(orphan.read_text())
        changed["status"] = "disagreement"  # No change to subject digest.
        orphan.write_text(json.dumps(changed, sort_keys=True) + "\n")
        with self.assertRaisesRegex(EvidenceError, "manifest subject digest mismatch"):
            collect_checkpoint(
                ["https://rpc-a.example", "https://rpc-b.example"],
                network="testnet", head_hash=HEAD,
                store=EvidenceStore(self.root, "testnet"),
                rpc=fake_transport(), retrieved_at="2026-11-06T01:00:00Z",
            )
        self.assertFalse(index.exists(), "tampered orphan was incorrectly indexed")

    def test_simultaneous_collector_processes_write_one_index_record(self):
        # Four actual processes contend on the same content-addressed
        # manifest. A lock must prevent duplicate index rows.
        child = """
import sys
from pathlib import Path
from near_evidence import EvidenceStore, collect_checkpoint
from test_near_evidence import HEAD, fake_transport
collect_checkpoint(
    ["https://rpc-a.example", "https://rpc-b.example"],
    network="testnet", head_hash=HEAD,
    store=EvidenceStore(Path(sys.argv[1]), "testnet"),
    rpc=fake_transport(), retrieved_at="2026-11-06T00:00:00Z",
)
"""
        env = dict(os.environ)
        env["PYTHONPATH"] = str(Path(__file__).resolve().parent)
        workers = [
            subprocess.Popen(
                [sys.executable, "-c", child, str(self.root)],
                env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            )
            for _ in range(4)
        ]
        for worker in workers:
            stdout, stderr = worker.communicate(timeout=20)
            self.assertEqual(worker.returncode, 0, stderr.decode() or stdout.decode())
        self.assertEqual(
            len((self.root / "testnet" / "index.jsonl").read_bytes().splitlines()), 1
        )
        self.assertEqual(
            len(list((self.root / "testnet" / "manifests").glob("*.json"))), 1
        )
        verify_store(self.root)

    def test_truncated_index_refuses_to_repair_silently(self):
        store = EvidenceStore(self.root, "testnet")
        collect_checkpoint(
            ["https://rpc-a.example", "https://rpc-b.example"],
            network="testnet",
            head_hash=HEAD,
            store=store,
            rpc=fake_transport(),
            retrieved_at="2026-11-06T00:00:00Z",
        )
        index = self.root / "testnet" / "index.jsonl"
        corrupted = index.read_bytes()[:-1]  # Remove terminating newline.
        index.write_bytes(corrupted)
        with self.assertRaisesRegex(EvidenceError, "partial evidence index"):
            collect_checkpoint(
                ["https://rpc-a.example", "https://rpc-b.example"],
                network="testnet",
                head_hash=HEAD,
                store=EvidenceStore(self.root, "testnet"),
                rpc=fake_transport(),
                retrieved_at="2026-11-06T01:00:00Z",
            )
        self.assertEqual(index.read_bytes(), corrupted, "corruption was silently rewritten")
        with self.assertRaises(EvidenceError):
            verify_store(self.root)

    def test_duplicate_evidence_is_not_appended_twice(self):
        store = EvidenceStore(self.root, "testnet")
        args = dict(
            network="testnet",
            head_hash=HEAD,
            store=store,
            rpc=fake_transport(),
        )
        collect_checkpoint(
            ["https://rpc-a.example", "https://rpc-b.example"],
            retrieved_at="2026-11-06T00:00:00Z",
            **args,
        )
        duplicate_manifest, _, duplicate = collect_checkpoint(
            ["https://rpc-a.example", "https://rpc-b.example"],
            retrieved_at="2026-11-06T01:00:00Z",
            **args,
        )
        self.assertTrue(duplicate)
        self.assertEqual(duplicate_manifest["retrieved_at"], "2026-11-06T00:00:00Z")
        index = (self.root / "testnet" / "index.jsonl").read_text().splitlines()
        self.assertEqual(len(index), 1)

    def test_provider_disagreement_is_preserved_and_reported(self):
        first = checkpoint_responses()
        second = checkpoint_responses()
        light_key = ("next_light_client_block", (HEAD,))
        second[light_key] = copy.deepcopy(second[light_key])
        second[light_key]["result"]["inner_lite"]["next_epoch_id"] = b58("other-epoch")
        candidate_key = (
            "block",
            (("block_id", 103),),
        )
        second[candidate_key] = copy.deepcopy(second[candidate_key])
        second[candidate_key]["result"]["header"]["next_epoch_id"] = b58("other-epoch")
        store = EvidenceStore(self.root, "testnet")
        manifest, _, _ = collect_checkpoint(
            ["https://rpc-a.example", "https://rpc-b.example"],
            network="testnet",
            head_hash=HEAD,
            store=store,
            rpc=fake_transport(first, second),
            retrieved_at="2026-11-06T00:00:00Z",
        )
        self.assertEqual(manifest["status"], "disagreement")
        self.assertIsNotNone(manifest["candidate"])
        self.assertFalse(manifest["candidate"]["provider_consensus"])
        verify_store(self.root)

    def test_three_rpc_split_brain_cannot_claim_provider_consensus(self):
        # Two coherent but untrusted RPC responses against a third conflicting
        # epoch must not be mistaken for authenticated finalized consensus.
        honest = checkpoint_responses()
        conflicting = checkpoint_responses()
        light_key = ("next_light_client_block", (HEAD,))
        conflicting[light_key] = copy.deepcopy(conflicting[light_key])
        conflicting[light_key]["result"]["inner_lite"]["next_epoch_id"] = b58("forked-epoch")
        block_key = ("block", (("block_id", 103),))
        conflicting[block_key] = copy.deepcopy(conflicting[block_key])
        conflicting[block_key]["result"]["header"]["next_epoch_id"] = b58("forked-epoch")
        store = EvidenceStore(self.root, "testnet")
        manifest, _, _ = collect_checkpoint(
            ["https://rpc-a.example", "https://rpc-b.example", "https://rpc-c.example"],
            network="testnet",
            head_hash=HEAD,
            store=store,
            rpc=fake_transport(honest, conflicting),
            retrieved_at="2026-11-06T00:00:00Z",
        )
        self.assertEqual(manifest["status"], "disagreement")
        self.assertEqual(len(manifest["providers"]), 3)
        self.assertTrue(all("error" not in provider for provider in manifest["providers"]))
        self.assertFalse(manifest["candidate"]["provider_consensus"])
        self.assertEqual(manifest["candidate"]["classification"], "candidate_checkpoint_untrusted")
        self.assertFalse(manifest["authenticated"])
        self.assertFalse(manifest["authorized_to_sign"])
        verify_store(self.root)

    def test_rpc_interrupted_mid_checkpoint_does_not_claim_provider_consensus(self):
        def midflight_timeout(endpoint, method, params, request_id, timeout_sec=20):
            if endpoint.endswith("b.example") and method == "next_light_client_block":
                raise RpcUnavailable("fault-injected RPC dropped after status/head")
            return fake_transport()(endpoint, method, params, request_id, timeout_sec)

        store = EvidenceStore(self.root, "testnet")
        manifest, _, _ = collect_checkpoint(
            ["https://rpc-a.example", "https://rpc-b.example"],
            network="testnet",
            head_hash=HEAD,
            store=store,
            rpc=midflight_timeout,
            retrieved_at="2026-11-06T00:00:00Z",
        )
        self.assertEqual(manifest["status"], "incomplete")
        self.assertEqual(len(manifest["providers"]), 2)
        self.assertIn("fault-injected RPC", manifest["providers"][1]["error"])
        self.assertFalse(manifest["candidate"]["provider_consensus"],
                         "one working provider does not establish a two-provider quorum")
        self.assertFalse(manifest["authenticated"])
        self.assertFalse(manifest["authorized_to_sign"])
        verify_store(self.root)

        # Network recovery creates a new separately indexed candidate.
        recovered = EvidenceStore(self.root, "testnet")
        successful, _, duplicate = collect_checkpoint(
            ["https://rpc-a.example", "https://rpc-b.example"],
            network="testnet",
            head_hash=HEAD,
            store=recovered,
            rpc=fake_transport(),
            retrieved_at="2026-11-06T01:00:00Z",
        )
        self.assertFalse(duplicate)
        self.assertEqual(successful["status"], "complete_candidate")
        self.assertFalse(successful["authenticated"])
        self.assertEqual(len((self.root / "testnet" / "index.jsonl").read_bytes().splitlines()), 2)
        verify_store(self.root)

    def test_partial_receipt_and_transaction_proofs_never_emit_candidates(self):
        # A lone surviving RPC is not a quorum. Preserve partial evidence
        # without giving downstream clients a candidate they can mistake for
        # a complete multi-provider proof.
        for kind in ("receipt", "transaction"):
            with self.subTest(kind=kind):
                responses = (
                    receipt_responses() if kind == "receipt" else transaction_responses()
                )
                unmodified = fake_transport(responses)

                def interrupted(endpoint, method, params, request_id, timeout_sec=20):
                    if endpoint.endswith("rpc-b.example") and method == "light_client_proof":
                        raise RpcUnavailable("fault-injected proof RPC timeout")
                    return unmodified(endpoint, method, params, request_id, timeout_sec)

                store_root = self.root / kind
                store = EvidenceStore(store_root, "testnet")
                params = dict(
                    endpoints=["https://rpc-a.example", "https://rpc-b.example"],
                    network="testnet",
                    head_hash=HEAD,
                    store=store,
                    rpc=interrupted,
                    retrieved_at="2026-11-06T00:00:00Z",
                )
                if kind == "receipt":
                    manifest, _, _ = collect_receipt(
                        receipt_id=RECEIPT, receiver_id=RECEIVER, **params
                    )
                else:
                    manifest, _, _ = collect_transaction(
                        transaction_hash=TRANSACTION, sender_id=SENDER, **params
                    )
                self.assertEqual(manifest["status"], "incomplete")
                self.assertIsNone(manifest["candidate"])
                self.assertFalse(manifest["authenticated"])
                self.assertFalse(manifest["authorized_to_sign"])
                self.assertIn("fault-injected", manifest["providers"][1]["error"])
                verify_store(store_root)

    def test_wrong_network_and_unavailable_provider_are_never_substituted(self):
        wrong_network = checkpoint_responses()
        wrong_network[("status", ())] = response(
            "x3-evidence-status", status_result() | {"chain_id": "mainnet"}
        )
        store = EvidenceStore(self.root, "testnet")
        manifest, _, _ = collect_checkpoint(
            ["https://rpc-a.example", "https://rpc-b.example"],
            network="testnet",
            head_hash=HEAD,
            store=store,
            rpc=fake_transport(wrong_network),
            retrieved_at="2026-11-06T00:00:00Z",
        )
        self.assertEqual(manifest["status"], "incomplete")
        self.assertTrue(all("error" in item for item in manifest["providers"]))
        self.assertIsNone(manifest["candidate"])

        def unavailable(endpoint, method, params, request_id, timeout_sec=20):
            if endpoint.endswith("b.example"):
                raise EvidenceError("provider unavailable")
            return fake_transport(receipt_responses())(
                endpoint, method, params, request_id, timeout_sec
            )

        receipt_store = EvidenceStore(self.root / "partial", "testnet")
        manifest, _, _ = collect_receipt(
            ["https://rpc-a.example", "https://rpc-b.example"],
            network="testnet",
            receipt_id=RECEIPT,
            receiver_id=RECEIVER,
            head_hash=HEAD,
            store=receipt_store,
            rpc=unavailable,
            retrieved_at="2026-11-06T00:00:00Z",
        )
        self.assertEqual(manifest["status"], "incomplete")
        failed_provider = next(
            item for item in manifest["providers"] if item["endpoint"].endswith("b.example")
        )
        self.assertEqual(failed_provider["error"], "provider unavailable")
        self.assertIn("calls", failed_provider)
        verify_store(self.root / "partial")

    def test_tampered_artifact_fails_offline_verification(self):
        store = EvidenceStore(self.root, "testnet")
        manifest, _, _ = collect_checkpoint(
            ["https://rpc-a.example", "https://rpc-b.example"],
            network="testnet",
            head_hash=HEAD,
            store=store,
            rpc=fake_transport(),
            retrieved_at="2026-11-06T00:00:00Z",
        )
        artifact = manifest["artifacts"][0]
        path = self.root / "testnet" / artifact["path"]
        path.write_bytes(b'{"tampered":true}\n')
        with self.assertRaises(EvidenceError):
            verify_store(self.root)

    def test_real_derived_negative_corpus_has_all_twelve_cases(self):
        store = EvidenceStore(self.root, "testnet")
        collect_checkpoint(
            ["https://rpc-a.example", "https://rpc-b.example"],
            network="testnet",
            head_hash=HEAD,
            store=store,
            rpc=fake_transport(),
            retrieved_at="2026-11-06T00:00:00Z",
        )
        collect_receipt(
            ["https://rpc-a.example", "https://rpc-b.example"],
            network="testnet",
            receipt_id=RECEIPT,
            receiver_id=RECEIVER,
            head_hash=HEAD,
            store=store,
            rpc=fake_transport(receipt_responses()),
            retrieved_at="2026-11-06T00:01:00Z",
        )
        manifest_path, mutation_manifest = generate_negative_corpus(
            self.root, network="testnet"
        )
        self.assertTrue(manifest_path.is_file())
        self.assertEqual(set(mutation_manifest["cases"]), set(NEGATIVE_CASES))
        wrong_network = json.loads(
            (
                manifest_path.parent
                / mutation_manifest["cases"]["wrong_network"]["path"]
            ).read_text()
        )
        self.assertEqual(wrong_network["context"]["network"], "mainnet")
        self.assertFalse(wrong_network["authenticated"])
        incomplete = json.loads(
            (
                manifest_path.parent
                / mutation_manifest["cases"]["truncated_or_malformed_proof"]["path"]
            ).read_text()
        )
        self.assertNotIn("outcome_root_proof", incomplete["payload"]["result"])
        summary = verify_store(self.root, require_negative_corpus=True)
        self.assertEqual(summary["negative_fixtures"], 12)

        negative_dir = manifest_path.parent
        parser_rejected = (
            "wrong_receipt_id",
            "wrong_receiving_contract",
            "modified_outcome_status",
            "truncated_or_malformed_proof",
            "cross_swap_evidence_substitution",
        )
        for case in parser_rejected:
            with self.subTest(case=case):
                document = json.loads(
                    (negative_dir / f"{case}.json").read_text()
                )
                with self.assertRaises(ReceiptProofError):
                    get_candidate(
                        document["payload"],
                        receipt_id=RECEIPT,
                        receiver_id=RECEIVER,
                        light_client_head=HEAD,
                    )


if __name__ == "__main__":
    unittest.main()
