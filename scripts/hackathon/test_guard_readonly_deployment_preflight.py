"""Non-chain arithmetic and rejection tests; not evidence of real deployment."""
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from guard_readonly_deployment_preflight import PreflightFailure, minimum_wasm_stake, run


class GuardReadOnlyPreflightTests(unittest.TestCase):
    def test_protocol_priced_code_stake_exceeds_two_near(self):
        # These are the genuine locally observed WASM length and protocol price.
        self.assertEqual(minimum_wasm_stake(545640, 10**19), 545640 * 10**19)
        self.assertGreater(minimum_wasm_stake(545640, 10**19), 2 * 10**24)

    def test_refuses_zero_or_unbounded_code(self):
        for length in (0, -1, 4 * 1024 * 1024 + 1):
            with self.assertRaises(PreflightFailure):
                minimum_wasm_stake(length, 10**19)

    def test_refuses_zero_storage_price(self):
        with self.assertRaises(PreflightFailure):
            minimum_wasm_stake(545640, 0)

    def test_refuses_mainnet_name_before_any_rpc(self):
        with self.assertRaisesRegex(PreflightFailure, "testnet"):
            run("x3guard.near", Path("non-existent-file"))

    def test_refuses_missing_artifact_before_any_rpc(self):
        with self.assertRaisesRegex(PreflightFailure, "missing"):
            run("x3lang.testnet", Path("non-existent-file"))


if __name__ == "__main__":
    unittest.main()
