"""Independent fixed fixture vectors shared with Rust policy_encoding tests."""
import hashlib
import unittest

from x3_policy_ir import Profile, bind


class PolicyEncodingVectors(unittest.TestCase):
    def test_binding_vectors(self):
        expected = {
            Profile.EVM_ACTION: "6872573330be773c6d26bad352d5acf1cd4cc9461657e0d36aa4f73271f0e789",
            Profile.AGENT_ADMISSION: "475eb8d792028f685739c5786b8f27a3406a1edb1fb08651c6dedb46ea98fe82",
            Profile.NEAR_BSC_WINDOW: "e1dc240594c666538ef1a4e5593dbc2388adce51bd868b769a8c93690af28b3e",
        }
        for profile, digest in expected.items():
            with self.subTest(profile=profile):
                self.assertEqual(bind(profile, b"fixture policy bytes",
                                      "sandbox:fixture", bytes(range(32))).digest, digest)

    def test_typed_grant_reference(self):
        # Independent fixture encoder, NOT a general Python grant implementation.
        def lp(value):
            raw = value.encode("utf-8")
            return len(raw).to_bytes(2, "big") + raw

        raw = (b"X3/PIR/GRANT/1\x00" + lp("parent") + b"\x00"
               + lp("sandbox:fixture") + b"\x01" + (84532).to_bytes(8, "big")
               + bytes([1])*20 + bytes([2])*20 + bytes([3])*4 + bytes([4])*32
               + b"\x01" + lp("evm") + lp("base-sepolia") + lp("token") + lp("USD")
               + bytes([6]) + (100).to_bytes(16, "big") + (1000).to_bytes(8, "big")
               + bytes([2, 2, 1, 2]))
        self.assertEqual(hashlib.sha256(raw).hexdigest(),
                         "301bde48f2d974c394b66241448544e3428400498bd2375d0ae9c25a03951762")
