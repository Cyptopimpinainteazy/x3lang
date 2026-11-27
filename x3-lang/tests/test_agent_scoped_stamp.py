"""No-network scoped X3 Stamp tests: signature, authority, expiry, revocation."""
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from cryptography.hazmat.primitives import serialization
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from agent_scoped_stamp import StampRequest, issue_scoped_stamp, verify_scoped_stamp, SCORE_IDS


class StampTests(unittest.TestCase):
    def setUp(self):
        self.key = Ed25519PrivateKey.generate()
        self.public = self.key.public_key().public_bytes(
            serialization.Encoding.Raw, serialization.PublicFormat.Raw)
        self.req = StampRequest("test-agent", "compile-intent", b"safe-test-source",
                                "a" * 64, 100, 200, "nonce-1",
                                ("source_digest", "capability_scope"))
        self.stamp = issue_scoped_stamp(self.req, self.key)
        self.kw = dict(trusted_issuer=self.public, agent_id="test-agent",
                       action="compile-intent", artifact=b"safe-test-source",
                       policy_sha256="a" * 64, now=150)

    def test_scoped_valid(self):
        self.assertTrue(verify_scoped_stamp(self.stamp, **self.kw))
        self.assertFalse(self.stamp["payload"]["full_ten_commandments_enforced"])

    def test_all_ten_listed_but_not_attested(self):
        self.assertEqual(len(SCORE_IDS), 10)
        self.assertNotIn("full_ten_commandments_enforced", self.stamp["payload"]["passed_checks"])

    def test_modified_source_refused(self):
        with self.assertRaises(ValueError):
            verify_scoped_stamp(self.stamp, **{**self.kw, "artifact": b"tampered"})

    def test_wrong_agent_action_or_policy_refused(self):
        for key, bad in [("agent_id", "attacker"), ("action", "deploy"), ("policy_sha256", "b" * 64)]:
            with self.subTest(key=key), self.assertRaises(ValueError):
                verify_scoped_stamp(self.stamp, **{**self.kw, key: bad})

    def test_expiry_and_preissuance_refused(self):
        for timestamp in (99, 200, 300):
            with self.subTest(now=timestamp), self.assertRaises(ValueError):
                verify_scoped_stamp(self.stamp, **{**self.kw, "now": timestamp})

    def test_revocation_refused(self):
        with self.assertRaises(ValueError):
            verify_scoped_stamp(self.stamp, **{**self.kw, "revoked_nonces": {"nonce-1"}})

    def test_wrong_issuer_refused(self):
        other = Ed25519PrivateKey.generate().public_key().public_bytes(
            serialization.Encoding.Raw, serialization.PublicFormat.Raw)
        with self.assertRaises(ValueError):
            verify_scoped_stamp(self.stamp, **{**self.kw, "trusted_issuer": other})

    def test_tampered_claim_refused(self):
        self.stamp["payload"]["full_ten_commandments_enforced"] = True
        with self.assertRaises(ValueError):
            verify_scoped_stamp(self.stamp, **self.kw)

    def test_forged_signature_refused(self):
        self.stamp["signature"] = "00" * 64
        with self.assertRaises(ValueError):
            verify_scoped_stamp(self.stamp, **self.kw)

    def test_unknown_checks_fail_closed(self):
        request = StampRequest("test-agent", "compile-intent", b"safe-test-source",
                               "a" * 64, 100, 200, "nonce-1", ("FakeSafety",))
        with self.assertRaises(ValueError):
            issue_scoped_stamp(request, self.key)

    def test_missing_required_policy_check_fails_closed(self):
        with self.assertRaises(ValueError):
            verify_scoped_stamp(self.stamp, **{**self.kw,
                "required_checks": {"major_action_jury"}})
        self.assertTrue(verify_scoped_stamp(self.stamp, **{**self.kw,
            "required_checks": {"source_digest", "capability_scope"}}))

    def test_missing_check_set_fails_closed(self):
        self.stamp["payload"]["passed_checks"] = []
        with self.assertRaises(ValueError):
            verify_scoped_stamp(self.stamp, **self.kw)


if __name__ == "__main__":
    unittest.main()
