"""Offline denial-path tests; no network or wallet required."""
import importlib.util
from pathlib import Path
import unittest

path = Path(__file__).with_name("tidepool_testnet_preflight.py")
spec = importlib.util.spec_from_file_location("preflight", path)
preflight = importlib.util.module_from_spec(spec)
spec.loader.exec_module(preflight)


class TestPreflight(unittest.TestCase):
    def setUp(self):
        self.origin = {"assetId": preflight.ORIGIN_ASSET, "symbol": "USDC", "decimals": 6, "blockchain": "base"}
        self.destination = {"assetId": preflight.DEST_ASSET, "symbol": "USDC", "decimals": 6, "blockchain": "near"}
        self.health = {"network": "testnet", "verifier": preflight.VERIFIER, "verifierWasmSha256": "f" * 64}
        self.account = {"amount": "1234", "block_height": 23}
        self.guard = {"sha256": preflight.EXPECTED_GUARD_SHA256, "block_height": 24}
        self.verifier = {"sha256": "f" * 64, "block_height": 25}

    def report(self):
        return preflight.build_report(self.health, [self.origin, self.destination], self.account, self.guard, self.verifier)

    def test_normal_is_read_only_and_blocked(self):
        result = self.report()
        self.assertEqual(result["authorization_state"], "BLOCKED_DIFFERENT_QUOTE_MANAGER_KEY_AND_GUARD_ABI")
        self.assertFalse(result["signed"])
        self.assertFalse(result["funds_moved"])
        self.assertFalse(result["settlement_proven"])

    def test_reject_mainnet_provider(self):
        self.health["network"] = "mainnet"
        with self.assertRaisesRegex(RuntimeError, "network/verifier"):
            self.report()

    def test_reject_incorrect_code_hash(self):
        self.guard["sha256"] = "0" * 64
        with self.assertRaisesRegex(RuntimeError, "Guard"):
            self.report()

    def test_reject_verifier_mismatch(self):
        self.verifier["sha256"] = "0" * 64
        with self.assertRaisesRegex(RuntimeError, "verifier hash"):
            self.report()

    def test_reject_chain_mismatch(self):
        self.origin["blockchain"] = "near"
        with self.assertRaisesRegex(RuntimeError, "chain mismatch"):
            self.report()

    def test_reject_precision_mismatch(self):
        self.destination["decimals"] = 18
        with self.assertRaisesRegex(RuntimeError, "precision"):
            self.report()

    def test_reject_missing_pair(self):
        with self.assertRaisesRegex(RuntimeError, "pair absent"):
            preflight.select_pair([self.origin])


if __name__ == "__main__":
    unittest.main()
