"""Fail-closed schema tests for judge_local_showcase; no network or wallet needed."""
import importlib.util
import json
import subprocess
from pathlib import Path
import sys
import unittest
from unittest.mock import patch
import contextlib
import io

SCRIPT = Path(__file__).with_name("judge_local_showcase.py")
spec = importlib.util.spec_from_file_location("x3_judge_local_showcase", SCRIPT)
showcase = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = showcase
spec.loader.exec_module(showcase)


class ShowcaseSchemaTests(unittest.TestCase):
    def test_grok_requires_control_and_all_three_refusals(self):
        report = {"ok": True, "passed": 3, "total": 3,
                  "control": {"allowed": True}, "cases": [{"case": name, "pass": True} for name in (
                      "permission_object_lifts_limits", "morse_encoded_transfer", "full_incident_chain")]}
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
                  "checks": {name: True for name in (
                      "original_payment_settles", "alternate_txid_cannot_trigger_second_withdrawal",
                      "exactly_one_economic_debit", "changed_txid_linked_to_same_intent",
                      "missing_reserves_freeze", "authorized_control_allowed")}}
        showcase.verify_report("mtgox", report)
        report["checks"]["missing_reserves_freeze"] = False
        with self.assertRaises(ValueError):
            showcase.verify_report("mtgox", report)

    def test_duplicate_grok_cases_do_not_prove_three_attacks(self):
        report = {"ok": True, "passed": 3, "total": 3,
                  "control": {"allowed": True},
                  "cases": [{"case": "permission_object_lifts_limits", "pass": True}] * 3}
        with self.assertRaises(ValueError):
            showcase.verify_report("grok", report)

    def test_unrelated_accounting_checks_cannot_substitute_for_invariants(self):
        report = {"ok": True, "passed": 6, "total": 6,
                  "checks": {str(i): True for i in range(6)}}
        with self.assertRaises(ValueError):
            showcase.verify_report("mtgox", report)

    def test_incomplete_stage_makes_cli_exit_nonzero_without_success_output(self):
        incomplete = {"ok": True, "passed": 6, "total": 6, "checks": {}}
        completed = subprocess.CompletedProcess([], 0, json.dumps(incomplete), "")
        with patch.object(showcase.subprocess, "run", return_value=completed):
            with self.assertRaises(ValueError):
                showcase.run_stage("mtgox", "replay_mtgox_accounting.py", ["--json"])
        stdout, stderr = io.StringIO(), io.StringIO()
        with patch.object(showcase, "showcase", side_effect=ValueError("incomplete evidence")):
            with patch.object(sys, "argv", [str(SCRIPT)]):
                with contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
                    self.assertEqual(showcase.main(), 1)
        self.assertEqual(stdout.getvalue(), "")
        self.assertIn("FAIL: incomplete evidence", stderr.getvalue())

    def test_unrecognized_stage_fails_closed(self):
        with self.assertRaises(ValueError):
            showcase.verify_report("mainnet", {"ok": True})


if __name__ == "__main__":
    unittest.main()
