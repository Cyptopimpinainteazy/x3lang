"""Synthetic negative observations, NOT blockchain evidence; live RPC separate."""
import copy
import unittest

from scripts.hackathon.near_guard_quorum import verify_guard_views, validate_guard_identity

GUARD_SHA = "1d7a8b1ee97b7cdc500db1c19b8f3a2bef06e986576ec428f62f8c3ea1b5a145"
GENESIS = "FWJ9kR6KFWoyMoNjpLXXGHeuiy7tEY6GmoFeCA5yuc6b"
FINAL_HASH = "sPCSCCtVtPE3d4JGvno83zNoK41K5Ns51xspSx3fH1x"
def make_row(rpc, head_height):
    policy={"owner":"x3lang.testnet","nonce":"x3_base_testnet_trade_001",
            "quote_sha256":"a"*64,"revoked":False,"policy":{"program_sha256":"b"*64}}
    return {
        "rpc":rpc, "chain_id":"testnet", "genesis_hash":GENESIS,
        "head_height":head_height,"head_hash":"7ukSxmeJf7qn2Fy7x4LfJLXWbuGaTyYsMGBqAEtq2eij",
        "checkpoint_height":276600020, "checkpoint_hash":FINAL_HASH,
        "checkpoint_code_sha256":GUARD_SHA,"checkpoint_policy":copy.deepcopy(policy),
        "head_code_sha256":GUARD_SHA,"head_policy":copy.deepcopy(policy)
    }
class StrictGuardQuorumTests(unittest.TestCase):
    def setUp(self):
        self.a=make_row("https://test.rpc.fastnear.com",276600020)
        self.b=make_row("https://rpc.testnet.near.org",276600031)
    def go(self):
        return verify_guard_views([self.a,self.b],expected_code_sha=GUARD_SHA,
                                  owner="x3lang.testnet",nonce="x3_base_testnet_trade_001")
    def test_two_independent_matching_finalized_views_allow_readonly_observation(self):
        result=self.go()
        self.assertEqual(result["policy"]["owner"],"x3lang.testnet")
        self.assertEqual(result["checkpoint"]["height"],276600020)
        self.assertFalse(result["trustless_validator_finality_proven"])
    def test_single_rpc_cannot_claim_guard_finality(self):
        with self.assertRaises(ValueError):verify_guard_views([self.a],
            expected_code_sha=GUARD_SHA,owner="x3lang.testnet",nonce="x3_base_testnet_trade_001")
    def test_same_host_is_not_independent(self):
        self.b["rpc"]="https://test.rpc.fastnear.com"
        with self.assertRaises(ValueError):self.go()
    def test_different_canonical_checkpoint_must_refuse(self):
        self.b["checkpoint_hash"]="Another1Canonical1BlockHash1111111111111111111"
        with self.assertRaises(ValueError):self.go()
    def test_wrong_testnet_genesis_must_refuse(self):
        self.b["genesis_hash"]="forged"
        with self.assertRaises(ValueError):self.go()
    def test_lagging_finalized_provider_must_refuse(self):
        self.b["head_height"]=276600090
        with self.assertRaises(ValueError):self.go()
    def test_code_swap_at_newer_finalized_head_must_refuse(self):
        self.b["head_code_sha256"]="0"*64
        with self.assertRaises(ValueError):self.go()
    def test_policy_revoked_at_a_newer_finalized_head_must_refuse(self):
        self.b["head_policy"]["revoked"]=True
        with self.assertRaises(ValueError):self.go()
    def test_two_rpc_policy_disagreement_must_refuse(self):
        self.b["checkpoint_policy"]["quote_sha256"]="0"*64
        with self.assertRaises(ValueError):self.go()
    def test_unreviewed_guard_account_cannot_become_authority(self):
        with self.assertRaises(ValueError):
            validate_guard_identity("attacker-guard.testnet", GUARD_SHA)

    def test_owner_cannot_supply_own_fake_guard_code_hash(self):
        with self.assertRaises(ValueError):
            validate_guard_identity("x3guardlaunch2026.testnet", "a"*64)

    def test_wrong_owner_nonce_must_refuse(self):
        self.b["checkpoint_policy"]["owner"]="attacker.testnet"
        with self.assertRaises(ValueError):self.go()

if __name__=="__main__":
    unittest.main()
