"""Public standalone proof admission CLI, using real disposable Git checkouts."""
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

from test_proof_source_binding import ProofFixture, SOURCE


class StandaloneCheckoutTests(unittest.TestCase):
    def run_cli(self, fixture, head=None, output=None):
        return subprocess.run(
            [sys.executable, str(fixture.repo / "scripts/proof/check_x3lang_checkout.py"),
             "--expected-head", head or fixture.revision,
             "--output-dir", str(output or fixture.output)],
            cwd=fixture.repo, env=fixture.env, capture_output=True, text=True, timeout=20,
        )

    def assert_refused(self, fixture, result, reason):
        self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
        self.assertIn(reason, result.stderr)
        self.assertFalse((fixture.output / "checkout.json").exists())
        self.assertFalse(fixture.calls.exists())

    def test_clean_checkout_reports_source_identity_without_peer_or_cargo(self):
        with tempfile.TemporaryDirectory() as temporary:
            fixture = ProofFixture(Path(temporary), "check_x3lang_checkout.py")
            fixture.env.pop("X3LANG_PEER_READ_SSH_KEY", None)
            result = self.run_cli(fixture)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            report = json.loads((fixture.output / "checkout.json").read_text())
            self.assertEqual(report["head"], fixture.revision)
            self.assertEqual(report["cross_repository_parity"], "not_checked")
            self.assertFalse(fixture.calls.exists(), "source admission must not invoke Cargo")

    def test_hidden_dirty_compiler_input_is_refused(self):
        with tempfile.TemporaryDirectory() as temporary:
            fixture = ProofFixture(Path(temporary), "check_x3lang_checkout.py")
            fixture.git("update-index", "--assume-unchanged", SOURCE)
            (fixture.repo / SOURCE).write_text("// uncommitted compiler mutation\n")
            self.assert_refused(fixture, self.run_cli(fixture), SOURCE)

    def test_ignored_untracked_build_input_is_refused(self):
        with tempfile.TemporaryDirectory() as temporary:
            fixture = ProofFixture(Path(temporary), "check_x3lang_checkout.py")
            fixture.git("config", "core.excludesFile", str(Path(temporary) / "ignored"))
            (Path(temporary) / "ignored").write_text("build.rs\n")
            (fixture.repo / "x3-lang/compiler/build.rs").write_text("fn main() {}\n")
            self.assert_refused(fixture, self.run_cli(fixture), "build.rs")

    def test_wrong_head_is_refused(self):
        with tempfile.TemporaryDirectory() as temporary:
            fixture = ProofFixture(Path(temporary), "check_x3lang_checkout.py")
            self.assert_refused(fixture, self.run_cli(fixture, head="0" * 40), "HEAD changed")

    def test_deleted_required_proof_input_is_refused_even_if_committed(self):
        with tempfile.TemporaryDirectory() as temporary:
            fixture = ProofFixture(Path(temporary), "check_x3lang_checkout.py")
            missing = "x3-lang/vm/tests/test_nonce_replay.rs"
            fixture.git("rm", missing)
            fixture.git("commit", "--quiet", "-m", "delete required proof")
            fixture.revision = fixture.git("rev-parse", "HEAD").stdout.strip()
            self.assert_refused(fixture, self.run_cli(fixture), missing)

    def test_required_proof_symlink_is_refused_even_with_committed_target(self):
        with tempfile.TemporaryDirectory() as temporary:
            fixture = ProofFixture(Path(temporary), "check_x3lang_checkout.py")
            relative = "x3-lang/vm/tests/test_nonce_replay.rs"
            target = fixture.repo / relative
            target.unlink()
            target.symlink_to("trading_receipts.rs")
            fixture.git("add", relative)
            fixture.git("commit", "--quiet", "-m", "replace required proof with link")
            fixture.revision = fixture.git("rev-parse", "HEAD").stdout.strip()
            self.assert_refused(fixture, self.run_cli(fixture), "committed regular file")

    def test_output_inside_checkout_is_refused(self):
        with tempfile.TemporaryDirectory() as temporary:
            fixture = ProofFixture(Path(temporary), "check_x3lang_checkout.py")
            self.assert_refused(fixture, self.run_cli(fixture, output=fixture.repo / "report"), "--output-dir")

    def test_automatic_workflow_has_exact_head_and_no_peer_dependency(self):
        workflow = (Path(__file__).resolve().parents[2] / ".github/workflows/x3lang-execution-proof.yml").read_text()
        self.assertIn("Verify standalone committed proof inputs", workflow)
        self.assertIn('check_x3lang_checkout.py --expected-head "$EXPECTED_HEAD"', workflow)
        self.assertIn('EXPECTED_HEAD: ${{ github.event.pull_request.head.sha || github.sha }}', workflow)
        self.assertNotIn("secrets.", workflow)
        self.assertNotIn("repository: Cyptopimpinainteazy/xxxstar", workflow)
        self.assertNotIn("continue-on-error:", workflow)
        self.assertNotIn("pull_request_target:", workflow)
        self.assertIn("python3 scripts/proof/x3lang_execution_proof.py", workflow)


if __name__ == "__main__":
    unittest.main()
