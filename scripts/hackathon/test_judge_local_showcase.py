"""Fail-closed schema tests for judge_local_showcase; no network or wallet needed."""
import importlib.util
from pathlib import Path
import sys
import unittest

SCRIPT = Path(__file__).with_name("judge_local_showcase.py")
spec = importlib.util.spec_from_file_location("x3_judge_local_showcase", SCRIPT)
showcase = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = showcase
spec.loader.exec_module(showcase)


class ShowcaseSchemaTests(unittest.TestCase):
    def test_grok_requires_control_and_all_three_refusals(self):
        report = {"ok": True, "passed": 3, "total": 3,
                  "control": {"allowed": True}, "cases": [{"pass": True}] * 3}
        self.assertEqual(showcase.verify_report("grok", report)["level"], "LOCAL")
        report["control"]["allowed"] = False
        with self.assertRaises(ValueError):
            showcase.verify_report("grok", report)

    def test_agent_retirement_must_survive_restart(self):
        report = {"replay_blocked": True, "retired_after_policy_violation": True,
                  "retirement_survives_restart": True,
                  "compliant_plan": {"kind": "local-compiled-plan-no-payment"}}
        showcase.verify_report("agent_law", report)
        report["retirement_survives_restart"] = False
        with self.assertRaises(ValueError):
            showcase.verify_report("agent_law", report)

    def test_mtgox_requires_six_actual_passes(self):
        report = {"ok": True, "passed": 6, "total": 6,
                  "checks": {str(i): True for i in range(6)}}
        showcase.verify_report("mtgox", report)
        report["checks"]["3"] = False
        with self.assertRaises(ValueError):
            showcase.verify_report("mtgox", report)

    def test_unrecognized_stage_fails_closed(self):
        with self.assertRaises(ValueError):
            showcase.verify_report("mainnet", {"ok": True})


if __name__ == "__main__":
    unittest.main()
