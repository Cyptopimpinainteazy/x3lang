"""Exercise the real allocator gate with controlled Cargo subprocess outcomes."""

import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts/check-near-allocator.sh"
CLEAN_TREE = "x3-guard v0.1.0\nnear-sdk v5.17.2\n"
CLEAN_LOCK = 'version = 4\n\n[[package]]\nname = "near-sdk"\nversion = "5.17.2"\n'


class NearAllocatorGateTests(unittest.TestCase):
    def setUp(self):
        """Copy the script and a lock fixture; never resolve real dependencies."""
        temporary = tempfile.TemporaryDirectory(prefix="x3-allocator-gate-")
        self.addCleanup(temporary.cleanup)
        self.tmp = Path(temporary.name)
        repo = self.tmp / "checkout"
        scripts = repo / "scripts"
        scripts.mkdir(parents=True)
        self.script = scripts / "check-near-allocator.sh"
        shutil.copyfile(SCRIPT, self.script)
        self.guard = repo / "x3-lang/near/x3-guard"
        self.guard.mkdir(parents=True)
        self.lockfile = self.guard / "Cargo.lock"
        self.lockfile.write_text(CLEAN_LOCK)
        self.binaries = self.tmp / "bin"
        self.binaries.mkdir()
        cargo = self.binaries / "cargo"
        cargo.write_text(
            '#!/bin/sh\n'
            'printf "%s\\n" "$*" > "$X3_TEST_CARGO_ARGS"\n'
            'pwd > "$X3_TEST_CARGO_CWD"\n'
            'printf "%s" "$X3_TEST_CARGO_STDOUT"\n'
            'if [ "$X3_TEST_CARGO_EXIT" != 0 ]; then\n'
            '  echo "intentional probe: dependency graph unavailable" >&2\n'
            'fi\n'
            'exit "$X3_TEST_CARGO_EXIT"\n'
        )
        cargo.chmod(0o755)
        self.real_grep = shutil.which("grep", path=os.defpath)
        self.assertIsNotNone(self.real_grep, "allocator gate requires grep")
        grep = self.binaries / "grep"
        grep.write_text(
            '#!/bin/sh\n'
            'count=0\n'
            'if [ -f "$X3_TEST_GREP_COUNT" ]; then\n'
            '  read -r count < "$X3_TEST_GREP_COUNT"\n'
            'fi\n'
            'count=$((count + 1))\n'
            'printf "%s\\n" "$count" > "$X3_TEST_GREP_COUNT"\n'
            'if [ "$count" -eq "$X3_TEST_GREP_FAIL_ON" ]; then\n'
            '  echo "intentional probe: grep inspection failed" >&2\n'
            '  exit 2\n'
            'fi\n'
            'exec "$X3_TEST_REAL_GREP" "$@"\n'
        )
        grep.chmod(0o755)
        self.args_file = self.tmp / "cargo-args"
        self.cwd_file = self.tmp / "cargo-cwd"

    def run_gate(self, *, cargo_exit=0, tree=CLEAN_TREE, lock=CLEAN_LOCK, grep_fail_on=0):
        if lock is None:
            self.lockfile.unlink()
        else:
            self.lockfile.write_text(lock)
        grep_count = self.tmp / "grep-count"
        grep_count.unlink(missing_ok=True)
        result = subprocess.run(
            ["bash", str(self.script)],
            cwd=self.tmp,
            env={
                "PATH": str(self.binaries) + os.pathsep + os.defpath,
                "X3_TEST_CARGO_EXIT": str(cargo_exit),
                "X3_TEST_CARGO_STDOUT": tree,
                "X3_TEST_CARGO_ARGS": str(self.args_file),
                "X3_TEST_CARGO_CWD": str(self.cwd_file),
                "X3_TEST_GREP_COUNT": str(grep_count),
                "X3_TEST_GREP_FAIL_ON": str(grep_fail_on),
                "X3_TEST_REAL_GREP": self.real_grep,
            },
            capture_output=True,
            text=True,
            timeout=10,
            check=False,
        )
        self.assertEqual(
            self.args_file.read_text().strip(),
            "tree --target wasm32-unknown-unknown --locked --prefix none",
        )
        self.assertEqual(self.cwd_file.read_text().strip(), str(self.guard))
        return result

    def assert_cargo_failure(self, tree):
        result = self.run_gate(cargo_exit=42, tree=tree)
        self.assertEqual(result.returncode, 42, result.stdout + result.stderr)
        self.assertIn("dependency graph unavailable", result.stderr)
        self.assertNotIn("OK:", result.stdout)

    def test_clean_graph_and_lock_pass(self):
        result = self.run_gate()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("OK: no wee_alloc", result.stdout)

    def test_cargo_failure_without_output_is_rejected(self):
        self.assert_cargo_failure("")

    def test_cargo_failure_with_partial_clean_output_is_rejected(self):
        self.assert_cargo_failure(CLEAN_TREE)

    def test_cargo_failure_with_forbidden_output_is_rejected(self):
        self.assert_cargo_failure("wee_alloc v0.4.5\n")

    def test_successful_but_empty_graph_is_unverified(self):
        result = self.run_gate(tree="")
        self.assertNotEqual(result.returncode, 0)
        self.assertNotIn("OK:", result.stdout)

    def test_whitespace_only_graph_is_unverified(self):
        result = self.run_gate(tree=" \t\n\t \n")
        self.assertNotEqual(result.returncode, 0)
        self.assertNotIn("OK:", result.stdout)

    def test_forbidden_allocator_in_graph_is_rejected(self):
        result = self.run_gate(tree=CLEAN_TREE + "wee_alloc v0.4.5\n")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("includes unmaintained wee_alloc", result.stderr)
        self.assertNotIn("OK:", result.stdout)

    def test_forbidden_allocator_in_lock_is_rejected(self):
        result = self.run_gate(
            lock=CLEAN_LOCK + '\n[[package]]\nname = "wee_alloc"\nversion = "0.4.5"\n'
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Cargo.lock still resolves wee_alloc", result.stderr)
        self.assertNotIn("OK:", result.stdout)

    def test_forbidden_allocator_in_large_graph_is_rejected(self):
        result = self.run_gate(tree="wee_alloc v0.4.5\n" + "near-sdk v5.17.2\n" * 3000)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("includes unmaintained wee_alloc", result.stderr)
        self.assertNotIn("OK:", result.stdout)

    def test_forbidden_allocator_lock_with_toml_spacing(self):
        for name_line in (
            '  name="wee_alloc"',
            '\tname\t=\t"wee_alloc"\t',
            'name = "wee_alloc" # forbidden even outside the wasm32 graph',
            "name = 'wee_alloc'",
            "\tname\t= 'wee_alloc' # single-quoted TOML strings are valid",
        ):
            with self.subTest(name_line=name_line):
                result = self.run_gate(
                    lock=CLEAN_LOCK + '\n[[package]]\n' + name_line + '\nversion = "0.4.5"\n'
                )
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("Cargo.lock still resolves wee_alloc", result.stderr)
                self.assertNotIn("OK:", result.stdout)

    def test_graph_inspection_error_cannot_report_success(self):
        result = self.run_gate(tree="wee_alloc v0.4.5\n", grep_fail_on=1)
        self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
        self.assertIn("grep inspection failed", result.stderr)
        self.assertNotIn("OK:", result.stdout)

    def test_lock_inspection_error_cannot_report_success(self):
        result = self.run_gate(
            lock=CLEAN_LOCK + '\n[[package]]\nname = "wee_alloc"\nversion = "0.4.5"\n',
            grep_fail_on=2,
        )
        self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
        self.assertIn("grep inspection failed", result.stderr)
        self.assertNotIn("OK:", result.stdout)

    def test_empty_lock_cannot_report_success(self):
        result = self.run_gate(lock="")
        self.assertNotEqual(result.returncode, 0)
        self.assertNotIn("OK:", result.stdout)

    def test_whitespace_only_lock_cannot_report_success(self):
        result = self.run_gate(lock=" \t\n\t \n")
        self.assertNotEqual(result.returncode, 0)
        self.assertNotIn("OK:", result.stdout)

    def test_missing_lock_cannot_report_success(self):
        result = self.run_gate(lock=None)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Cargo.lock", result.stderr)
        self.assertNotIn("OK:", result.stdout)


if __name__ == "__main__":
    unittest.main()
