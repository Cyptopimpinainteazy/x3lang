"""Real subprocess checks for the mutation runner; no test-process mocking."""
import unittest
from pathlib import Path

import mutation_runner as runner


ROOT = Path(__file__).resolve().parents[2]


class MutationRunnerTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.source = (ROOT / "x3-lang" / "near_mandate.py").read_text()
        cls.recipient = next(m for m in runner.get_mutations() if m.id == "recipient_bypass")

    def test_unique_expected_source_replacement(self):
        changed = runner.apply_mutation(self.source, self.recipient)
        self.assertNotEqual(changed, self.source)
        self.assertIn('record("mandate_recipient", True,', changed)
        self.assertEqual(changed.count('record("mandate_recipient", True,'), 1)

    def test_source_drift_refused(self):
        with self.assertRaises(ValueError):
            runner.apply_mutation(self.source.replace(self.recipient.original, "False"), self.recipient)

    def test_ambiguous_mutation_refused(self):
        with self.assertRaises(ValueError):
            runner.apply_mutation(self.source + "\n# " + self.recipient.original, self.recipient)

    def test_real_assertion_failure_counts_killed(self):
        changed = runner.apply_mutation(self.source, self.recipient)
        result = runner.run_tests_in_copy(ROOT, changed, self.recipient, timeout_seconds=30)
        self.assertEqual(result.status, "KILLED", result.test_stdout[-1000:])
        self.assertTrue(result.failed_tests)

    def test_real_import_error_is_not_killed(self):
        # The copy contains invalid Python instead of a functioning mutant.
        # An import/collection error must be INVALID, never a security success.
        result = runner.run_tests_in_copy(
            ROOT, "this is invalid Python syntax\n", self.recipient, timeout_seconds=30
        )
        self.assertEqual(result.status, "INVALID", result.test_stdout[-1000:])
        self.assertEqual(result.killed_by_tests, [])


if __name__ == "__main__":
    unittest.main()
