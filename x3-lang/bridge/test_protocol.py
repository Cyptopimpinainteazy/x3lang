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
BSC_ESCROW = "0x1111111111111111111111111111111111111111"
NEAR_VAULT = "x3-vault.testnet"
NONCE = "11" * 32

def plan(**overrides):
    kwargs = dict(secret=bytes(range(32)), initiator_bsc=ALICE_BSC,
                  lp_bsc=LP_BSC, recipient_near="x3lang.testnet", lp_near="liquidity.testnet",
                  bsc_escrow=BSC_ESCROW, near_vault=NEAR_VAULT, nonce_hex=NONCE,
                  bnb_wei=10**15, near_yocto=10**23,
                  bsc_expiry_sec=1_700_010_800,
                  near_funded_before_sec=1_700_002_400,
                  near_expiry_sec=1_700_003_600,
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
        self.assertEqual(p.recipient_near, "x3lang.testnet")
        self.assertEqual(p.bsc_escrow, BSC_ESCROW)
        self.assertEqual(p.near_vault, NEAR_VAULT)
        self.assertEqual(p.nonce_hex, NONCE)
        self.assertEqual(p.near_funded_before_sec, 1_700_002_400)
        self.assertEqual(p.near_funded_before_ns, 1_700_002_400 * 1_000_000_000)
        self.assertEqual(p.near_expiry_sec, 1_700_003_600)
        self.assertEqual(p.near_expiry_ns, 1_700_003_600 * 1_000_000_000)
        self.assertEqual(
            p.bsc_deadline_arguments(),
            {
                "nearFundedBefore": 1_700_002_400,
                "nearExpiresAt": 1_700_003_600,
                "expiresAt": 1_700_010_800,
            },
        )
        self.assertNotIn(bytes(range(32)).hex(), str(asdict(p)))
        self.assertNotEqual(p.hashlock_hex, bytes(range(32)).hex())

    def test_swap_identity_binds_contracts_nonce_and_both_deadline_schedules(self):
        baseline = plan()
        variants = [
            plan(bsc_escrow="0x2222222222222222222222222222222222222222"),
            plan(near_vault="another-vault.testnet"),
            plan(nonce_hex="22" * 32),
            plan(near_funded_before_sec=1_700_002_500),
            plan(near_expiry_sec=1_700_003_700),
            plan(bsc_expiry_sec=1_700_010_900),
        ]
        self.assertEqual(len({baseline.swap_id_hex, *(p.swap_id_hex for p in variants)}), 7)

    def test_rejects_unstaggered_deadlines(self):
        with self.assertRaisesRegex(mod.ProtocolError, "safety window"):
            plan(bsc_expiry_sec=1_700_004_000)

    def test_rejects_invalid_secret_and_malformed_addresses(self):
        with self.assertRaises(mod.ProtocolError):
            plan(secret=b"not 32 bytes")
        with self.assertRaises(mod.ProtocolError):
            plan(lp_bsc="0x1234")
        with self.assertRaises(mod.ProtocolError):
            plan(bsc_escrow="0x" + "00" * 20)
        with self.assertRaises(mod.ProtocolError):
            plan(nonce_hex="abc")
        with self.assertRaises(mod.ProtocolError):
            plan(initiator_bsc=LP_BSC)
        with self.assertRaises(mod.ProtocolError):
            plan(recipient_near="liquidity.testnet")
        with self.assertRaises(mod.ProtocolError):
            plan(recipient_near="x3-vault.testnet")

    def test_amounts_and_expiries_must_be_positive(self):
        with self.assertRaises(mod.ProtocolError):
            plan(bnb_wei=0)
        with self.assertRaises(mod.ProtocolError):
            plan(near_yocto=0)
        with self.assertRaises(mod.ProtocolError):
            plan(near_expiry_sec=1_699_999_999)
        with self.assertRaises(mod.ProtocolError):
            plan(near_funded_before_sec=1_700_003_600)
        with self.assertRaises(mod.ProtocolError):
            plan(now_sec=2**64)

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
