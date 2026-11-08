#!/usr/bin/env python3
"""Exercise proof CLI source admission in owned Git fixtures, without running Cargo."""

from __future__ import annotations

from contextlib import contextmanager
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
RUNNERS = {
    "x3lang_execution_proof.py": 4,
    "x3lang_runtime_equivalence.py": 6,
}
SOURCE = "x3-lang/compiler/src/lib.rs"
REQUIRED = (
    "x3-lang/compiler/tests/test_control_flow_e2e.rs",
    "x3-lang/compiler/tests/test_bytecode_version_gate.rs",
    "x3-lang/vm/tests/test_nonce_replay.rs",
    "x3-lang/vm/tests/trading_receipts.rs",
    "crates/x3-integration/tests/source_language.rs",
    "crates/x3-integration/tests/chain_engine.rs",
    "crates/x3-integration/tests/opcode_differential.rs",
    "crates/x3-integration/tests/gas_accounting.rs",
    "crates/x3-integration/tests/x3lang_trading_rollback.rs",
    "crates/x3-integration/tests/x3lang_trading_receipt_replay.rs",
)


class ProofFixture:
    def __init__(self, base: Path, runner: str):
        self.repo = base / "repo"
        self.output = base / "proof-output"
        self.calls = base / "tool-calls.jsonl"
        self.runner = runner
        self.env = {k: v for k, v in os.environ.items() if not k.startswith("GIT_")}
        self.env.update(GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_GLOBAL=os.devnull)
        self.env.pop("PYTHONPATH", None)
        self.repo.mkdir()
        scripts = self.repo / "scripts/proof"
        scripts.mkdir(parents=True)
        for script in (ROOT / "scripts/proof").glob("*.py"):
            shutil.copyfile(script, scripts / script.name)
        for relative in REQUIRED + (
            SOURCE, "x3-lang/vm/src/lib.rs", "crates/x3-integration/src/lib.rs",
            "crates/x3bc-core/src/lib.rs", "x3-lang/spec/opcodes.rs",
        ):
            path = self.repo / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("// Controlled source fixture; Cargo is never executed.\n")
        for relative in (
            "Cargo.toml", "x3-lang/Cargo.toml", "x3-lang/compiler/Cargo.toml",
            "x3-lang/vm/Cargo.toml", "crates/x3-integration/Cargo.toml",
        ):
            (self.repo / relative).write_text("# Controlled manifest fixture.\n")
        (self.repo / "x3-lang/Cargo.lock").write_text("version = 3\n")
        (self.repo / ".gitignore").write_text("**/target/\n**/__pycache__/\n")
        self.git("init", "--quiet", "--template=")
        self.git("config", "user.name", "Source binding test")
        self.git("config", "user.email", "source-binding@example.invalid")
        self.git("remote", "add", "origin", "https://example.invalid/proof-fixture.git")
        self.git("add", "--", ".")
        self.git("commit", "--quiet", "-m", "Controlled fixture")
        self.revision = self.git("rev-parse", "HEAD").stdout.strip()

        binaries = base / "bin"
        binaries.mkdir()
        stub = f"#!{sys.executable}\n" + '''\
import json
import os
from pathlib import Path
import subprocess
import sys

tool = Path(sys.argv[0]).name
with Path(os.environ["PROOF_STUB_CALLS"]).open("a") as handle:
    handle.write(json.dumps([tool, *sys.argv[1:]]) + "\\n")
if sys.argv[1:] == ["--version"]:
    print(tool + " fixture-only")
elif tool == "cargo" and sys.argv[1:2] == ["test"]:
    if os.environ.get("PROOF_STUB_MUTATE"):
        with Path(os.environ["PROOF_STUB_MUTATE"]).open("a") as handle:
            handle.write("// Mutation during a stub suite.\\n")
    if os.environ.get("PROOF_STUB_NEW_HEAD"):
        subprocess.run(["git", "update-ref", "HEAD", os.environ["PROOF_STUB_NEW_HEAD"]],
                       check=True)
    print("test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out")
else:
    sys.exit("Unexpected fixture tool invocation")
'''
        for name in ("cargo", "rustc"):
            path = binaries / name
            path.write_text(stub)
            path.chmod(0o755)
        self.env.update(
            PATH=str(binaries) + os.pathsep + os.environ["PATH"],
            PROOF_STUB_CALLS=str(self.calls),
        )

    def git(self, *args: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            ["git", "-C", str(self.repo), *args], env=self.env,
            check=True, capture_output=True, text=True, timeout=10,
        )

    def run(self) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [sys.executable, str(self.repo / "scripts/proof" / self.runner),
             "--output-dir", str(self.output)],
            cwd=self.repo, env=self.env, capture_output=True, text=True, timeout=20,
        )


class ProofSourceBindingTests(unittest.TestCase):
    @contextmanager
    def fixture(self, runner):
        with tempfile.TemporaryDirectory(prefix="x3-proof-binding-") as temporary:
            yield ProofFixture(Path(temporary), runner)

    def assert_refused_before_tools(self, fixture, expected_path):
        result = fixture.run()
        self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
        self.assertIn("source binding", result.stderr.lower())
        self.assertIn(expected_path, result.stderr)
        self.assertFalse(fixture.calls.exists(), "Refusal must precede Cargo/rustc execution")
        self.assertFalse((fixture.output / "manifest.json").exists())

    def test_dirty_tracked_inputs_are_refused_before_tools(self):
        for runner in RUNNERS:
            for relative in (
                SOURCE, "x3-lang/vm/src/lib.rs", "crates/x3-integration/src/lib.rs",
                "crates/x3bc-core/src/lib.rs", "x3-lang/spec/opcodes.rs",
                "x3-lang/Cargo.toml", "x3-lang/Cargo.lock",
                "scripts/proof/" + runner,
            ):
                with self.subTest(runner=runner, path=relative), self.fixture(runner) as fixture:
                    with (fixture.repo / relative).open("a") as handle:
                        handle.write("\n# changed\n" if relative.endswith(".py") else "\n// changed\n")
                    self.assert_refused_before_tools(fixture, relative)

    def test_index_changes_and_hidden_worktree_edits_are_refused(self):
        for runner in RUNNERS:
            for mode in ("staged", "staged_then_restored", "assume-unchanged", "skip-worktree"):
                with self.subTest(runner=runner, mode=mode), self.fixture(runner) as fixture:
                    path = fixture.repo / SOURCE
                    original = path.read_bytes()
                    if mode in ("assume-unchanged", "skip-worktree"):
                        fixture.git("update-index", "--" + mode, SOURCE)
                    path.write_bytes(original + b"// changed\n")
                    if mode.startswith("staged"):
                        fixture.git("add", "--", SOURCE)
                    if mode == "staged_then_restored":
                        path.write_bytes(original)
                    self.assert_refused_before_tools(fixture, SOURCE)

    def test_untracked_inputs_are_refused_even_when_ignored(self):
        for runner in RUNNERS:
            for relative in (
                "x3-lang/compiler/src/untracked.rs", "x3-lang/compiler/build.rs",
                ".cargo/config.toml", "rust-toolchain.toml",
                "crates/x3-integration/tests/untracked.json",
            ):
                for state in ("untracked", "ignored", "intent-to-add"):
                    with self.subTest(runner=runner, path=relative, state=state), self.fixture(runner) as fixture:
                        path = fixture.repo / relative
                        path.parent.mkdir(parents=True, exist_ok=True)
                        # No compiler/config commands in fixtures; only inert comments/data.
                        path.write_text("# Controlled untracked input.\n")
                        if state == "ignored":
                            exclude = fixture.repo / ".git/info/exclude"
                            exclude.parent.mkdir(parents=True, exist_ok=True)
                            exclude.write_text("/" + relative + "\n")
                        elif state == "intent-to-add":
                            fixture.git("add", "--intent-to-add", "--", relative)
                        self.assert_refused_before_tools(fixture, relative)

    def test_output_directory_cannot_exempt_source_from_binding(self):
        for runner in RUNNERS:
            for relative in ("x3-lang/compiler/src", "target/proof"):
                with self.subTest(runner=runner, output=relative), self.fixture(runner) as fixture:
                    fixture.output = fixture.repo / relative
                    self.assert_refused_before_tools(fixture, "--output-dir")

    def test_clean_control_and_existing_generated_outputs_are_accepted(self):
        for runner, suites in RUNNERS.items():
            with self.subTest(runner=runner), self.fixture(runner) as fixture:
                artifact = fixture.repo / "x3-lang/target/debug/fixture.rlib"
                artifact.parent.mkdir(parents=True)
                artifact.write_bytes(b"generated fixture, never loaded")
                for _ in range(2):
                    result = fixture.run()
                    self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                    report = json.loads((fixture.output / "manifest.json").read_text())
                    self.assertEqual(report["status"], "passed")
                    self.assertEqual(report["repo_revision"], fixture.revision)
                    self.assertEqual(len(report["tests"]), suites)
                calls = [json.loads(line) for line in fixture.calls.read_text().splitlines()]
                self.assertEqual(sum(call[:2] == ["cargo", "test"] for call in calls), suites * 2)

    def test_source_mutation_during_suites_prevents_passed_manifest(self):
        for runner in RUNNERS:
            with self.subTest(runner=runner), self.fixture(runner) as fixture:
                fixture.env["PROOF_STUB_MUTATE"] = str(fixture.repo / SOURCE)
                result = fixture.run()
                self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
                report = json.loads((fixture.output / "manifest.json").read_text())
                self.assertEqual(report["status"], "failed")
                self.assertIn(SOURCE, report["source_binding_error"])

    def test_head_change_during_suites_prevents_passed_manifest(self):
        for runner in RUNNERS:
            with self.subTest(runner=runner), self.fixture(runner) as fixture:
                fixture.git("commit", "--quiet", "--allow-empty", "-m", "Next fixture revision")
                next_revision = fixture.git("rev-parse", "HEAD").stdout.strip()
                fixture.git("update-ref", "HEAD", fixture.revision)
                fixture.env["PROOF_STUB_NEW_HEAD"] = next_revision
                result = fixture.run()
                self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
                report = json.loads((fixture.output / "manifest.json").read_text())
                self.assertEqual(report["status"], "failed")
                self.assertEqual(report["repo_revision"], fixture.revision)
                self.assertIn("HEAD changed", report["source_binding_error"])

    def test_missing_canonical_workspace_is_an_explicit_refusal(self):
        with self.fixture("x3lang_runtime_equivalence.py") as fixture:
            (fixture.repo / "crates/x3-integration/Cargo.toml").unlink()
            result = fixture.run()
            self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
            self.assertIn("canonical runtime workspace absent", result.stderr)
            self.assertFalse(fixture.calls.exists())


if __name__ == "__main__":
    unittest.main()
