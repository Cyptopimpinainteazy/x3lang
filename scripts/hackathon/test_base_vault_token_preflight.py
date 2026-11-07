"""Pure validation regression tests; fake observations are NOT chain evidence."""
import copy
import unittest
from unittest.mock import patch
from scripts.hackathon.base_vault_token_preflight import attest_pair, checked_block, HIST_HEIGHT

PIN = "db77a21a647b5973bac5728bf9f6977b56933006bf2f0730a14d109d33072c83"
HASH = "0x1a95c4b60f2bc909969f52d24fd73727b237427a1ba60763214eebd03ea86157"
ZERO = "0x" + "0" * 64

def observation(rpc, head=55_600_000):
    return {
        "rpc": rpc,
        "chain_id": 84532,
        "historical_height": 55_557_819,
        "historical_hash": HASH,
        "historical_code_sha256": PIN,
        "finalized_height": head,
        "finalized_code_sha256": PIN,
        "impl_slot_historical": ZERO,
        "impl_slot_finalized": ZERO,
        "beacon_slot_historical": ZERO,
        "beacon_slot_finalized": ZERO,
    }

class TokenAttestationTests(unittest.TestCase):
    def setUp(self):
        self.left = observation("https://base-sepolia.gateway.tenderly.co", 55_600_001)
        self.right = observation("https://base-sepolia-rpc.publicnode.com", 55_600_000)

    def test_canonical_finalized_checkpoint_without_exact_head_alignment(self):
        report = attest_pair(self.left, self.right, "0x" + "7" * 64, "0x" + "7" * 64)
        self.assertEqual(report["common_finalized_height"], 55_600_000)
        self.assertTrue(report["standard_eip1967_slots_zero"])

    def test_fake_hex_checkpoint_is_rejected_even_if_providers_agree(self):
        forged_hash = "0x" + "z" * 64
        with self.assertRaises(ValueError):
            attest_pair(self.left, self.right, forged_hash, forged_hash)

    def test_provider_cannot_return_another_block_number_for_pin(self):
        with patch("scripts.hackathon.base_vault_token_preflight.strict_rpc",
                   return_value={"hash": HASH, "number": hex(HIST_HEIGHT + 1)}):
            with self.assertRaises(ValueError):
                checked_block(self.left["rpc"], HIST_HEIGHT)

    def test_wrong_chain_hard_fails(self):
        bad = copy.deepcopy(self.right);bad["chain_id"] = 1
        with self.assertRaises(ValueError):
            attest_pair(self.left, bad, "0x" + "7" * 64, "0x" + "7" * 64)

    def test_replaced_code_at_latest_finalized_hard_fails(self):
        bad = copy.deepcopy(self.right);bad["finalized_code_sha256"] = "0" * 64
        with self.assertRaises(ValueError):
            attest_pair(self.left, bad, "0x" + "7" * 64, "0x" + "7" * 64)

    def test_implementation_proxy_slot_hard_fails(self):
        bad = copy.deepcopy(self.right);bad["impl_slot_finalized"] = "0x" + "0" * 63 + "1"
        with self.assertRaises(ValueError):
            attest_pair(self.left, bad, "0x" + "7" * 64, "0x" + "7" * 64)

    def test_mismatched_common_canonical_finalized_hash_hard_fails(self):
        with self.assertRaises(ValueError):
            attest_pair(self.left, self.right, "0x" + "7" * 64, "0x" + "8" * 64)

    def test_rpc_cannot_be_same_provider_twice(self):
        duplicate = copy.deepcopy(self.right);duplicate["rpc"] = "https://base-sepolia.gateway.tenderly.co"
        with self.assertRaises(ValueError):
            attest_pair(self.left, duplicate, "0x" + "7" * 64, "0x" + "7" * 64)

if __name__ == "__main__":
    unittest.main()
