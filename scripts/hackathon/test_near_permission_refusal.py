"""Synthetic response tests only: no node, signing, or on-chain evidence."""
from __future__ import annotations

import copy
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

MODULE_PATH = Path(__file__).with_name("near_permission_refusal.py")


def response(reason="RequiresFullAccess", request_id="direct-transfer-1"):
    return {
        "jsonrpc": "2.0", "id": request_id,
        "error": {
            "name": "HANDLER_ERROR", "code": -32000, "message": "Server error",
            "cause": {"name": "INVALID_TRANSACTION", "info": {}},
            "data": {"TxExecutionError": {"InvalidTxError": {"InvalidAccessKeyError": reason}}},
        },
    }


def encoded(value):
    return json.dumps(value, separators=(",", ":")).encode()


class PermissionRefusalTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.verifier = None
        if MODULE_PATH.is_file():
            spec = importlib.util.spec_from_file_location("near_permission_refusal", MODULE_PATH)
            cls.verifier = importlib.util.module_from_spec(spec)
            sys.modules[spec.name] = cls.verifier
            spec.loader.exec_module(cls.verifier)

    def setUp(self):
        self.assertIsNotNone(self.verifier, "raw-response permission verifier is not implemented")
        self.expected = self.verifier.Expectation("direct-transfer-1", "direct_transfer")

    def reject(self, value, expected=None):
        raw = value if isinstance(value, bytes) else encoded(value)
        with self.assertRaises(self.verifier.RefusalEvidenceError):
            self.verifier.classify(raw, expected or self.expected)

    def test_direct_transfer_requires_exact_full_access_reason(self):
        result = self.verifier.classify(encoded(response()), self.expected)
        self.assertEqual(result["observed_reason"], "RequiresFullAccess")
        self.assertEqual(result["request_id"], self.expected.request_id)
        self.assertEqual(result["classification"], "EXPECTED_PERMISSION_ERROR_IN_RPC_RESPONSE")
        self.assertFalse(result["live_execution_verified"])
        self.assertFalse(result["signed_transaction_verified"])
        self.assertFalse(result["cryptographic_proof"])
        self.assertIsNone(result["included_receipt_id"])

    def test_wrong_method_must_match_requested_method(self):
        expected = self.verifier.Expectation("direct-transfer-1", "wrong_method", method_name="recover_available")
        raw = response({"MethodNameMismatch": {"method_name": "recover_available"}})
        self.assertEqual(self.verifier.classify(encoded(raw), expected)["observed_reason"], raw["error"]["data"]["TxExecutionError"]["InvalidTxError"]["InvalidAccessKeyError"])
        raw["error"]["data"]["TxExecutionError"]["InvalidTxError"]["InvalidAccessKeyError"]["MethodNameMismatch"]["method_name"] = "request_payment"
        self.reject(raw, expected)

    def test_wrong_receiver_must_match_both_receivers(self):
        expected = self.verifier.Expectation("direct-transfer-1", "wrong_receiver", tx_receiver="attacker.testnet", ak_receiver="vault.testnet")
        raw = response({"ReceiverMismatch": {"tx_receiver": "attacker.testnet", "ak_receiver": "vault.testnet"}})
        self.assertEqual(self.verifier.classify(encoded(raw), expected)["case"], "wrong_receiver")
        for field in ("tx_receiver", "ak_receiver"):
            changed = copy.deepcopy(raw)
            changed["error"]["data"]["TxExecutionError"]["InvalidTxError"]["InvalidAccessKeyError"]["ReceiverMismatch"][field] = "other.testnet"
            self.reject(changed, expected)

    def test_attached_deposit_requires_its_own_reason(self):
        expected = self.verifier.Expectation("direct-transfer-1", "attached_deposit")
        self.assertEqual(self.verifier.classify(encoded(response("DepositWithFunctionCall")), expected)["case"], "attached_deposit")
        self.reject(response(), expected)

    def test_generic_invalid_transaction_is_inconclusive(self):
        raw = response()
        raw["error"]["data"] = {"TxExecutionError": {"InvalidTxError": "InvalidTransaction"}}
        self.reject(raw)

    def test_nonce_expiry_signature_and_funding_errors_never_count(self):
        for reason in ("Expired", "InvalidSignature", {"InvalidNonce": {"tx_nonce": 1, "ak_nonce": 2}}, {"NotEnoughBalance": {"balance": "0", "cost": "1"}}):
            with self.subTest(reason=reason):
                raw = response()
                raw["error"]["data"]["TxExecutionError"]["InvalidTxError"] = reason
                self.reject(raw)

    def test_other_access_key_errors_do_not_count(self):
        for reason in ("DepositWithFunctionCall", {"AccessKeyNotFound": {"account_id": "agent.testnet", "public_key": "ed25519:example"}}, {"NotEnoughAllowance": {"allowance": "0", "cost": "1"}}, {"MethodNameMismatch": {"method_name": "request_payment"}}):
            with self.subTest(reason=reason):
                self.reject(response(reason))

    def test_transport_timeout_and_non_json_fail(self):
        for raw in (b"", b"timeout", b"connection refused", b"<html>502 gateway</html>", b"\xff"):
            with self.subTest(raw=raw):
                self.reject(raw)

    def test_rpc_timeout_internal_and_validation_errors_fail(self):
        for name in ("TIMEOUT_ERROR", "UNKNOWN_TRANSACTION", "INTERNAL_ERROR", "REQUEST_VALIDATION_ERROR"):
            with self.subTest(name=name):
                raw = response()
                raw["error"]["cause"]["name"] = name
                self.reject(raw)

    def test_pending_and_success_results_do_not_count(self):
        for value in ({"final_execution_status": "NONE"}, {"status": {"SuccessValue": ""}}, {"status": {"Failure": "RequiresFullAccess"}}):
            with self.subTest(value=value):
                self.reject({"jsonrpc": "2.0", "id": self.expected.request_id, "result": value})

    def test_result_and_error_together_fail(self):
        raw = response()
        raw["result"] = None
        self.reject(raw)

    def test_wrong_missing_or_wrong_type_request_id_fail(self):
        for value in (None, "other-attempt", 1, True, {}, []):
            with self.subTest(value=value):
                self.reject(response(request_id=value))
        raw = response()
        del raw["id"]
        self.reject(raw)

    def test_protocol_version_and_server_error_code_are_checked(self):
        for value in (None, "1.0", 2.0):
            raw = response()
            raw["jsonrpc"] = value
            self.reject(raw)
        for value in (None, "-32000", -32000.0, True, -32603):
            raw = response()
            raw["error"]["code"] = value
            self.reject(raw)

    def test_error_name_is_checked(self):
        raw = response()
        raw["error"]["name"] = "INTERNAL_ERROR"
        self.reject(raw)

    def test_missing_or_nonempty_cause_info_fails(self):
        for value in (None, [], {"reason": "RequiresFullAccess"}):
            raw = response()
            raw["error"]["cause"]["info"] = value
            self.reject(raw)
        raw = response()
        del raw["error"]["cause"]["info"]
        self.reject(raw)

    def test_text_and_logs_cannot_impersonate_structured_error(self):
        raw = response()
        raw["error"]["data"] = "RequiresFullAccess"
        raw["logs"] = ["InvalidAccessKeyError RequiresFullAccess"]
        self.reject(raw)

    def test_duplicate_keys_at_every_matched_level_fail(self):
        raw = encoded(response())
        for old, new in ((b'"id":"direct-transfer-1"', b'"id":"wrong","id":"direct-transfer-1"'), (b'"code":-32000', b'"code":0,"code":-32000'), (b'"InvalidAccessKeyError":"RequiresFullAccess"', b'"InvalidAccessKeyError":"Expired","InvalidAccessKeyError":"RequiresFullAccess"')):
            with self.subTest(old=old):
                self.reject(raw.replace(old, new))

    def test_nonfinite_numbers_fail(self):
        for constant in (b"NaN", b"Infinity", b"-Infinity", b"1e999", b"-1e999"):
            self.reject(encoded(response()).replace(b'"code":-32000', b'"unused":' + constant + b',"code":-32000'))

    def test_ambiguous_variant_objects_fail(self):
        raw = response()
        raw["error"]["data"]["TxExecutionError"]["InvalidTxError"]["Other"] = "ignored"
        self.reject(raw)
        raw = response()
        raw["error"]["data"]["TxExecutionError"]["ActionError"] = {}
        self.reject(raw)

    def test_wrong_variant_shape_and_extra_variant_fields_fail(self):
        for reason in ({"RequiresFullAccess": {}}, ["RequiresFullAccess"], None, True, {"MethodNameMismatch": {"method_name": "recover_available", "extra": "x"}}):
            self.reject(response(reason))

    def test_missing_structural_levels_fail(self):
        for path in (("error", "data"), ("error", "cause"), ("error", "data", "TxExecutionError"), ("error", "data", "TxExecutionError", "InvalidTxError")):
            raw = response()
            parent = raw
            for part in path[:-1]:
                parent = parent[part]
            del parent[path[-1]]
            self.reject(raw)

    def test_oversized_and_deep_payloads_fail(self):
        self.reject(b" " * (self.verifier.MAX_RESPONSE_BYTES + 1))
        self.reject(b"[" * 1500 + b"0" + b"]" * 1500)

    def test_response_digest_is_for_original_bytes(self):
        import hashlib
        a = encoded(response())
        b = json.dumps(response(), indent=2).encode()
        result = self.verifier.classify(a, self.expected)
        self.assertEqual(result["response_sha256"], hashlib.sha256(a).hexdigest())
        self.assertNotEqual(result["response_sha256"], self.verifier.classify(b, self.expected)["response_sha256"])

    def test_invalid_expectation_cases_and_ids_fail(self):
        for values in (("x", "unknown"), ("", "direct_transfer"), (True, "direct_transfer"), ("x" * 129, "direct_transfer")):
            with self.subTest(values=values), self.assertRaises(self.verifier.RefusalEvidenceError):
                self.verifier.Expectation(*values)

    def test_missing_or_irrelevant_expectation_fields_fail(self):
        for case, kwargs in (("wrong_method", {}), ("wrong_receiver", {}), ("direct_transfer", {"method_name": "x"}), ("wrong_method", {"method_name": "x", "ak_receiver": "vault.testnet"}), ("wrong_receiver", {"tx_receiver": "vault.testnet", "ak_receiver": "vault.testnet"}), ("attached_deposit", {"method_name": "x"})):
            with self.subTest(case=case, kwargs=kwargs), self.assertRaises(self.verifier.RefusalEvidenceError):
                self.verifier.Expectation("x", case, **kwargs)

    def test_cli_is_read_only_and_explicitly_not_chain_proof(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            observed = root / "response.json"
            expected = root / "expected.json"
            observed.write_bytes(encoded(response()))
            expected.write_text(json.dumps({"request_id": "direct-transfer-1", "case": "direct_transfer"}))
            args = [sys.executable, str(MODULE_PATH), "--response", str(observed), "--expectation", str(expected)]
            run = subprocess.run(args, capture_output=True, text=True, timeout=10)
            self.assertEqual(run.returncode, 0, run.stderr)
            result = json.loads(run.stdout)
            self.assertEqual(result["classification"], "EXPECTED_PERMISSION_ERROR_IN_RPC_RESPONSE")
            self.assertFalse(result["live_execution_verified"])
            self.assertEqual(sorted(p.name for p in root.iterdir()), ["expected.json", "response.json"])
            observed.write_text("connection failed")
            failed = subprocess.run(args, capture_output=True, text=True, timeout=10)
            self.assertEqual(failed.returncode, 2)
            self.assertEqual(failed.stdout, "")
            self.assertIn("INCONCLUSIVE", failed.stderr)

    def test_cli_unknown_expectation_field_is_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            observed = root / "response.json"
            expected = root / "expected.json"
            observed.write_bytes(encoded(response()))
            expected.write_text(json.dumps({"request_id": "direct-transfer-1", "case": "direct_transfer", "ignore_errors": True}))
            run = subprocess.run([sys.executable, str(MODULE_PATH), "--response", str(observed), "--expectation", str(expected)], capture_output=True, text=True, timeout=10)
            self.assertEqual(run.returncode, 2)
            self.assertEqual(run.stdout, "")


if __name__ == "__main__":
    unittest.main()
