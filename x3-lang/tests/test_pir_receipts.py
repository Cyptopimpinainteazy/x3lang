import json
import os
import sqlite3
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import test_pir_agent_runtime as fixture
from agent_runtime import Action, Adapter, canonical, digest
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from pir_receipts import (
    DOMAIN,
    ReceiptLedger,
    issue_local,
    issue_reference,
    verify_local_provenance,
    verify_receipt,
)
from test_pir_agent_runtime import ARTIFACT, SCOPE, raw

FIELDS = (
    "profile",
    "status",
    "scope",
    "context",
    "subject",
    "source_sha256",
    "policy_sha256",
    "native_id",
    "artifact_sha256",
)


class ReferenceTests(unittest.TestCase):
    def setUp(self):
        self.key = Ed25519PrivateKey.generate()
        self.claims = {
            "profile": "evm",
            "status": "observed_unverified",
            "scope": "sandbox:pir06",
            "context": "evm:84532:fixture",
            "subject": "0x" + "33" * 20,
            "source_sha256": "11" * 32,
            "policy_sha256": "22" * 32,
            "native_id": "33" * 32,
            "artifact_sha256": "44" * 32,
            "result_sha256": None,
        }
        self.evidence = b"unverified local EVM transcript"

    def expected(self, c):
        return {k: c[k] for k in FIELDS}

    def test_reference_signature_context_and_evidence(self):
        receipt = issue_reference(self.claims, self.evidence, self.key)
        self.assertTrue(receipt.startswith(b'{"payload":'))
        p = verify_receipt(
            receipt, self.evidence, raw(self.key), self.expected(self.claims)
        )
        self.assertEqual(p["status"], "observed_unverified")
        self.assertEqual(p["evidence_sha256"], digest(self.evidence))
        with self.assertRaises(ValueError):
            verify_receipt(
                receipt, b"changed", raw(self.key), self.expected(self.claims)
            )
        with self.assertRaises(ValueError):
            verify_receipt(
                receipt,
                self.evidence,
                raw(Ed25519PrivateKey.generate()),
                self.expected(self.claims),
            )
        with self.assertRaises(ValueError):
            verify_receipt(
                receipt,
                self.evidence,
                raw(self.key),
                {**self.expected(self.claims), "context": "other"},
            )
        with self.assertRaises(ValueError):
            verify_receipt(
                receipt + b"\n",
                self.evidence,
                raw(self.key),
                self.expected(self.claims),
            )
        changed = json.loads(receipt)
        changed["payload"]["policy_sha256"] = "55" * 32
        with self.assertRaises(ValueError):
            verify_receipt(
                canonical(changed),
                self.evidence,
                raw(self.key),
                self.expected(changed["payload"]),
            )

    def test_profile_and_status_cannot_be_promoted(self):
        for changes in (
            {"status": "finalized"},
            {"status": "completed"},
            {"profile": "local", "status": "completed"},
            {"result_sha256": "00" * 32},
            {"extra": True},
        ):
            with self.assertRaises(ValueError):
                issue_reference({**self.claims, **changes}, self.evidence, self.key)
        c = {
            **self.claims,
            "profile": "settlement",
            "status": "blocked",
            "context": "near-bsc:testnet",
        }
        receipt = issue_reference(c, self.evidence, self.key)
        self.assertTrue(receipt.startswith(b'{"payload":'))
        self.assertEqual(
            verify_receipt(receipt, self.evidence, raw(self.key), self.expected(c))[
                "status"
            ],
            "blocked",
        )
        with tempfile.TemporaryDirectory() as d:
            ledger = ReceiptLedger(Path(d) / "ledger.db")
            with self.assertRaises(ValueError):
                ledger.accept_local_completion(
                    receipt, self.evidence, raw(self.key), self.expected(c)
                )


@unittest.skipUnless(
    os.environ.get("X3_PIR_COMPILER"), "requires real Rust compiler example"
)
class LocalReceiptTests(unittest.TestCase):
    setUp = fixture.PirRuntimeTests.setUp
    open = fixture.PirRuntimeTests.open
    admit = fixture.PirRuntimeTests.admit
    execute = fixture.PirRuntimeTests.execute

    def test_success_replay_and_restart(self):
        self.assertEqual(self.execute(), b"ok")
        action = Action("child", "reader", ARTIFACT, "one", 50)
        native = digest(self.r.action_payload(action))
        issuer = Ed25519PrivateKey.generate()
        receipt, evidence = issue_local(self.r, native, issuer)
        self.assertTrue(receipt.startswith(b'{"payload":'))
        p = json.loads(receipt)["payload"]
        expected = {k: p[k] for k in FIELDS}
        self.assertEqual(p["profile"], "local")
        self.assertEqual(p["status"], "completed")
        self.assertEqual(p["source_sha256"], self.grants[1]["source_sha256"])
        self.assertEqual(
            p["policy_sha256"], digest(bytes.fromhex(self.grants[1]["encoded"]))
        )
        self.assertEqual(p["result_sha256"], digest(b"ok"))
        self.assertEqual(p["context"], "runtime:" + self.r.runtime_id)
        self.assertEqual(verify_receipt(receipt, evidence, raw(issuer), expected), p)
        ledger_path = Path(self.tmp.name) / "seen.db"
        ledger = ReceiptLedger(ledger_path)
        inputs = {
            "owner_key": raw(self.owner),
            "agent_key": raw(self.child),
            "source": fixture.source("child", "root", 1, expires=70).encode(),
            "artifact": ARTIFACT,
            "output": b"ok",
        }
        with self.assertRaises(ValueError):
            ledger.accept_local_completion(receipt, evidence, raw(issuer), expected)
        ledger.accept_local_completion(
            receipt, evidence, raw(issuer), expected, **inputs
        )
        again, proof = issue_local(self.open(), native, issuer)
        self.assertEqual(receipt, again)
        with self.assertRaisesRegex(ValueError, "replay"):
            ReceiptLedger(ledger_path).accept_local_completion(
                again, proof, raw(issuer), expected, **inputs
            )
        with sqlite3.connect(self.db) as db:
            db.execute(
                "UPDATE actions SET output=? WHERE digest=?", (b"tampered", native)
            )
        with self.assertRaises(ValueError):
            issue_local(self.r, native, issuer)

    def test_export_preserves_snapshot_while_an_independent_writer_commits(self):
        self.assertEqual(self.execute(), b"ok")
        action = Action("child", "reader", ARTIFACT, "one", 50)
        native = digest(self.r.action_payload(action))
        issuer = Ed25519PrivateKey.generate()
        writer = self.open()
        verify_log = self.r._verify_log

        def verify_and_revoke(reader):
            verify_log(reader)
            old_status = reader.execute("SELECT status FROM agents WHERE id='child'").fetchone()[0]
            writer.revoke("child", "concurrent operator revocation")
            self.assertEqual(reader.execute("SELECT status FROM agents WHERE id='child'").fetchone()[0],
                             old_status)
            self.assertEqual(reader.execute("PRAGMA query_only").fetchone()[0], 1)

        with patch.object(self.r, "_verify_log", side_effect=verify_and_revoke):
            receipt, evidence = issue_local(self.r, native, issuer)
        payload = json.loads(receipt)["payload"]
        self.assertEqual(payload["status"], "completed")
        verify_receipt(receipt, evidence, raw(issuer), {field: payload[field] for field in FIELDS})
        self.assertEqual(writer.snapshot("child")["status"], "revoked")

    def test_noncompletion_is_never_a_completion(self):
        issuer = Ed25519PrivateKey.generate()
        action = Action("child", "reader", ARTIFACT, "interrupted", 50)
        native = digest(self.r.action_payload(action))

        class Interrupted(BaseException):
            pass

        def crash(_):
            raise Interrupted()

        self.adapters["reader"] = Adapter("read", "local", 1, False, crash)
        runtime = self.open()
        with self.assertRaises(Interrupted):
            runtime.execute(action, self.child.sign(runtime.action_payload(action)))
        receipt, evidence = issue_local(self.open(), native, issuer)
        self.assertTrue(receipt.startswith(b'{"payload":'))
        p = json.loads(receipt)["payload"]
        self.assertEqual(p["status"], "pending")
        self.assertIsNone(p["result_sha256"])
        expected = {k: p[k] for k in FIELDS}
        self.assertEqual(verify_receipt(receipt, evidence, raw(issuer), expected), p)
        with self.assertRaises(ValueError):
            ReceiptLedger(Path(self.tmp.name) / "pending.db").accept_local_completion(
                receipt, evidence, raw(issuer), expected
            )

    def test_independent_native_signatures_and_bytes(self):
        self.assertEqual(self.execute(), b"ok")
        native = digest(
            self.r.action_payload(Action("child", "reader", ARTIFACT, "one", 50))
        )
        issuer = Ed25519PrivateKey.generate()
        receipt, evidence = issue_local(self.r, native, issuer)
        p = json.loads(receipt)["payload"]
        expected = {
            "profile": "local",
            "status": "completed",
            "scope": SCOPE,
            "context": "runtime:" + self.r.runtime_id,
            "subject": "child",
            "source_sha256": self.grants[1]["source_sha256"],
            "policy_sha256": digest(bytes.fromhex(self.grants[1]["encoded"])),
            "native_id": native,
            "artifact_sha256": digest(ARTIFACT),
        }
        inputs = {
            "owner_key": raw(self.owner),
            "agent_key": raw(self.child),
            "source": fixture.source("child", "root", 1, expires=70).encode(),
            "artifact": ARTIFACT,
            "output": b"ok",
        }
        self.assertEqual(
            verify_local_provenance(receipt, evidence, raw(issuer), expected, **inputs),
            p,
        )
        for changed in (
            {"source": b"wrong"},
            {"artifact": b"wrong"},
            {"output": b"wrong"},
            {"owner_key": raw(self.root)},
            {"agent_key": raw(self.root)},
        ):
            with self.assertRaises(ValueError):
                verify_local_provenance(
                    receipt, evidence, raw(issuer), expected, **{**inputs, **changed}
                )

    def test_native_signatures_required_even_with_valid_issuer_attestation(self):
        self.execute()
        native = digest(
            self.r.action_payload(Action("child", "reader", ARTIFACT, "one", 50))
        )
        issuer = Ed25519PrivateKey.generate()
        receipt, evidence = issue_local(self.r, native, issuer)
        payload = json.loads(receipt)["payload"]
        expected = {k: payload[k] for k in FIELDS}
        proof = json.loads(evidence)
        inputs = {
            "owner_key": raw(self.owner),
            "agent_key": raw(self.child),
            "source": fixture.source("child", "root", 1, expires=70).encode(),
            "artifact": ARTIFACT,
            "output": b"ok",
        }
        ledger = ReceiptLedger(Path(self.tmp.name) / "native.db")
        for changes in (
            {"agent_signature": "00" * 64},
            {"owner_grant": {**proof["owner_grant"], "signature": "00" * 64}},
            {"request_hex": canonical({"domain": "wrong"}).hex()},
            {"output_sha256": "00" * 32},
            {"terminal_event_sha256": None},
            {"status": "pending"},
            {"unexpected_field": True},
        ):
            with self.subTest(field=next(iter(changes))):
                modified_evidence = canonical({**proof, **changes})
                changed = {**payload, "evidence_sha256": digest(modified_evidence)}
                sig = issuer.sign(DOMAIN + canonical(changed)).hex()
                modified_receipt = canonical({"payload": changed, "signature": sig})
                self.assertEqual(
                    verify_receipt(
                        modified_receipt, modified_evidence, raw(issuer), expected
                    ),
                    changed,
                )
                with self.assertRaises(ValueError):
                    ledger.accept_local_completion(
                        modified_receipt,
                        modified_evidence,
                        raw(issuer),
                        expected,
                        **inputs,
                    )
        self.assertEqual(
            ledger.accept_local_completion(
                receipt, evidence, raw(issuer), expected, **inputs
            ),
            payload,
        )

    def test_failed_callback_has_no_completion_or_output(self):
        def failing_callback(_):
            raise RuntimeError("local callback failed")

        self.adapters["reader"] = Adapter("read", "local", 1, False, failing_callback)
        runtime = self.open()
        action = Action("child", "reader", ARTIFACT, "failure", 50)
        request = runtime.action_payload(action)
        with self.assertRaises(RuntimeError):
            runtime.execute(action, self.child.sign(request))
        issuer = Ed25519PrivateKey.generate()
        receipt, evidence = issue_local(runtime, digest(request), issuer)
        payload = json.loads(receipt)["payload"]
        expected = {k: payload[k] for k in FIELDS}
        self.assertEqual(
            verify_receipt(receipt, evidence, raw(issuer), expected), payload
        )
        self.assertEqual(payload["status"], "failed")
        self.assertIsNone(payload["result_sha256"])
        with self.assertRaisesRegex(ValueError, "local completion"):
            verify_local_provenance(receipt, evidence, raw(issuer), expected)
