"""Synthetic wire-capture regression tests; no signing, node or chain proof."""
from __future__ import annotations

import base64
import copy
import hashlib
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import unittest

SCRIPT = Path(__file__).with_name("near_permission_refusal.py")
spec = importlib.util.spec_from_file_location("capture_classifier", SCRIPT)
assert spec and spec.loader
classifier = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = classifier
spec.loader.exec_module(classifier)


def encoded(raw: bytes) -> str:
    return base64.b64encode(raw).decode("ascii")


def dumped(value: object) -> bytes:
    return json.dumps(value, separators=(",", ":")).encode()


def fixture() -> dict:
    # Deliberately not a transaction: the classifier must NOT authenticate it.
    signed = b"synthetic signed-byte fixture, not chain evidence"
    request_id = "x3-vault-" + hashlib.sha256(signed).hexdigest() + "-0"
    request = {"jsonrpc": "2.0", "id": request_id, "method": "send_tx",
               "params": {"signed_tx_base64": encoded(signed), "wait_until": "FINAL"}}
    response = {"jsonrpc": "2.0", "id": request_id, "error": {
        "code": -32000, "name": "HANDLER_ERROR",
        "cause": {"name": "INVALID_TRANSACTION", "info": {}},
        "data": {"TxExecutionError": {"InvalidTxError": {
            "InvalidAccessKeyError": "RequiresFullAccess"}}}}}
    return {"request_base64": encoded(dumped(request)),
            "response_base64": encoded(dumped(response)),
            "expectation": {"request_id": request_id, "case": "direct_transfer"}}


class WireCaptureTests(unittest.TestCase):
    def setUp(self) -> None:
        self.assertTrue(callable(getattr(classifier, "classify_capture", None)),
                        "live raw-byte capture is not connected to the strict classifier")
        self.capture = fixture()

    def run_capture(self, capture: dict | None = None) -> dict:
        return classifier.classify_capture(dumped(self.capture if capture is None else capture))

    def assert_rejected(self, capture: dict) -> None:
        with self.assertRaises(classifier.RefusalEvidenceError):
            self.run_capture(capture)

    def edit(self, field: str, transform) -> None:
        obj = json.loads(base64.b64decode(self.capture[field]))
        transform(obj)
        self.capture[field] = encoded(dumped(obj))

    def test_original_bytes_and_transaction_digest_are_preserved(self):
        result = self.run_capture()
        request_bytes = base64.b64decode(self.capture["request_base64"])
        tx_bytes = base64.b64decode(json.loads(request_bytes)["params"]["signed_tx_base64"])
        self.assertEqual(result["request_sha256"], hashlib.sha256(request_bytes).hexdigest())
        self.assertEqual(result["signed_transaction_sha256"], hashlib.sha256(tx_bytes).hexdigest())
        self.assertEqual(result["original_request_base64"], self.capture["request_base64"])
        self.assertEqual(result["original_response_base64"], self.capture["response_base64"])
        self.assertEqual(result["rpc_response"], json.loads(base64.b64decode(self.capture["response_base64"])))
        self.assertTrue(result["request_payload_bound"])

    def test_bytes_do_not_become_cryptographic_or_live_proof(self):
        result = self.run_capture()
        for field in ("live_execution_verified", "signed_transaction_verified", "cryptographic_proof"):
            self.assertIs(result[field], False)
        self.assertIsNone(result["included_receipt_id"])

    def test_changed_signed_bytes_with_old_request_id_fail(self):
        self.edit("request_base64", lambda r: r["params"].update(signed_tx_base64=encoded(b"different bytes")))
        self.assert_rejected(self.capture)

    def test_different_response_request_id_fails(self):
        self.edit("response_base64", lambda r: r.update(id="other-operation"))
        self.assert_rejected(self.capture)

    def test_different_request_id_from_expectation_fails(self):
        self.edit("request_base64", lambda r: r.update(id="other-operation"))
        self.assert_rejected(self.capture)

    def test_request_id_counter_must_be_canonical_u64(self):
        for suffix in ("", "-1", "+1", "00", "1.0", "x", "18446744073709551616", "١"):
            with self.subTest(suffix=suffix):
                self.capture = fixture()
                identity = self.capture["expectation"]["request_id"].rsplit("-", 1)[0] + "-" + suffix
                self.edit("request_base64", lambda r: r.update(id=identity))
                self.edit("response_base64", lambda r: r.update(id=identity))
                self.capture["expectation"]["request_id"] = identity
                self.assert_rejected(self.capture)

    def test_request_method_and_final_status_cannot_change(self):
        for key, value in (("method", "query"), ("jsonrpc", "1.0")):
            self.capture = fixture()
            self.edit("request_base64", lambda r: r.update({key: value}))
            self.assert_rejected(self.capture)
        self.capture = fixture()
        self.edit("request_base64", lambda r: r["params"].update(wait_until="NONE"))
        self.assert_rejected(self.capture)

    def test_extra_and_missing_request_fields_fail(self):
        for field in ("id", "params", "method", "jsonrpc"):
            self.capture = fixture()
            self.edit("request_base64", lambda r: r.pop(field))
            self.assert_rejected(self.capture)
        self.capture = fixture()
        self.edit("request_base64", lambda r: r.update(extra=True))
        self.assert_rejected(self.capture)
        self.capture = fixture()
        self.edit("request_base64", lambda r: r["params"].update(extra=True))
        self.assert_rejected(self.capture)

    def test_capture_schema_is_exact(self):
        for key in tuple(self.capture):
            changed = copy.deepcopy(self.capture)
            del changed[key]
            self.assert_rejected(changed)
        changed = copy.deepcopy(self.capture)
        changed["permission_proven"] = True
        self.assert_rejected(changed)

    def test_duplicate_keys_are_rejected_before_conversion(self):
        for field in ("request_base64", "response_base64"):
            changed = copy.deepcopy(self.capture)
            raw = base64.b64decode(changed[field])
            changed[field] = encoded(b'{"id":"forged",' + raw[1:])
            self.assert_rejected(changed)
        raw = dumped(self.capture)
        with self.assertRaises(classifier.RefusalEvidenceError):
            classifier.classify_capture(b'{"expectation":{},' + raw[1:])

    def test_base64_is_strict_nonempty_and_canonical(self):
        for value in ("", "%%%", "a", "Zg===", "Z g==", "Zh==", 7, None):
            for field in ("request_base64", "response_base64"):
                changed = copy.deepcopy(self.capture)
                changed[field] = value
                self.assert_rejected(changed)
        for value in ("", "%%%", "Zh==", 7):
            self.capture = fixture()
            self.edit("request_base64", lambda r: r["params"].update(signed_tx_base64=value))
            self.assert_rejected(self.capture)

    def test_original_response_limits_still_apply(self):
        for raw in (b"x" * (classifier.MAX_RESPONSE_BYTES + 1), b'{"error": NaN}', b"[" * 2000):
            changed = copy.deepcopy(self.capture)
            changed["response_base64"] = encoded(raw)
            self.assert_rejected(changed)
        with self.assertRaises(classifier.RefusalEvidenceError):
            classifier.classify_capture(b" " * (classifier.MAX_CAPTURE_BYTES + 1))

    def test_transaction_size_is_bounded(self):
        self.edit("request_base64", lambda r: r["params"].update(
            signed_tx_base64=encoded(b"x" * (classifier.MAX_SIGNED_TRANSACTION_BYTES + 1))))
        self.assert_rejected(self.capture)

    def test_wrong_permission_reason_cannot_pass(self):
        self.edit("response_base64", lambda r: r["error"]["data"]["TxExecutionError"]["InvalidTxError"].update(
            InvalidAccessKeyError="DepositWithFunctionCall"))
        self.assert_rejected(self.capture)

    def test_wrong_method_details_are_checked_in_capture_mode(self):
        self.capture["expectation"].update(case="wrong_method", method_name="recover_available")
        self.edit("response_base64", lambda r: r["error"]["data"]["TxExecutionError"]["InvalidTxError"].update(
            InvalidAccessKeyError={"MethodNameMismatch": {"method_name": "recover_available"}}))
        self.assertEqual(self.run_capture()["case"], "wrong_method")
        self.capture["expectation"]["method_name"] = "revoke"
        self.assert_rejected(self.capture)

    def test_pending_result_and_transport_errors_fail(self):
        for response in ({"result": {"final_execution_status": "NONE"}},
                         {"error": "timeout"}, {"error": {"code": -32000}}):
            changed = copy.deepcopy(self.capture)
            changed["response_base64"] = encoded(dumped(response))
            self.assert_rejected(changed)

    def test_unknown_expectation_field_fails_closed(self):
        self.capture["expectation"]["ignore_errors"] = True
        self.assert_rejected(self.capture)

    def test_cli_capture_mode_has_no_file_or_network_requirement(self):
        result = subprocess.run([sys.executable, str(SCRIPT), "--capture-stdin"],
                                input=dumped(self.capture), capture_output=True, timeout=5)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(json.loads(result.stdout)["classification"], "EXPECTED_PERMISSION_ERROR_IN_RPC_RESPONSE")

    def test_cli_rejects_mixed_input_modes_and_trailing_data(self):
        for args, raw in ((["--capture-stdin", "--response", "anything"], dumped(self.capture)),
                          (["--capture-stdin"], dumped(self.capture) + b"{}")):
            result = subprocess.run([sys.executable, str(SCRIPT), *args], input=raw,
                                    capture_output=True, timeout=5)
            self.assertEqual(result.returncode, 2)
            self.assertEqual(result.stdout, b"")


if __name__ == "__main__":
    unittest.main(verbosity=2)
