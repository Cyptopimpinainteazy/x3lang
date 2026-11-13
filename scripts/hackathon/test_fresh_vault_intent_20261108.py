"""Fresh .x3 nonce candidate: actual X3Lang Python parser and typechecker only.

This does not sign, deploy, fund, approve Guard, or prove unused nonce on-chain.
"""
from __future__ import annotations
from pathlib import Path
from tempfile import TemporaryDirectory
import hashlib
import unittest
import sys
ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts/hackathon"))
from base_sepolia_x3_intent import compile_testnet_intent, ProgramRefused, NONCE as LEGACY_NONCE

CANDIDATE = ROOT / "docs/hackathon/candidates/base_vault_fresh_20261108.x3"
NONCE = "x3_base_vault_20261108_0cf8a80f52f96a1629f04a23"
SHA256 = "73eaa27705c86872c3f34feeab634a2e2b7ea1f6b913390a1663a652813f4f03"

class FreshVaultCandidateTest(unittest.TestCase):
    def test_real_frontend_typechecks_and_uses_fresh_nonce(self):
        result = compile_testnet_intent(CANDIDATE, expected_nonce=NONCE)
        self.assertEqual(result["classification"], "x3lang_python_frontend_testnet_policy_not_vm_bytecode")
        self.assertEqual(result["nonce"], NONCE)
        self.assertNotEqual(NONCE, LEGACY_NONCE)
        self.assertEqual(result["program_sha256"], SHA256)
        self.assertEqual(hashlib.sha256(CANDIDATE.read_bytes()).hexdigest(), SHA256)
        self.assertEqual(result["origin_amount"], 1_000_000)
        self.assertEqual(result["destination_account"], "x3lang.testnet")
        self.assertEqual(result["minimum_out"], 980_000)
        self.assertEqual(result["slippage_max_bps"], 100)
        self.assertEqual(result["timeout_minutes"], 30)
        self.assertEqual(result["refund_to"].lower(), result["source_account"].lower())

    def test_default_legacy_nonce_cannot_compile_new_candidate(self):
        with self.assertRaisesRegex(ProgramRefused, "TESTNET_NONCE_MISMATCH"):
            compile_testnet_intent(CANDIDATE)

    def test_new_nonce_is_not_reusable_on_legacy_source(self):
        with self.assertRaisesRegex(ProgramRefused, "TESTNET_NONCE_MISMATCH"):
            compile_testnet_intent(expected_nonce=NONCE)

    def test_mutated_sensitive_terms_are_refused_by_frontend(self):
        source = CANDIDATE.read_text()
        for prior, altered in [
            ("receiver x3lang.testnet", "receiver attacker.testnet"),
            ("amount 1000000", "amount 2000000"),
            ("min_receive 980000", "min_receive 1"),
            ("slippage <= 1%", "slippage <= 100%"),
            ("refund BaseSepolia.USDC to 0x5aF0c1BF95B3A158f6Fc48e3DED35903b2fa0721", "refund BaseSepolia.USDC to 0x0000000000000000000000000000000000000001"),
            ("require nonce unused " + NONCE, "require nonce unused "+LEGACY_NONCE),
        ]:
            self.assertIn(prior, source)
            with self.subTest(prior=prior), TemporaryDirectory() as tmp:
                test_path = Path(tmp) / "altered.x3"
                test_path.write_text(source.replace(prior, altered))
                with self.assertRaises(ProgramRefused):
                    compile_testnet_intent(test_path, expected_nonce=NONCE)

    def test_missing_or_malformed_expected_nonce_is_refused(self):
        for value in ("", "x3_base_testnet_bad nonce", "MAINNET", None):
            with self.subTest(value=value), self.assertRaises(ProgramRefused):
                compile_testnet_intent(CANDIDATE, expected_nonce=value)

if __name__ == "__main__":
    unittest.main()
