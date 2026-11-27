"""Independent replay of a regression proposal against the demo's real compiler policy."""
import importlib.util
from pathlib import Path
import unittest

from agent_runtime import digest

SCRIPT = Path(__file__).resolve().parents[2] / "scripts/hackathon/agent_law_local_demo.py"
SOURCE = (Path(__file__).resolve().parents[1] / "examples/near_intents_agent_payment.x3").read_bytes()


def load_demo():
    spec = importlib.util.spec_from_file_location("agent_law_local_demo", SCRIPT)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class RegressionReplayTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.demo = load_demo()

    def proposal(self, source, **case):
        fields = {"source_hex": source.hex(), "source_sha256": digest(source), "adapter": "compile-intent",
                  "policy_id": self.demo.demo_policy().policy_id(), "expected": "reject",
                  "expected_error": "policy violation: recipient", "action_digest": "0" * 64}
        fields.update(case)
        return {"format": "x3.regression-proposal", "version": 1, "case": fields}

    def test_rejected_source_replays_as_proposed(self):
        attack = SOURCE.replace(b"receiver x3-demo.near", b"receiver attacker.near")
        verdict = self.demo.replay_regression(self.proposal(attack))
        self.assertTrue(verdict["matches"])
        self.assertEqual(verdict["observed"]["error"], "policy violation: recipient")

    def test_source_the_compiler_accepts_does_not_match(self):
        verdict = self.demo.replay_regression(self.proposal(SOURCE))
        self.assertFalse(verdict["matches"])
        self.assertFalse(verdict["observed"]["rejected"])

    def test_different_rejection_reason_does_not_match(self):
        overspend = SOURCE.replace(b"amount 25000000", b"amount 26000000")
        verdict = self.demo.replay_regression(self.proposal(overspend))
        self.assertTrue(verdict["observed"]["rejected"])
        self.assertFalse(verdict["matches"])

    def test_non_policy_refusal_is_reported_but_does_not_match(self):
        verdict = self.demo.replay_regression(self.proposal(bytes([255])))
        self.assertEqual(verdict["observed"]["error_type"], "UnicodeDecodeError")
        self.assertFalse(verdict["matches"])

    def test_other_policy_revision_or_altered_source_refused(self):
        attack = SOURCE.replace(b"receiver x3-demo.near", b"receiver attacker.near")
        with self.assertRaisesRegex(ValueError, "policy revision"):
            self.demo.replay_regression(self.proposal(attack, policy_id="1" * 64))
        with self.assertRaisesRegex(ValueError, "digest"):
            self.demo.replay_regression(self.proposal(attack, source_sha256=digest(SOURCE)))
        with self.assertRaisesRegex(ValueError, "unsupported"):
            self.demo.replay_regression({**self.proposal(attack), "version": 2})


if __name__ == "__main__":
    unittest.main()
