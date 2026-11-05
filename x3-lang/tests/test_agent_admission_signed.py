"""Adversarial verification for the signed agent-admission boundary."""
import unittest
from dataclasses import replace

from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey

from agent_constitution import AgentManifest
from agent_admission_signed import SignedApproval, approval_payload, verify_signed_admission


class SignedAdmissionTests(unittest.TestCase):
    def setUp(self):
        self.key = Ed25519PrivateKey.generate()
        self.other = Ed25519PrivateKey.generate()
        self.parent = AgentManifest(
            "parent", None, frozenset({"read", "write"}), frozenset({"repo", "api"}),
            100, 100, (), 2,
        )
        self.child = AgentManifest(
            "child", "parent", frozenset({"read"}), frozenset({"repo"}),
            20, 90, ("parent",), 0,
        )

    def approval(self, child=None, parent=None, nonce="one", key=None):
        child = child or self.child
        parent = parent or self.parent
        data = approval_payload(child, parent, nonce=nonce, expires_at=80)
        signer = key or self.key
        return SignedApproval(nonce, 80, signer.sign(data))

    def verify(self, approval=None, child=None, parent=None, trust=None, ids=frozenset({"parent"})):
        return verify_signed_admission(
            child or self.child, parent or self.parent, approval or self.approval(),
            trusted_public_key=trust if trust is not None else self.key.public_key(),
            now=10, existing_ids=ids, parent_budget_available=80, parent_children=0,
        )

    def test_valid(self):
        self.assertTrue(self.verify().allowed)

    def test_wrong_signer(self):
        self.assertIn("signature", self.verify(trust=self.other.public_key()).reasons)

    def test_modified_budget(self):
        self.assertIn("signature", self.verify(child=replace(self.child, budget=30)).reasons)

    def test_modified_capabilities(self):
        self.assertIn("signature", self.verify(child=replace(self.child, capabilities=frozenset({"write"}))).reasons)

    def test_modified_parent(self):
        self.assertIn("signature", self.verify(parent=replace(self.parent, budget=99)).reasons)

    def test_wrong_nonce(self):
        self.assertIn("signature", self.verify(approval=replace(self.approval(), nonce="two")).reasons)

    def test_expired_approval(self):
        a = self.approval()
        decision = verify_signed_admission(
            self.child, self.parent, a, trusted_public_key=self.key.public_key(),
            now=81, existing_ids=frozenset({"parent"}),
            parent_budget_available=80, parent_children=0,
        )
        self.assertIn("approval_expired", decision.reasons)

    def test_duplicate_identity(self):
        self.assertIn("identity", self.verify(ids=frozenset({"parent", "child"})).reasons)

    def test_untrusted_public_key_bytes_rejected(self):
        self.assertIn("signature", self.verify(trust=b"garbage").reasons)

    def test_invalid_signature_length(self):
        self.assertIn("signature", self.verify(approval=replace(self.approval(), signature=b"bad")).reasons)

    def test_other_key_cannot_sign(self):
        self.assertIn("signature", self.verify(approval=self.approval(key=self.other)).reasons)

    def test_malformed_expiry_fails_closed(self):
        self.assertIn(
            "approval_invalid_expiry",
            self.verify(approval=replace(self.approval(), expires_at="bad")).reasons,
        )

    def test_approval_outliving_child_is_rejected(self):
        child = replace(self.child, expires_at=75)
        a = self.approval(child=child)
        self.assertIn("approval_outlives_child", self.verify(child=child, approval=a).reasons)


if __name__ == "__main__":
    unittest.main()
