"""Exercise proof-runner refusal boundaries without claiming Rust execution.

Rust tool processes are substituted in these fixtures. Race hooks replace real
fixture files at filesystem-call boundaries; Git, manifests and logs are real.
"""
from contextlib import redirect_stderr, redirect_stdout
import hashlib
import importlib.util
import io
import json
import os
import shutil
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch


ROOT = Path(__file__).resolve().parents[2]
REAL_RUN = subprocess.run
sys.path.insert(0, str(ROOT / "scripts/proof"))


def load(name):
    spec = importlib.util.spec_from_file_location(name, ROOT / "scripts/proof" / (name + ".py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


MIRROR = load("check_x3lang_mirror")
RUNNERS = (load("x3lang_execution_proof"), load("x3lang_runtime_equivalence"))


def git(repo, *args):
    return subprocess.check_output(["git", "-C", str(repo), *args], text=True).strip()


def initialize(repo, paths):
    repo.mkdir(parents=True)
    git(repo, "init", "--quiet")
    git(repo, "config", "user.name", "Proof fixture")
    git(repo, "config", "user.email", "proof@example.invalid")
    git(repo, "config", "core.warnAmbiguousRefs", "true")
    git(repo, "remote", "add", "origin", "https://example.invalid/proof-fixture.git")
    for path in paths:
        target = repo / path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text("tracked fixture input\n")
    git(repo, "add", ".")
    git(repo, "commit", "--quiet", "-m", "fixture baseline")


def invoke(module, repo, *args, capture=None):
    output, errors = io.StringIO(), io.StringIO()
    try:
        with patch.object(module, "ROOT", repo), patch.object(sys, "argv", [module.__name__, *args]), \
                redirect_stdout(output), redirect_stderr(errors):
            return module.main()
    finally:
        if capture is not None:
            capture.update(stdout=output.getvalue(), stderr=errors.getvalue())


class MirrorTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.current, self.peer = self.root / "current", self.root / "peer"
        for repo in (self.current, self.peer):
            initialize(repo, MIRROR.SHARED)

    def compare(self, *args):
        return invoke(MIRROR, self.current, "--peer-repo", str(self.peer), *args)

    def test_missing_mirror_fails_then_restored_bytes_pass(self):
        target = self.peer / MIRROR.SHARED[0]
        original = target.read_bytes()
        target.unlink()
        self.assertEqual(self.compare(), 1)
        target.write_bytes(original)
        self.assertEqual(self.compare(), 0)

    def test_changed_bytes_fail(self):
        (self.peer / MIRROR.SHARED[-1]).write_text("drift\n")
        self.assertEqual(self.compare(), 1)

    def test_self_comparison_is_refused(self):
        with self.assertRaises(SystemExit) as refusal:
            invoke(MIRROR, self.current, "--peer-repo", str(self.current))
        self.assertEqual(refusal.exception.code, 2)

    def test_linked_worktree_peer_is_supported(self):
        linked = self.root / "linked"
        git(self.peer, "worktree", "add", "--quiet", "--detach", str(linked))
        self.assertTrue((linked / ".git").is_file())
        self.assertEqual(invoke(MIRROR, self.current, "--peer-repo", str(linked)), 0)

    def test_changed_scoped_path_is_checked_beyond_fixed_allowlist(self):
        baseline = git(self.current, "rev-parse", "HEAD")
        target = self.current / "x3-lang/new-test.rs"
        target.write_text("new scoped input\n")
        git(self.current, "add", str(target))
        git(self.current, "commit", "--quiet", "-m", "new scoped path")
        self.assertEqual(self.compare("--base-ref", baseline), 1)
        peer_target = self.peer / "x3-lang/new-test.rs"
        peer_target.write_bytes(target.read_bytes())
        git(self.peer, "add", "x3-lang/new-test.rs")
        git(self.peer, "commit", "--quiet", "-m", "mirror scoped input")
        self.assertEqual(self.compare("--base-ref", baseline), 0)

    def test_type_changes_are_checked_without_following_links(self):
        relative = "x3-lang/implementation.rs"
        for repo in (self.current, self.peer):
            (repo / relative).write_text("implementation\n")
            git(repo, "add", relative)
            git(repo, "commit", "--quiet", "-m", "implementation")
        baseline = git(self.current, "rev-parse", "HEAD")
        target = self.current / relative
        target.unlink()
        target.symlink_to("missing.rs")
        git(self.current, "add", relative)
        git(self.current, "commit", "--quiet", "-m", "change file type")
        self.assertEqual(MIRROR.changes(self.current, baseline), {relative})
        self.assertEqual(self.compare("--base-ref", baseline), 1)
        (self.peer / relative).unlink()
        (self.peer / relative).symlink_to("missing.rs")
        git(self.peer, "add", relative)
        git(self.peer, "commit", "--quiet", "-m", "mirror file type")
        self.assertEqual(self.compare("--base-ref", baseline), 0)

    def test_rename_checks_old_path_deletion_on_both_sides(self):
        for repo in (self.current, self.peer):
            (repo / "x3-lang/old.rs").write_text("same source\n")
            git(repo, "add", "x3-lang/old.rs")
            git(repo, "commit", "--quiet", "-m", "old path")
        base, peer_base = (git(repo, "rev-parse", "HEAD") for repo in (self.current, self.peer))
        git(self.current, "mv", "x3-lang/old.rs", "x3-lang/new.rs")
        git(self.current, "commit", "--quiet", "-m", "rename")
        (self.peer / "x3-lang/new.rs").write_text("same source\n")
        git(self.peer, "add", "x3-lang/new.rs")
        git(self.peer, "commit", "--quiet", "-m", "copy without deletion")
        self.assertEqual(MIRROR.changes(self.current, base), {"x3-lang/old.rs", "x3-lang/new.rs"})
        self.assertEqual(self.compare("--base-ref", base, "--peer-base-ref", peer_base), 1)
        (self.peer / "x3-lang/old.rs").unlink()
        git(self.peer, "add", "x3-lang/old.rs")
        git(self.peer, "commit", "--quiet", "-m", "mirror source deletion")
        self.assertEqual(self.compare("--base-ref", base, "--peer-base-ref", peer_base), 0)

    def test_executable_mode_and_required_symlinks_are_checked(self):
        relative = MIRROR.SHARED[0]
        target = self.peer / relative
        target.chmod(0o755)
        self.assertEqual(self.compare(), 1)
        target.chmod(0o644)
        self.assertEqual(self.compare(), 0)
        for repo in (self.current, self.peer):
            file = repo / relative
            file.unlink()
            file.symlink_to("external.py")
            git(repo, "add", relative)
            git(repo, "commit", "--quiet", "-m", "committed required input symlink")
        self.assertEqual(self.compare(), 1)


    def test_symlink_parent_cannot_substitute_identical_external_files(self):
        source = self.peer / "x3-lang"
        outside = self.root / "external-language"
        shutil.copytree(source, outside)
        shutil.rmtree(source)
        source.symlink_to(outside, target_is_directory=True)
        self.assertEqual(self.compare(), 1)

    def test_file_replaced_by_symlink_after_stat_is_refused(self):
        relative = MIRROR.SHARED[0]
        target = self.peer / relative
        outside = self.root / "external-input"
        outside.write_bytes(target.read_bytes())
        real_stat = os.stat
        replaced = False

        def swap_after_stat(path, *args, **kwargs):
            nonlocal replaced
            observed = real_stat(path, *args, **kwargs)
            if not replaced and os.fspath(path) in (str(target), target.name):
                replaced = True
                target.unlink()
                target.symlink_to(outside)
            return observed

        with patch.object(MIRROR.os, "stat", side_effect=swap_after_stat):
            with self.assertRaises((OSError, ValueError)):
                MIRROR.identity(self.peer, relative)
        self.assertTrue(replaced)

    def test_parent_replaced_by_symlink_after_stat_is_refused(self):
        relative = "x3-lang/vm/tests/trading_receipts.rs"
        parent = self.peer / "x3-lang"
        outside = self.root / "external-language"
        real_stat = os.stat
        replaced = False

        def swap_after_stat(path, *args, **kwargs):
            nonlocal replaced
            observed = real_stat(path, *args, **kwargs)
            if not replaced and os.fspath(path) in (str(parent), parent.name):
                replaced = True
                parent.rename(outside)
                parent.symlink_to(outside, target_is_directory=True)
            return observed

        with patch.object(MIRROR.os, "stat", side_effect=swap_after_stat):
            with self.assertRaises((OSError, ValueError)):
                MIRROR.identity(self.peer, relative)
        self.assertTrue(replaced)

    def test_open_file_mode_mutation_during_read_is_refused(self):
        relative = MIRROR.SHARED[0]
        target = self.peer / relative
        original = target.stat()
        real_read = os.read
        changed = False

        def change_after_read(fd, size):
            nonlocal changed
            chunk = real_read(fd, size)
            opened = os.fstat(fd)
            if not changed and (opened.st_dev, opened.st_ino) == (original.st_dev, original.st_ino):
                changed = True
                target.chmod(0o755)
            return chunk

        with patch.object(MIRROR.os, "read", side_effect=change_after_read):
            with self.assertRaisesRegex(ValueError, "changed while reading"):
                MIRROR.identity(self.peer, relative)
        self.assertTrue(changed)

    def test_open_parent_replacement_during_read_is_refused(self):
        relative = "x3-lang/vm/tests/trading_receipts.rs"
        original = (self.peer / relative).stat()
        parent = self.peer / "x3-lang"
        outside = self.root / "external-language"
        real_read = os.read
        changed = False

        def replace_after_read(fd, size):
            nonlocal changed
            chunk = real_read(fd, size)
            opened = os.fstat(fd)
            if not changed and (opened.st_dev, opened.st_ino) == (original.st_dev, original.st_ino):
                changed = True
                parent.rename(outside)
                parent.symlink_to(outside, target_is_directory=True)
            return chunk

        with patch.object(MIRROR.os, "read", side_effect=replace_after_read):
            with self.assertRaisesRegex(ValueError, "parent changed while reading"):
                MIRROR.identity(self.peer, relative)
        self.assertTrue(changed)

    def test_committed_mode_mismatch_cannot_be_hidden_by_normalized_worktree_modes(self):
        relative = MIRROR.SHARED[0]
        target = self.peer / relative
        target.chmod(0o755)
        git(self.peer, "add", relative)
        git(self.peer, "commit", "--quiet", "-m", "committed executable mode")
        git(self.peer, "config", "core.fileMode", "false")
        target.chmod(0o644)
        self.assertEqual(self.compare(), 1)
        target.chmod(0o755)
        self.assertEqual(self.compare(), 1)
        git(self.peer, "update-index", "--chmod=-x", relative)
        git(self.peer, "commit", "--quiet", "-m", "restore committed mode")
        target.chmod(0o644)
        self.assertEqual(self.compare(), 0)

    def test_new_and_deleted_mirror_paths_must_be_committed_on_both_sides(self):
        relative = "x3-lang/new-implementation.rs"
        baseline = git(self.current, "rev-parse", "HEAD")
        (self.current / relative).write_text("new source\n")
        git(self.current, "add", relative)
        git(self.current, "commit", "--quiet", "-m", "new implementation")
        (self.peer / relative).write_text("new source\n")
        self.assertEqual(self.compare("--base-ref", baseline), 1)
        git(self.peer, "add", relative)
        git(self.peer, "commit", "--quiet", "-m", "mirror implementation")
        self.assertEqual(self.compare("--base-ref", baseline), 0)
        base, peer_base = (git(repo, "rev-parse", "HEAD") for repo in (self.current, self.peer))
        git(self.current, "rm", "--quiet", relative)
        git(self.current, "commit", "--quiet", "-m", "delete implementation")
        (self.peer / relative).unlink()
        args = ("--base-ref", base, "--peer-base-ref", peer_base)
        self.assertEqual(self.compare(*args), 1)
        git(self.peer, "add", relative)
        git(self.peer, "commit", "--quiet", "-m", "mirror deletion")
        self.assertEqual(self.compare(*args), 0)

    def test_git_warning_stderr_does_not_become_a_diff_revision_or_path(self):
        git(self.current, "update-ref", "refs/tags/HEAD", git(self.current, "rev-parse", "HEAD"))
        warning = REAL_RUN(["git", "-C", str(self.current), "merge-base", "HEAD", "HEAD"],
                           capture_output=True, text=True, check=True)
        self.assertIn("ambiguous", warning.stderr)
        self.assertEqual(MIRROR.changes(self.current, "HEAD"), set())
        self.assertEqual(self.compare("--base-ref", "HEAD"), 0)

    def test_committed_parent_replacements_preserve_descendant_deletions(self):
        relative = "x3-lang/assets/old.txt"
        for kind in ("file", "symlink"):
            with self.subTest(kind=kind):
                repos = [self.root / kind / side for side in ("current", "peer")]
                bases = []
                outside = self.root / (kind + "-external")
                outside.mkdir()
                (outside / "old.txt").write_text("must not follow this descendant\n")
                for repo in repos:
                    initialize(repo, (*MIRROR.SHARED, relative))
                    bases.append(git(repo, "rev-parse", "HEAD"))
                    (repo / relative).unlink()
                    parent = repo / "x3-lang/assets"
                    parent.rmdir()
                    if kind == "file":
                        parent.write_text("replacement file\n")
                    else:
                        parent.symlink_to(outside, target_is_directory=True)
                    git(repo, "add", "x3-lang/assets")
                    git(repo, "commit", "--quiet", "-m", "replace committed directory")
                self.assertEqual(invoke(MIRROR, repos[0], "--peer-repo", str(repos[1]),
                                        "--base-ref", bases[0], "--peer-base-ref", bases[1]), 0)

    def test_uncommitted_parent_cannot_hide_a_committed_descendant_deletion(self):
        relative = "x3-lang/assets/old.txt"
        bases = []
        for repo in (self.current, self.peer):
            target = repo / relative
            target.parent.mkdir(parents=True)
            target.write_text("old input\n")
            git(repo, "add", relative)
            git(repo, "commit", "--quiet", "-m", "old descendant")
            bases.append(git(repo, "rev-parse", "HEAD"))
            git(repo, "rm", "--quiet", relative)
            git(repo, "commit", "--quiet", "-m", "delete descendant only")
            target.parent.write_text("uncommitted parent replacement\n")
        self.assertEqual(self.compare("--base-ref", bases[0], "--peer-base-ref", bases[1]), 1)


class RunnerTests(unittest.TestCase):
    BUILD_INPUTS = (
        "x3-lang/vm/src/trading.rs", "crates/x3-vm/src/lib.rs", "Cargo.lock",
        "x3-lang/Cargo.lock", "Cargo.toml", "build.rs", ".cargo/config.toml",
        "rust-toolchain.toml",
    )

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.cases = []
        for module in RUNNERS:
            repo = self.root / module.__name__
            inputs = getattr(module, "INPUTS", None)
            if inputs is None:
                inputs = (module.MANIFEST,) + tuple(
                    f"crates/x3-integration/tests/{test}.rs" for _, test, _ in module.SUITES
                )
            initialize(repo, (*inputs, *self.BUILD_INPUTS))
            self.cases.append((module, repo, inputs))

    def run_fixture(self, module, repo, output_name, cargo_code=0, cases=3, absent=False, on_run=None):
        output = self.root / "evidence" / module.__name__ / output_name
        called = False

        def rust_process(command, *args, **kwargs):
            nonlocal called
            if command[0] not in ("cargo", "rustc"):
                return REAL_RUN(command, *args, **kwargs)
            if absent:
                raise FileNotFoundError("Rust tool fixture is absent")
            if "--version" in command:
                return subprocess.CompletedProcess(command, 0, stdout="fixture tool version\n")
            if on_run and not called:
                called = True
                on_run()
            data = f"test result: ok. {cases} passed; 0 failed; 0 ignored\n".encode()
            return subprocess.CompletedProcess(command, cargo_code, stdout=data)

        capture = {}
        with patch.object(subprocess, "run", side_effect=rust_process):
            code = invoke(module, repo, "--output-dir", str(output), capture=capture)
        self.last_stderr = capture["stderr"]
        return code, json.loads((output / "manifest.json").read_text()), output

    def test_zero_executed_cases_are_refused(self):
        for module, repo, _ in self.cases:
            with self.subTest(module=module.__name__):
                code, report, _ = self.run_fixture(module, repo, "zero", cases=0)
                self.assertEqual(code, 1)
                self.assertEqual(report["status"], "failed")
                self.assertTrue(all(t["exit_code"] == 65 for t in report["tests"]))

    def test_nonzero_cargo_exit_remains_failed_even_with_success_text(self):
        for module, repo, _ in self.cases:
            with self.subTest(module=module.__name__):
                code, report, _ = self.run_fixture(module, repo, "failed", cargo_code=101)
                self.assertEqual(code, 1)
                self.assertEqual(report["status"], "failed")
                self.assertTrue(all(t["exit_code"] == 101 for t in report["tests"]))

    def test_missing_rust_tool_writes_failed_manifest(self):
        for module, repo, _ in self.cases:
            with self.subTest(module=module.__name__):
                code, report, _ = self.run_fixture(module, repo, "absent", absent=True)
                self.assertEqual(code, 1)
                self.assertEqual(report["status"], "failed")
                self.assertTrue(all(t["exit_code"] == 127 for t in report["tests"]))

    def test_success_manifest_binds_revision_inputs_and_actual_logs(self):
        for module, repo, inputs in self.cases:
            with self.subTest(module=module.__name__):
                code, report, output = self.run_fixture(module, repo, "positive")
                self.assertEqual(code, 0)
                self.assertEqual(report["status"], "passed")
                expected_schema = "x3lang-local-execution-proof/v2" if hasattr(module, "INPUTS") else "x3lang-local-runtime-equivalence/v2"
                self.assertEqual(report["schema"], expected_schema)
                self.assertEqual(report["repo_revision"], git(repo, "rev-parse", "HEAD"))
                self.assertEqual(report["repo_tree"], git(repo, "rev-parse", "HEAD^{tree}"))
                self.assertEqual(report["source_binding"]["before"], report["source_binding"]["after"])
                self.assertEqual(set(report["inputs_sha256"]), set(inputs))
                for path, digest in report["inputs_sha256"].items():
                    self.assertEqual(digest, hashlib.sha256((repo / path).read_bytes()).hexdigest())
                for test in report["tests"]:
                    self.assertEqual(test["test_cases_passed"], 3)
                    self.assertEqual(test["log_sha256"], hashlib.sha256(
                        (output / test["log_file"]).read_bytes()).hexdigest())

    def test_dirty_or_missing_inputs_are_refused_before_evidence(self):
        for module, repo, inputs in self.cases:
            with self.subTest(module=module.__name__):
                target = repo / inputs[0]
                target.write_text("uncommitted change\n")
                for missing in (False, True):
                    if missing:
                        target.unlink()
                    output = self.root / "evidence" / module.__name__ / ("missing" if missing else "dirty")
                    with self.assertRaises(SystemExit) as refusal:
                        invoke(module, repo, "--output-dir", str(output))
                    self.assertEqual(refusal.exception.code, 2)
                    self.assertFalse((output / "manifest.json").exists())

    def test_dirty_implementation_lock_and_build_inputs_are_refused(self):
        for module, repo, _ in self.cases:
            for relative in self.BUILD_INPUTS:
                for staged in (False, True):
                    with self.subTest(module=module.__name__, path=relative, staged=staged):
                        (repo / relative).write_text("changed build input\n")
                        if staged:
                            git(repo, "add", relative)
                        with self.assertRaises(SystemExit) as refusal:
                            self.run_fixture(module, repo, "dirty-build")
                        self.assertEqual(refusal.exception.code, 2)
                        git(repo, "restore", "--source=HEAD", "--staged", "--worktree", "--", relative)

    def test_untracked_source_is_refused(self):
        for module, repo, _ in self.cases:
            with self.subTest(module=module.__name__):
                (repo / "build-helper.rs").write_text("untracked source\n")
                with self.assertRaises(SystemExit) as refusal:
                    self.run_fixture(module, repo, "untracked")
                self.assertEqual(refusal.exception.code, 2)

    def test_hidden_index_flags_are_refused_even_if_contents_look_clean(self):
        for module, repo, _ in self.cases:
            for flag in ("skip-worktree", "assume-unchanged"):
                with self.subTest(module=module.__name__, flag=flag):
                    relative = self.BUILD_INPUTS[0]
                    git(repo, "update-index", "--" + flag, relative)
                    with self.assertRaises(SystemExit) as refusal:
                        self.run_fixture(module, repo, flag)
                    self.assertEqual(refusal.exception.code, 2)
                    git(repo, "update-index", "--no-" + flag, relative)

    def test_output_inside_checkout_is_refused(self):
        for module, repo, _ in self.cases:
            with self.subTest(module=module.__name__):
                with self.assertRaises(SystemExit) as refusal:
                    invoke(module, repo, "--output-dir", str(repo / "target/evidence"))
                self.assertEqual(refusal.exception.code, 2)
                self.assertFalse((repo / "target/evidence").exists())

    def test_ignored_build_outputs_do_not_invalidate_source_check(self):
        for module, repo, _ in self.cases:
            with self.subTest(module=module.__name__):
                (repo / ".gitignore").write_text("target/\n")
                git(repo, "add", ".gitignore")
                git(repo, "commit", "--quiet", "-m", "ignore build output")
                (repo / "target").mkdir()
                (repo / "target/object").write_bytes(b"compiled output")
                code, report, _ = self.run_fixture(module, repo, "ignored")
                self.assertEqual(code, 0)
                self.assertEqual(report["status"], "passed")

    def test_source_mutation_during_tests_cannot_produce_passed_evidence(self):
        for module, repo, _ in self.cases:
            with self.subTest(module=module.__name__):
                code, report, _ = self.run_fixture(
                    module, repo, "source-mutated",
                    on_run=lambda: (repo / self.BUILD_INPUTS[0]).write_text("changed during run\n"),
                )
                self.assertEqual(code, 1)
                self.assertEqual(report["status"], "failed")
                self.assertIn("uncommitted", report["input_error"])
                self.assertIn(report["input_error"], self.last_stderr)
                self.assertIsNone(report["source_binding"]["after"])

    def test_head_mutation_during_tests_cannot_produce_passed_evidence(self):
        for module, repo, _ in self.cases:
            with self.subTest(module=module.__name__):
                before = git(repo, "rev-parse", "HEAD")
                code, report, _ = self.run_fixture(
                    module, repo, "head-mutated",
                    on_run=lambda: git(repo, "commit", "--quiet", "--allow-empty", "-m", "changed HEAD"),
                )
                self.assertEqual(code, 1)
                self.assertEqual(report["status"], "failed")
                self.assertEqual(report["repo_revision"], before)
                self.assertNotEqual(before, git(repo, "rev-parse", "HEAD"))
                self.assertIn("HEAD/tree changed", report["input_error"])
                self.assertIn(report["input_error"], self.last_stderr)


    def test_executable_changes_are_refused_even_with_core_filemode_false(self):
        for module, repo, _ in self.cases:
            with self.subTest(module=module.__name__):
                relative = self.BUILD_INPUTS[0]
                target = repo / relative
                git(repo, "config", "core.fileMode", "false")
                target.chmod(0o755)
                with self.assertRaises(SystemExit) as refusal:
                    self.run_fixture(module, repo, "hidden-mode")
                self.assertEqual(refusal.exception.code, 2)
                target.chmod(0o644)
                code, report, _ = self.run_fixture(
                    module, repo, "mode-during-run", on_run=lambda: target.chmod(0o755),
                )
                self.assertEqual(code, 1)
                self.assertEqual(report["status"], "failed")
                self.assertIn("uncommitted", report["input_error"])
                target.chmod(0o644)

    def test_git_warning_stderr_does_not_contaminate_bound_hashes(self):
        for module, repo, _ in self.cases:
            with self.subTest(module=module.__name__):
                git(repo, "update-ref", "refs/tags/HEAD", git(repo, "rev-parse", "HEAD"))
                observed = REAL_RUN(["git", "-C", str(repo), "rev-parse", "HEAD"],
                                    capture_output=True, text=True, check=True)
                self.assertIn("ambiguous", observed.stderr)
                code, report, _ = self.run_fixture(module, repo, "warning-stderr")
                self.assertEqual(code, 0)
                self.assertEqual(report["repo_revision"], observed.stdout.strip())
                self.assertRegex(report["repo_tree"], r"^[0-9a-f]{40}$")


if __name__ == "__main__":
    unittest.main()
