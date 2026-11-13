#!/usr/bin/env python3
"""Regression gate for explicitly requested, read-only private peer audits."""

from __future__ import annotations

import os
from pathlib import Path
import re
import subprocess
import textwrap
import unittest

ROOT = Path(__file__).resolve().parents[2]
WORKFLOW = ROOT / ".github/workflows/x3lang-peer-parity.yml"


class PrivatePeerCheckoutTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.workflow = WORKFLOW.read_text()

    def key_gate(self) -> str:
        result = re.search(
            r"(?ms)^      - name: Require private peer read credential\n"
            r"(?P<block>.*?)(?=^      - name: |^  x3lang-proof:|\Z)",
            self.workflow,
        )
        self.assertIsNotNone(result, "Fail-closed private-peer preflight step absent")
        return result.group("block")

    def run_preflight(self, token: str | None) -> subprocess.CompletedProcess[str]:
        step = self.key_gate()
        match = re.search(
            r"(?m)^        run: \|\n(?P<script>(?:          .*\n|\s*\n)+)",
            step,
        )
        self.assertIsNotNone(match, "Expected inline preflight shell script")
        env = os.environ.copy()
        env.pop("X3LANG_PEER_READ_SSH_KEY", None)
        if token is not None:
            env["X3LANG_PEER_READ_SSH_KEY"] = token
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
        self.assertIn("X3LANG_PEER_READ_SSH_KEY", result.stdout + result.stderr)

    def test_placeholder_credential_passes_presence_only(self) -> None:
        fake = "TEST_ONLY_NOT_A_REAL_SSH_KEY"
        result = self.run_preflight(fake)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertNotIn(fake, result.stdout + result.stderr)

    def test_scoped_secret_only_on_preflight_and_peer_checkout(self) -> None:
        gate = self.key_gate()
        self.assertIn("X3LANG_PEER_READ_SSH_KEY: ${{ secrets.X3LANG_PEER_READ_SSH_KEY }}", gate)
        checkout = re.search(
            r"(?ms)^      - name: Checkout pinned peer revision\n"
            r"(?P<block>.*?)(?=^      - name: |^  x3lang-proof:|\Z)",
            self.workflow,
        )
        self.assertIsNotNone(checkout)
        block = checkout.group("block")
        self.assertIn("repository: Cyptopimpinainteazy/xxxstar", block)
        self.assertIn("ssh-key: ${{ secrets.X3LANG_PEER_READ_SSH_KEY }}", block)
        self.assertIn("persist-credentials: false", block)
        self.assertIn("ref: ${{ inputs.peer_commit }}", block)
        self.assertLess(self.workflow.index("Require private peer read credential"), self.workflow.index("Checkout pinned peer revision"))

    def test_manual_audit_retains_fail_closed_checks_and_unprivileged_trigger(self) -> None:
        self.assertIn("  workflow_dispatch:\n", self.workflow)
        self.assertNotIn("  pull_request:\n", self.workflow)
        self.assertNotIn("pull_request_target:", self.workflow)
        self.assertIn("python3 scripts/proof/check_x3lang_mirror.py --peer-repo peer-checkout --base-ref origin/master --peer-base-ref origin/master", self.workflow)
        self.assertNotIn("continue-on-error: true", self.workflow)
        self.assertIn("contents: read", self.workflow)
        self.assertIn("Validate pinned peer revision", self.workflow)

    def test_peer_revision_requires_full_sha_without_shell_expansion(self) -> None:
        block = re.search(
            r"(?ms)^      - name: Validate pinned peer revision\n(.*?)(?=^      - name: )",
            self.workflow,
        ).group(1)
        script = re.search(r"(?m)^        run: \|\n(?P<script>(?:          .*\n|\s*\n)+)", block)
        self.assertIsNotNone(script)
        for value, expected in [("a" * 40, 0), ("master", 2), ("", 2), ("a" * 39, 2), ("a" * 41, 2), ("$(exit 99)", 2)]:
            with self.subTest(value=value):
                env = dict(os.environ, PEER_COMMIT=value)
                result = subprocess.run(
                    ["bash", "-euo", "pipefail", "-c", textwrap.dedent(script.group("script"))],
                    env=env, capture_output=True, text=True, timeout=10,
                )
                self.assertEqual(result.returncode, expected, result.stdout + result.stderr)


if __name__ == "__main__":
    unittest.main()
