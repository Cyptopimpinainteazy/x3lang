#!/usr/bin/env python3
"""Static regression guards for known false-positive Sandbox evidence paths.

These checks inspect Rust source. They are NOT Rust execution, a protocol proof,
a Sandbox run, or a substitute for the typed-rejection tests in the finish plan.
Run from any directory; only this repository's harness is read.
"""
from pathlib import Path
import re
import unittest

ROOT = Path(__file__).resolve().parents[2]
HARNESS = ROOT / "x3-lang/near/agent-vault-sandbox-tests/src/main.rs"


class VaultRefusalSourceGuard(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.source = HARNESS.read_text(encoding="utf-8")

    def test_wildcard_sdk_error_is_not_success(self):
        self.assertNotRegex(self.source, r"Err\s*\(\s*_\s*\)\s*=>\s*true\b")

    def test_generic_sdk_error_has_no_permission_success_label(self):
        self.assertNotIn('"sdk_or_rpc_permission_refusal"', self.source)

    def test_arbitrary_failed_transaction_is_not_permission_proof(self):
        self.assertNotIn('"failed_chain_transaction"', self.source)

    def test_unclassified_direct_attempt_has_no_refusal_success_marker(self):
        self.assertNotIn("X3_VAULT_SANDBOX_SCOPED_DIRECT_TRANSFER_REFUSED", self.source)

    def test_contract_failure_observations_are_not_reason_verified_claims(self):
        for marker in (
            "X3_VAULT_SANDBOX_WRONG_RECIPIENT_AND_CAP_REFUSED",
            "X3_VAULT_SANDBOX_NONCE_REPLAY_AND_REVOCATION_REFUSED",
            "active_mandate_denied=true",
        ):
            with self.subTest(marker=marker):
                self.assertNotIn(marker, self.source)

    def test_completion_is_gated_on_no_inconclusive_observations(self):
        gate = re.search(r"ensure!\(\s*inconclusive\.is_empty\(\)", self.source)
        self.assertIsNotNone(gate, "missing fail-closed completion gate")
        completion = self.source.index('println!("X3_VAULT_SANDBOX_COMPLETE')
        self.assertLess(gate.start(), completion)

    def test_both_permission_attempts_record_inconclusive_results(self):
        self.assertIn('inconclusive.push("direct_transfer_permission")', self.source)
        self.assertIn('inconclusive.push("agent_recovery_permission")', self.source)

    def test_source_marks_the_required_followup_not_a_completed_fix(self):
        self.assertIn("Structured raw-node rejection evidence is still required", self.source)


if __name__ == "__main__":
    unittest.main(verbosity=2)
