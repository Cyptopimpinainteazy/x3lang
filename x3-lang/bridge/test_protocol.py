import importlib.util
import sys
from pathlib import Path
import unittest
from dataclasses import asdict

SPEC = importlib.util.spec_from_file_location("x3_bridge_protocol", Path(__file__).with_name("protocol.py"))
mod = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = mod
SPEC.loader.exec_module(mod)

ALICE_BSC = "0xf311B0CD27d4fB9C1a615df8579004008aEFc228"
LP_BSC = "0x0123456789012345678901234567890123456789"

def plan(**overrides):
    kwargs = dict(secret=bytes(range(32)), initiator_bsc=ALICE_BSC,
                  lp_bsc=LP_BSC, recipient_near="x3lang.near", lp_near="liquidity.testnet",
                  bnb_wei=10**15, near_yocto=10**23,
                  bsc_expiry_sec=1_700_010_800, near_expiry_sec=1_700_003_600,
                  now_sec=1_700_000_000)
    kwargs.update(overrides)
    return mod.create_plan(**kwargs)

class ProtocolTests(unittest.TestCase):
    def test_valid_plan_has_same_32_byte_hashlock_both_chains_without_secret(self):
        p = plan()
        self.assertEqual(len(p.hashlock_hex), 64)
        self.assertEqual(len(p.swap_id_hex), 64)
        self.assertEqual(p.bsc_chain_id, 56)
        self.assertEqual(p.destination_chain, "near")
        self.assertEqual(p.recipient_near, "x3lang.near")
        self.assertNotIn(bytes(range(32)).hex(), str(asdict(p)))
        self.assertNotEqual(p.hashlock_hex, bytes(range(32)).hex())

    def test_rejects_unstaggered_deadlines(self):
        with self.assertRaisesRegex(mod.ProtocolError, "safety window"):
            plan(bsc_expiry_sec=1_700_004_000)

    def test_rejects_invalid_secret_and_malformed_addresses(self):
        with self.assertRaises(mod.ProtocolError):
            plan(secret=b"not 32 bytes")
        with self.assertRaises(mod.ProtocolError):
            plan(lp_bsc="0x1234")

    def test_amounts_and_expiries_must_be_positive(self):
        with self.assertRaises(mod.ProtocolError):
            plan(bnb_wei=0)
        with self.assertRaises(mod.ProtocolError):
            plan(near_yocto=0)
        with self.assertRaises(mod.ProtocolError):
            plan(near_expiry_sec=1_699_999_999)

    def test_chain_domain_separation_prevents_cross_network_swap_id_reuse(self):
        a = plan(near_network="mainnet", bsc_chain_id=56)
        b = plan(near_network="testnet", bsc_chain_id=97)
        self.assertNotEqual(a.swap_id_hex, b.swap_id_hex)
        self.assertEqual(b.near_network, "testnet")
        self.assertEqual(b.bsc_chain_id, 97)

    def test_rejects_dangerously_short_user_selected_safety_window(self):
        with self.assertRaises(mod.ProtocolError):
            plan(min_safety_sec=60)

    def test_no_manual_status_can_establish_funded_settlement(self):
        p = plan()
        result = mod.describe_progress(p, source_claimed=True, near_claim_credited=False,
                                       source_rpc_final=False, near_rpc_final=False)
        self.assertEqual(result["status"], "UNVERIFIED")
        self.assertFalse(result["settled"])

    def test_verified_mismatch_is_a_security_halt(self):
        p = plan()
        result = mod.describe_progress(p, source_claimed=True, near_claim_credited=False,
                                       source_rpc_final=True, near_rpc_final=True)
        self.assertEqual(result["status"], "SECURITY_HALT")
        self.assertFalse(result["settled"])

if __name__ == "__main__":
    unittest.main()
