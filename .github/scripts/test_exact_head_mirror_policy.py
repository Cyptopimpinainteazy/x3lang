"""Adversarial tests for the proposed exact-head mirror policy."""
from __future__ import annotations

import hashlib
import json
from pathlib import Path
import subprocess
import tempfile
import unittest

from exact_head_mirror_policy import (
    APPROVED,
    UNAPPROVED,
    SCHEMA,
    MirrorPolicyError,
    load_policy_bytes,
    require_policy_matches_base,
    verify_fallback,
    verify_policy_review,
)

MIRRORED = (
    "scripts/proof/check_x3lang_mirror.py",
    "scripts/proof/x3lang_execution_proof.py",
    "scripts/proof/x3lang_runtime_equivalence.py",
    "x3-lang/compiler/tests/test_bytecode_version_gate.rs",
    "x3-lang/compiler/tests/test_control_flow_e2e.rs",
    "x3-lang/vm/tests/test_nonce_replay.rs",
    "x3-lang/vm/tests/trading_receipts.rs",
)


def run_git(repo: Path, *args: str) -> str:
    return subprocess.check_output(
        ["git", "-C", str(repo), *args], text=True, stderr=subprocess.STDOUT
    ).strip()


def commit(repo: Path, message: str) -> str:
    run_git(repo, "add", "-A")
    run_git(repo, "commit", "-m", message)
    return run_git(repo, "rev-parse", "HEAD")


def write(repo: Path, rel: str, content: str) -> None:
    path = repo / rel
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(content)


def initialize(repo: Path, branch: str) -> str:
    repo.mkdir(parents=True)
    run_git(repo, "init", "-b", branch)
    run_git(repo, "config", "user.name", "Mirror Test")
    run_git(repo, "config", "user.email", "mirror@example.invalid")
    for rel in MIRRORED:
        write(repo, rel, f"shared:{rel}\n")
    write(repo, "x3-lang/bridge/state.py", "BASE = 1\n")
    return commit(repo, "base")


class ExactHeadMirrorPolicyTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory()
        root = Path(self.temp.name)
        self.workspace = root / "x3lang"
        self.peer = root / "xxxstar"
        workspace_base = initialize(self.workspace, "main")
        peer_base = initialize(self.peer, "master")
        run_git(
            self.workspace,
            "update-ref",
            "refs/remotes/origin/main",
            workspace_base,
        )
        run_git(self.peer, "update-ref", "refs/remotes/origin/master", peer_base)
        self.anchor = peer_base
        self.policy = {
            "schema": SCHEMA,
            "status": APPROVED,
            "owner_approval_reference": "test://review/approved",
            "peer_repository": "Cyptopimpinainteazy/xxxstar",
            "peer_content_anchor_sha": self.anchor,
            "mirrored_files": {
                rel: hashlib.sha256((self.workspace / rel).read_bytes()).hexdigest()
                for rel in MIRRORED
            },
            "protected_peer_paths": ["scripts/proof/"],
            "x3lang_only_prefixes": ["x3-lang/bridge/"],
        }

    def tearDown(self) -> None:
        self.temp.cleanup()

    def feature_commit(self, rel: str, content: str) -> None:
        write(self.workspace, rel, content)
        commit(self.workspace, f"change {rel}")

    def verify(self) -> dict:
        return verify_fallback(
            self.workspace,
            self.peer,
            self.policy,
            base_ref="origin/main",
            peer_base_ref="origin/master",
            peer_branch="feature/missing",
        )

    def test_unapproved_policy_refuses_missing_peer_branch(self) -> None:
        self.feature_commit("x3-lang/bridge/state.py", "BASE = 2\n")
        self.policy["status"] = "UNAPPROVED"
        self.policy["owner_approval_reference"] = None
        with self.assertRaisesRegex(MirrorPolicyError, "UNAPPROVED"):
            self.verify()

    def test_approved_bridge_only_change_matches_pinned_shared_hashes(self) -> None:
        self.feature_commit("x3-lang/bridge/state.py", "BASE = 2\n")
        result = self.verify()
        self.assertEqual(result["mode"], "OWNER_APPROVED_X3LANG_ONLY_FALLBACK")
        self.assertEqual(result["changed_paths_checked"], ["x3-lang/bridge/state.py"])
        self.assertEqual(result["mirrored_files_checked"], len(MIRRORED))

    def test_vm_change_requires_matching_peer_branch(self) -> None:
        self.feature_commit("x3-lang/vm/src/lib.rs", "pub const X: u8 = 1;\n")
        with self.assertRaisesRegex(MirrorPolicyError, "does not cover"):
            self.verify()

    def test_new_proof_script_requires_matching_peer_branch(self) -> None:
        self.feature_commit("scripts/proof/unreviewed.py", "print('no')\n")
        with self.assertRaisesRegex(MirrorPolicyError, "proof path"):
            self.verify()

    def test_any_shared_file_change_requires_matching_peer_branch(self) -> None:
        rel = "x3-lang/vm/tests/trading_receipts.rs"
        self.feature_commit(rel, "changed\n")
        with self.assertRaisesRegex(MirrorPolicyError, "proof path"):
            self.verify()

    def test_peer_anchor_drift_is_rejected(self) -> None:
        for rel in MIRRORED:
            write(self.peer, rel, f"unapproved:{rel}\n")
        bad_anchor = commit(self.peer, "bad anchor")
        self.policy["peer_content_anchor_sha"] = bad_anchor
        self.feature_commit("x3-lang/bridge/state.py", "BASE = 2\n")
        with self.assertRaisesRegex(MirrorPolicyError, "anchor=.*expected="):
            self.verify()

    def test_approved_policy_change_without_scoped_diff_matches_shared_hashes(self) -> None:
        result = self.verify()
        self.assertEqual(result["changed_paths_checked"], [])

    def test_policy_manifest_must_be_complete_before_fallback(self) -> None:
        self.feature_commit("x3-lang/bridge/state.py", "BASE = 2\n")
        self.policy["mirrored_files"].pop(MIRRORED[0])
        with self.assertRaisesRegex(MirrorPolicyError, "exactly the seven"):
            self.verify()

    def test_policy_manifest_cannot_broaden_the_bridge_allowlist(self) -> None:
        self.policy["x3lang_only_prefixes"] = ["x3-lang/"]
        with self.assertRaisesRegex(MirrorPolicyError, "exactly"):
            load_policy_bytes(
                json.dumps(self.policy).encode(),
                "test-policy",
            )

    def test_policy_cannot_approve_itself_in_the_pr_head(self) -> None:
        base_policy = dict(self.policy)
        base_policy["status"] = "UNAPPROVED"
        base_policy["owner_approval_reference"] = None
        base_path = self.workspace / ".github" / "mirror-policy.json"
        base_path.parent.mkdir(parents=True, exist_ok=True)
        base_path.write_text(json.dumps(base_policy, sort_keys=True))
        base = commit(self.workspace, "unapproved base policy")
        run_git(self.workspace, "update-ref", "refs/remotes/origin/main", base)

        head_policy = dict(self.policy)
        head_policy["status"] = APPROVED
        head_policy["owner_approval_reference"] = "self-approved://not-a-review"
        base_path.write_text(json.dumps(head_policy, sort_keys=True))
        commit(self.workspace, "self-approve policy in feature PR")
        self.feature_commit("x3-lang/bridge/state.py", "BASE = 2\n")

        with self.assertRaisesRegex(MirrorPolicyError, "cannot approve itself"):
            require_policy_matches_base(self.workspace, "origin/main")

    def test_policy_only_change_can_be_reviewed_without_admitting_code(self) -> None:
        proposal = dict(self.policy)
        proposal["status"] = UNAPPROVED
        proposal["owner_approval_reference"] = None
        write(
            self.workspace,
            ".github/mirror-policy.json",
            json.dumps(proposal, sort_keys=True),
        )
        commit(self.workspace, "propose mirror policy")

        result = verify_policy_review(
            self.workspace,
            self.peer,
            proposal,
            base_ref="origin/main",
            peer_base_ref="origin/master",
        )

        self.assertEqual(result["mode"], "POLICY_REVIEW_ONLY")
        self.assertEqual(result["policy_status"], UNAPPROVED)
        self.assertFalse(result["admits_feature_changes"])
        self.assertEqual(result["changed_paths_checked"], [".github/mirror-policy.json"])

    def test_policy_review_refuses_mixed_feature_changes(self) -> None:
        write(
            self.workspace,
            ".github/mirror-policy.json",
            json.dumps(self.policy, sort_keys=True),
        )
        commit(self.workspace, "propose mirror policy")
        self.feature_commit("x3-lang/bridge/state.py", "BASE = 2\n")

        with self.assertRaisesRegex(MirrorPolicyError, "policy-review paths"):
            verify_policy_review(
                self.workspace,
                self.peer,
                self.policy,
                base_ref="origin/main",
                peer_base_ref="origin/master",
            )


if __name__ == "__main__":
    unittest.main()
