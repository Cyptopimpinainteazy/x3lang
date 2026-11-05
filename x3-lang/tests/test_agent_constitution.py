import unittest
import sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from agent_constitution import AgentManifest, check_admission


def agent(agent_id="child", parent_id="parent", capabilities=frozenset({"read"}), surfaces=frozenset({"repo"}), budget=20, expires=90, lineage=("parent",), spawn_limit=0):
    return AgentManifest(agent_id, parent_id, capabilities, surfaces, budget, expires, lineage, spawn_limit)


class AdmissionTests(unittest.TestCase):
    def setUp(self):
        self.parent = agent("parent", None, frozenset({"read", "write"}), frozenset({"repo", "api"}), 100, 100, (), 2)

    def check(self, child=None, **kw):
        params = dict(now=10, existing_ids=frozenset({"parent"}), parent_budget_available=80, parent_children=0, approval_verified=True)
        params.update(kw)
        return check_admission(child or agent(), self.parent, **params)

    def test_valid_child(self):
        self.assertTrue(self.check().allowed)

    def test_child_cannot_expand_capabilities(self):
        result = self.check(agent(capabilities=frozenset({"read", "admin"})))
        self.assertFalse(result.allowed)
        self.assertIn("capabilities", result.reasons)

    def test_child_cannot_expand_surfaces(self):
        self.assertIn("surfaces", self.check(agent(surfaces=frozenset({"repo", "secrets"}))).reasons)

    def test_child_cannot_exceed_remaining_budget(self):
        self.assertIn("budget", self.check(agent(budget=81)).reasons)

    def test_child_cannot_outlive_parent(self):
        self.assertIn("expiry", self.check(agent(expires=101)).reasons)

    def test_parent_lineage_required(self):
        self.assertIn("lineage", self.check(agent(lineage=())).reasons)

    def test_duplicate_identity_rejected(self):
        self.assertIn("identity", self.check(agent("parent")).reasons)

    def test_parent_spawn_limit_enforced(self):
        self.assertIn("spawn_limit", self.check(parent_children=2).reasons)

    def test_missing_verified_approval_denied(self):
        self.assertIn("approval", self.check(approval_verified=False).reasons)

    def test_expired_parent_denied(self):
        self.assertIn("parent_expired", self.check(now=101).reasons)

    def test_no_parent_only_as_explicit_root(self):
        result = check_admission(agent("root", None, lineage=()), None, now=10, existing_ids=frozenset(), parent_budget_available=0, parent_children=0, approval_verified=True)
        self.assertIn("root_requires_separate_genesis", result.reasons)

    def test_invalid_negative_budget_denied(self):
        self.assertIn("budget", self.check(agent(budget=-1)).reasons)


if __name__ == "__main__":
    unittest.main()
