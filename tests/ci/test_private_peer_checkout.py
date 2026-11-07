#!/usr/bin/env python3
"""Regression gate for private x3lang -> xxxstar exact-head CI access."""

from __future__ import annotations

import os
from pathlib import Path
import re
import subprocess
import textwrap
import unittest

ROOT = Path(__file__).resolve().parents[2]
WORKFLOW = ROOT / ".github/workflows/x3lang-execution-proof.yml"


class PrivatePeerCheckoutTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.workflow = WORKFLOW.read_text()

    def token_gate(self) -> str:
        result = re.search(
            r"(?ms)^      - name: Require private peer read credential\n"
            r"(?P<block>.*?)(?=^      - name: |^  x3lang-proof:|\Z)",
            self.workflow,
        )
        self.assertIsNotNone(result, "Fail-closed private-peer preflight step absent")
        return result.group("block")

    def run_preflight(self, token: str | None) -> subprocess.CompletedProcess[str]:
        step = self.token_gate()
        match = re.search(
            r"(?m)^        run: \|\n(?P<script>(?:          .*\n|\s*\n)+)",
            step,
        )
        self.assertIsNotNone(match, "Expected inline preflight shell script")
        env = os.environ.copy()
        env.pop("X3LANG_PEER_READ_TOKEN", None)
        if token is not None:
            env["X3LANG_PEER_READ_TOKEN"] = token
        return subprocess.run(
            ["bash", "-e", "-u", "-o", "pipefail", "-c", textwrap.dedent(match.group("script"))],
            check=False,
            capture_output=True,
            text=True,
            env=env,
            timeout=10,
        )

    def test_missing_credential_fails_closed(self) -> None:
        result = self.run_preflight(None)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("X3LANG_PEER_READ_TOKEN", result.stdout + result.stderr)

    def test_placeholder_credential_passes_presence_only(self) -> None:
        fake = "TEST_ONLY_NOT_A_REAL_GITHUB_TOKEN"
        result = self.run_preflight(fake)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertNotIn(fake, result.stdout + result.stderr)

    def test_scoped_secret_only_on_preflight_and_peer_checkout(self) -> None:
        gate = self.token_gate()
        self.assertIn("X3LANG_PEER_READ_TOKEN: ${{ secrets.X3LANG_PEER_READ_TOKEN }}", gate)
        checkout = re.search(
            r"(?ms)^      - name: Checkout matching peer feature branch\n"
            r"(?P<block>.*?)(?=^      - name: |^  x3lang-proof:|\Z)",
            self.workflow,
        )
        self.assertIsNotNone(checkout)
        block = checkout.group("block")
        self.assertIn("repository: Cyptopimpinainteazy/xxxstar", block)
        self.assertIn("token: ${{ secrets.X3LANG_PEER_READ_TOKEN }}", block)
        self.assertIn("persist-credentials: false", block)
        self.assertIn("ref: ${{ github.head_ref || github.ref_name }}", block)
        self.assertLess(self.workflow.index("Require private peer read credential"), self.workflow.index("Checkout matching peer feature branch"))

    def test_no_weakening_or_untrusted_privileged_trigger(self) -> None:
        self.assertIn("  pull_request:\n", self.workflow)
        self.assertNotIn("pull_request_target:", self.workflow)
        self.assertIn("python3 scripts/proof/check_x3lang_mirror.py --peer-repo peer-checkout --base-ref origin/master --peer-base-ref origin/master", self.workflow)
        self.assertNotIn("continue-on-error: true", self.workflow)
        self.assertIn("- 'tests/ci/test_private_peer_checkout.py'", self.workflow)


if __name__ == "__main__":
    unittest.main()
