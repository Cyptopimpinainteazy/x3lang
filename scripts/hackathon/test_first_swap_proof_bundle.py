"""Offline integrity checks over REAL archived chain observations and signed quote.

These checks do not replace re-querying live RPC (two_chain_settlement_proof.py).
They ensure the immutable evidence bundle cannot silently drift or overclaim.
"""
from __future__ import annotations

from copy import deepcopy
from datetime import datetime
import hashlib
import json
from pathlib import Path
import sys
import unittest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "x3-lang"))
sys.path.insert(0, str(Path(__file__).resolve().parent))

from base_sepolia_guarded_admission import (
    AdmissionError, SOURCE, TOKEN, TOKEN_CODE_SHA256,
    check_quote, parse_time,
)
from base_sepolia_x3_intent import compile_testnet_intent

QUOTE = ROOT / "evidence/tidepool-testnet/first-swap-signed-live-quote-20261107.json"
PROOF = ROOT / "evidence/tidepool-testnet/first-swap-strict-two-chain-proof-20261107.json"
BASELINE = ROOT / "evidence/tidepool-testnet/pretrade-finalized-baseline-20261107.json"

QUOTE_SHA256 = "7718da27680bbb35cbf199caaae06ee77987dd5daf60f2dfd1ca175a08da7535"
PROOF_SHA256 = "9aa6d251b7e3c8f8cb6fee57e466034d3f747a84b3792233fe36cd97570bbbcc"
BASELINE_SHA256 = "69a835814805876acd2eafb1ca0390081e6bf4d4fca6bcc4c5347dbb609c4120"
EXPECTED_HASH = "0x46c21dc305a9271c4e74c6d0734fb50f847a1e53d3cad87976133d66fd6728d9"
EXPECTED_NEAR = "EBEBxJNgV9U5tBFBLj18ZzukgArqwXH5rBsBStFyCm1d"


class FirstSwapEvidenceTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.quote_file = json.loads(QUOTE.read_text())
        cls.quote = cls.quote_file["response"]
        cls.proof = json.loads(PROOF.read_text())
        cls.base = json.loads(BASELINE.read_text())

    def test_immutable_public_chain_artifact_bytes(self):
        for path, expected in [(QUOTE, QUOTE_SHA256), (PROOF, PROOF_SHA256),
                               (BASELINE, BASELINE_SHA256)]:
            with self.subTest(path=path.name):
                self.assertEqual(hashlib.sha256(path.read_bytes()).hexdigest(), expected)

    def test_genuine_tidepool_maker_signature_and_exact_output(self):
        issued = parse_time(self.quote["timestamp"])
        signed = check_quote(self.quote, issued)
        self.assertEqual(self.proof["quote_sha256"], signed["quote_sha256"])
        self.assertEqual(self.proof["quote_sha256"],
                         "131f314c2532965f2678e79f3f7cc87cfe8238a549da1405e802f4c90db1034c")
        self.assertEqual(signed["amount_out"], 997400)
        self.assertEqual(signed["minimum_out"], 987426)
        self.assertEqual(self.quote["quote"]["amountIn"], "1000000")
        self.assertEqual(self.quote["quoteRequest"]["recipient"], "x3lang.testnet")

    def test_modified_signed_quote_is_cryptographically_rejected(self):
        altered = deepcopy(self.quote)
        altered["quoteRequest"]["recipient"] = "attacker.testnet"
        with self.assertRaisesRegex(AdmissionError, "MAKER_SIGNATURE_INVALID"):
            check_quote(altered, parse_time(self.quote["timestamp"]))

    def test_real_x3_source_hash_and_one_use_nonce(self):
        compiled = compile_testnet_intent()
        self.assertEqual(compiled["program_sha256"], self.proof["program_sha256"])
        self.assertEqual(compiled["nonce"], "x3_base_testnet_trade_001")
        self.assertEqual(self.proof["guard_account"], "x3guardlaunch2026.testnet")
        self.assertEqual(self.proof["guard_wasm_sha256"],
                         "1d7a8b1ee97b7cdc500db1c19b8f3a2bef06e986576ec428f62f8c3ea1b5a145")

    def test_historical_guard_approval_precedes_real_base_transaction(self):
        historical = self.proof["historical_guard_approval_evidence"]
        before_source_ns = historical["approval_timestamp_nanoseconds"]
        observations = self.proof["base_evidence"]["observations"]
        inclusion_sec = observations[0]["inclusion_timestamp_unix_s"]
        self.assertLess(before_source_ns, inclusion_sec * 1_000_000_000)
        self.assertEqual(historical["approval_near_block_height"], 276595150)
        self.assertEqual(historical["owner"], "x3lang.testnet")
        self.assertEqual(historical["nonce"], "x3_base_testnet_trade_001")
        self.assertEqual(historical["historical_policy"]["quote_sha256"],
                         self.proof["quote_sha256"])
        self.assertEqual(historical["historical_policy"]["policy"]["program_sha256"],
                         self.proof["program_sha256"])
        self.assertEqual(historical["historical_code_sha256"], self.proof["guard_wasm_sha256"])
        self.assertLessEqual(inclusion_sec, int(parse_time(self.quote["quote"]["deadline"]).timestamp()))

    def test_two_independent_base_sepolia_finalized_receipts(self):
        observations = self.proof["base_evidence"]["observations"]
        self.assertEqual(len(observations), 2)
        self.assertNotEqual(observations[0]["rpc"], observations[1]["rpc"])
        for source in observations:
            with self.subTest(source=source["rpc"]):
                self.assertEqual(source["tx_hash"], EXPECTED_HASH)
                self.assertEqual(source["chain_id"], 84532)
                self.assertEqual(source["from"], SOURCE.lower())
                self.assertEqual(source["token"], TOKEN.lower())
                self.assertEqual(source["deposit_address"],
                                 self.quote["quote"]["depositAddress"].lower())
                self.assertEqual(source["units"], 1000000)
                self.assertEqual(source["block_number"], 55557170)
                self.assertGreaterEqual(source["finalized_block_height"], source["block_number"])
                self.assertGreaterEqual(source["confirmations"], 12)
                self.assertEqual(source["token_code_sha256"], TOKEN_CODE_SHA256)
        self.assertEqual(observations[0]["block_hash"], observations[1]["block_hash"])
        self.assertEqual(observations[0]["event_log_index"], observations[1]["event_log_index"])

    def test_successful_final_near_transfer_and_canonical_balance_delta(self):
        proof = self.proof["near_receipt_evidence"]
        balance = self.proof["near_balance_evidence"]
        self.assertEqual(proof["classification"], "near_finalized_transfer_receipt")
        self.assertEqual(proof["transaction_hash"], EXPECTED_NEAR)
        self.assertEqual(proof["delivered_units"], 997400)
        self.assertEqual(proof["receiver"], "x3lang.testnet")
        self.assertEqual(proof["token"], "usdc.tidepool-sandbox.testnet")
        self.assertEqual(proof["receipt_count"], 6)
        self.assertEqual(balance["before"]["units"], 100000000)
        self.assertEqual(balance["after"]["units"], 100997400)
        self.assertEqual(balance["net_increase_units"], 997400)
        self.assertLess(balance["before"]["block_height"], proof["transfer_receipt"]["block_height"])
        self.assertLessEqual(proof["transfer_receipt"]["block_height"], balance["after"]["block_height"])
        self.assertEqual(self.base["near"]["finalized_balance"]["block_hash"],
                         balance["before"]["block_hash"])

    def test_local_guard_security_artifact_cannot_impersonate_tidepool_deployment(self):
        path = ROOT / "evidence/tidepool-testnet/local-guard-adversarial-review-20261107.json"
        self.assertEqual(hashlib.sha256(path.read_bytes()).hexdigest(),
                         "2298a4e0f69a398f783e295d1fbcb4d9096a3c7f2047e77289b61ccccf68e3ac")
        row = json.loads(path.read_text())
        self.assertEqual(row["mutation_summary"]["counts"],
                         {"KILLED": 6, "SURVIVED": 0, "INVALID": 0, "TIMEOUT": 0})
        self.assertEqual(row["sandbox_execution"],
                         "REAL_LOCAL_OFFLINE_NEAR_SANDBOX_WASM_TRANSACTIONS_EXIT_0")
        self.assertIn("different from deployed Tidepool Guard", row["subject"])
        self.assertIn("not a test of the separate deployed Tidepool Guard", row["claims_excluded"])

    def test_strict_classification_cannot_claim_atomicity_or_mainnet(self):
        self.assertEqual(self.proof["classification"],
                         "independent_testnet_two_chain_transfer_evidence_NOT_atomicity_or_native_x3vm_proof")
        limitations = " ".join(self.proof["claim_limits"])
        self.assertIn("Provider is custodial testnet relay", limitations)
        self.assertIn("No cryptographic atomic HTLC proof", limitations)
        self.assertIn("No mainnet readiness", limitations)
        self.assertNotIn("trustless_atomic_swap", self.proof["classification"].lower())


if __name__ == "__main__":
    unittest.main()
