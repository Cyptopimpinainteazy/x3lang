#!/usr/bin/env python3
"""Static guards for exact Sandbox refusal classification.

These checks inspect the Rust harness. They are not a substitute for the real
Sandbox job, which exercises the compiled contract and raw node responses.
"""
from pathlib import Path
import hashlib
import json
import unittest

ROOT = Path(__file__).resolve().parents[2]
HARNESS = ROOT / "x3-lang/near/agent-vault-sandbox-tests/src/main.rs"
EVIDENCE = ROOT / "evidence/agent-vault-local/typed-refusal-sandbox-20261108.json"


class VaultRefusalSourceGuard(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.source = HARNESS.read_text(encoding="utf-8")

    def test_wildcard_sdk_error_is_not_success(self):
        self.assertNotIn("Err(_) => true", self.source)

    def test_generic_error_labels_are_absent(self):
        for marker in (
            "sdk_or_rpc_permission_refusal",
            "failed_chain_transaction",
            "non_success_outcome_unclassified",
            "sdk_or_rpc_error_unclassified",
        ):
            with self.subTest(marker=marker):
                self.assertNotIn(marker, self.source)

    def test_raw_response_requires_typed_access_key_error(self):
        self.assertIn(
            "/error/data/TxExecutionError/InvalidTxError/InvalidAccessKeyError",
            self.source,
        )
        self.assertIn('&json!("RequiresFullAccess")', self.source)

    def test_agent_recovery_requires_method_name_mismatch(self):
        self.assertIn(
            '&json!({"MethodNameMismatch":{"method_name":"recover_available"}})',
            self.source,
        )

    def test_contract_failures_require_typed_action_execution_reason(self):
        self.assertIn("TxExecutionError::ActionError", self.source)
        self.assertIn("ActionErrorKind::FunctionCallError", self.source)
        self.assertIn("FunctionCallError::ExecutionError", self.source)
        self.assertIn('strip_prefix("Smart contract panicked: ")', self.source)

    def test_all_contract_refusals_match_the_intended_stable_reason(self):
        for reason in (
            "X3_VAULT_DENIED: recipient not allowlisted",
            "X3_VAULT_DENIED: per-payment cap exceeded",
            "X3_VAULT_DENIED: nonce already used",
            "X3_VAULT_DENIED: revoke or expire mandate before recovery",
            "X3_VAULT_DENIED: owner revoked mandate",
        ):
            with self.subTest(reason=reason):
                self.assertIn(reason, self.source)

    def test_success_markers_follow_verified_checks(self):
        completion = self.source.index('println!("X3_VAULT_SANDBOX_COMPLETE')
        for marker in (
            "X3_VAULT_SANDBOX_SCOPED_DIRECT_TRANSFER_REFUSED",
            "X3_VAULT_SANDBOX_RECIPIENT_AND_CAP_REFUSED",
            "X3_VAULT_SANDBOX_REPLAY_AND_REVOCATION_REFUSED",
            "X3_VAULT_SANDBOX_AGENT_RECOVERY_REFUSED",
            "X3_VAULT_SANDBOX_OWNER_RECOVERY_SUCCESS",
        ):
            with self.subTest(marker=marker):
                self.assertLess(self.source.index(marker), completion)

    def test_balance_and_terminal_state_assertions_remain(self):
        for assertion in (
            "direct transfer changed attacker balance",
            "denied requests changed escrow",
            "successful payment reverted due to later denial",
            "vault account balance did not fund owner recovery",
        ):
            with self.subTest(assertion=assertion):
                self.assertIn(assertion, self.source)

    def test_evidence_bundle_log_hash_and_final_marker(self):
        evidence = json.loads(EVIDENCE.read_text(encoding="utf-8"))
        log_path = ROOT / evidence["sandbox_log"]
        log = log_path.read_bytes()
        self.assertEqual(hashlib.sha256(log).hexdigest(), evidence["sandbox_log_sha256"])
        self.assertIn(b"X3_VAULT_SANDBOX_COMPLETE", log)
        self.assertEqual(evidence["execution"]["sandbox_exit_code"], 0)


if __name__ == "__main__":
    unittest.main(verbosity=2)
