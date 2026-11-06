#!/usr/bin/env python3
"""Fail-closed exact-head mirror policy for stacked X3Lang feature branches.

The default remains the existing strict same-branch mirror gate. A missing peer
branch can use x3lang-only bridge changes only after an owner-approved policy
is committed, the peer repository is pinned to an exact content anchor, and
every always-shared proof file matches the reviewed SHA-256 manifest.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess
import sys
from typing import Any

SCHEMA = "x3.exact-head-mirror-policy.v1"
APPROVED = "OWNER_APPROVED"
UNAPPROVED = "UNAPPROVED"
SHA256_RE = re.compile(r"^[0-9a-f]{64}$")
SHA1_RE = re.compile(r"^[0-9a-f]{40}$")
MIRRORED_PATHS = frozenset(
    {
        "scripts/proof/check_x3lang_mirror.py",
        "scripts/proof/x3lang_execution_proof.py",
        "scripts/proof/x3lang_runtime_equivalence.py",
        "x3-lang/compiler/tests/test_bytecode_version_gate.rs",
        "x3-lang/compiler/tests/test_control_flow_e2e.rs",
        "x3-lang/vm/tests/test_nonce_replay.rs",
        "x3-lang/vm/tests/trading_receipts.rs",
    }
)
DEFAULT_POLICY_PATH = ".github/mirror-policy.json"
EXPECTED_PROTECTED_PEER_PATHS = ["scripts/proof/"]
EXPECTED_X3LANG_ONLY_PREFIXES = ["x3-lang/bridge/"]
POLICY_REVIEW_PATHS = frozenset(
    {
        ".github/CODEOWNERS",
        ".github/mirror-policy.json",
        ".github/scripts/exact_head_mirror_policy.py",
        ".github/scripts/test_exact_head_mirror_policy.py",
        ".github/workflows/x3lang-execution-proof.yml",
        "docs/security/X3LANG_EXACT_HEAD_MIRROR_POLICY.md",
    }
)


class MirrorPolicyError(RuntimeError):
    """A mirror-policy invariant was not satisfied."""


def git(repo: Path, *args: str, text: bool = True) -> str | bytes:
    completed = subprocess.run(
        ["git", "-C", str(repo), *args],
        check=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    return completed.stdout.decode().strip() if text else completed.stdout


def git_available(repo: Path) -> bool:
    try:
        git(repo, "rev-parse", "--git-dir")
        return True
    except subprocess.CalledProcessError:
        return False


def ensure_peer_checkout(peer: Path, peer_url: str) -> None:
    if not peer.exists():
        peer.parent.mkdir(parents=True, exist_ok=True)
        subprocess.run(
            [
                "git",
                "clone",
                "--no-checkout",
                "--filter=blob:none",
                "--origin",
                "origin",
                peer_url,
                str(peer),
            ],
            check=True,
        )
    elif not git_available(peer):
        raise MirrorPolicyError(f"peer path is not a Git checkout: {peer}")
    git(peer, "fetch", "--prune", "origin", "+refs/heads/*:refs/remotes/origin/*")


def valid_branch_name(branch: str) -> bool:
    if not branch or branch.startswith("-") or "\n" in branch:
        return False
    return subprocess.run(
        ["git", "check-ref-format", "--branch", branch],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    ).returncode == 0


def resolve_commit(repo: Path, ref: str) -> str:
    try:
        return str(git(repo, "rev-parse", "--verify", f"{ref}^{{commit}}"))
    except subprocess.CalledProcessError as exc:
        raise MirrorPolicyError(f"cannot resolve Git revision {ref} in {repo}") from exc


def ref_file_sha256(repo: Path, ref: str, rel: str) -> str | None:
    try:
        content = git(repo, "show", f"{ref}:{rel}", text=False)
    except subprocess.CalledProcessError:
        return None
    assert isinstance(content, bytes)
    return hashlib.sha256(content).hexdigest()


def changed_paths(repo: Path, base_ref: str, head_ref: str = "HEAD") -> set[str]:
    try:
        merge_base = str(git(repo, "merge-base", base_ref, head_ref))
        output = str(
            git(
                repo,
                "diff",
                "--name-only",
                "--diff-filter=ACDMR",
                merge_base,
                head_ref,
            )
        )
    except subprocess.CalledProcessError as exc:
        raise MirrorPolicyError(
            f"cannot establish comparable revisions for {head_ref} against {base_ref}"
        ) from exc
    return set(output.splitlines()) if output else set()


def load_policy_bytes(content: bytes, source: str) -> dict[str, Any]:
    try:
        policy = json.loads(content)
    except (UnicodeDecodeError, json.JSONDecodeError) as exc:
        raise MirrorPolicyError(f"cannot read mirror policy {source}: {exc}") from exc
    if not isinstance(policy, dict):
        raise MirrorPolicyError("mirror policy must be a JSON object")
    if policy.get("schema") != SCHEMA:
        raise MirrorPolicyError(f"unexpected mirror policy schema: {policy.get('schema')!r}")
    status = policy.get("status")
    if status not in {UNAPPROVED, APPROVED}:
        raise MirrorPolicyError(f"unsupported mirror policy status: {status!r}")
    if policy.get("peer_repository") != "Cyptopimpinainteazy/xxxstar":
        raise MirrorPolicyError("peer repository must remain Cyptopimpinainteazy/xxxstar")
    anchor = policy.get("peer_content_anchor_sha")
    if not isinstance(anchor, str) or SHA1_RE.fullmatch(anchor) is None:
        raise MirrorPolicyError("peer_content_anchor_sha must be a full 40-character commit SHA")
    mirrored = policy.get("mirrored_files")
    if not isinstance(mirrored, dict) or set(mirrored) != MIRRORED_PATHS:
        raise MirrorPolicyError(
            "mirrored_files must contain exactly the seven reviewed proof paths"
        )
    for rel, digest in mirrored.items():
        if not isinstance(rel, str) or not rel or Path(rel).is_absolute() or ".." in Path(rel).parts:
            raise MirrorPolicyError(f"unsafe mirrored path: {rel!r}")
        if not isinstance(digest, str) or SHA256_RE.fullmatch(digest) is None:
            raise MirrorPolicyError(f"invalid SHA-256 for mirrored path {rel}")
    for key in ("protected_peer_paths", "x3lang_only_prefixes"):
        values = policy.get(key)
        if not isinstance(values, list) or not all(isinstance(v, str) and v for v in values):
            raise MirrorPolicyError(f"{key} must be a non-empty string list or empty list")
    if policy["protected_peer_paths"] != EXPECTED_PROTECTED_PEER_PATHS:
        raise MirrorPolicyError(
            "protected_peer_paths must be exactly ['scripts/proof/']"
        )
    if policy["x3lang_only_prefixes"] != EXPECTED_X3LANG_ONLY_PREFIXES:
        raise MirrorPolicyError(
            "x3lang_only_prefixes must be exactly ['x3-lang/bridge/']"
        )
    if status == APPROVED:
        approval = policy.get("owner_approval_reference")
        if not isinstance(approval, str) or not approval.strip():
            raise MirrorPolicyError("OWNER_APPROVED policy requires owner_approval_reference")
    return policy


def load_policy(path: Path) -> dict[str, Any]:
    try:
        content = path.read_bytes()
    except OSError as exc:
        raise MirrorPolicyError(f"cannot read mirror policy {path}: {exc}") from exc
    return load_policy_bytes(content, str(path))


def load_policy_at_ref(
    repo: Path, ref: str, rel: str = DEFAULT_POLICY_PATH
) -> dict[str, Any]:
    try:
        content = git(repo, "show", f"{ref}:{rel}", text=False)
    except subprocess.CalledProcessError as exc:
        raise MirrorPolicyError(
            f"cannot read mirror policy {rel} from base revision {ref}"
        ) from exc
    assert isinstance(content, bytes)
    return load_policy_bytes(content, f"{ref}:{rel}")


def require_policy_matches_base(
    repo: Path, base_ref: str, rel: str = DEFAULT_POLICY_PATH
) -> dict[str, Any]:
    """Approval comes from the base branch, never from the PR being admitted."""
    try:
        base = git(repo, "show", f"{base_ref}:{rel}", text=False)
        head = git(repo, "show", f"HEAD:{rel}", text=False)
    except subprocess.CalledProcessError as exc:
        raise MirrorPolicyError(
            f"the mirror policy {rel} must exist at both {base_ref} and HEAD"
        ) from exc
    assert isinstance(base, bytes)
    assert isinstance(head, bytes)
    if base != head:
        raise MirrorPolicyError(
            f"{rel} differs from {base_ref}; an admission policy cannot approve itself "
            "inside the PR it is checking"
        )
    return load_policy_bytes(base, f"{base_ref}:{rel}")


def verify_shared_files(
    workspace: Path, peer: Path, policy: dict[str, Any]
) -> str:
    """Verify the reviewed shared-file manifest in both repositories."""
    mirrored = policy.get("mirrored_files")
    if not isinstance(mirrored, dict) or set(mirrored) != MIRRORED_PATHS:
        raise MirrorPolicyError(
            "mirrored_files must contain exactly the seven reviewed proof paths"
        )
    anchor = resolve_commit(peer, policy["peer_content_anchor_sha"])
    failures: list[str] = []
    for rel, expected in sorted(mirrored.items()):
        current = ref_file_sha256(workspace, "HEAD", rel)
        anchored = ref_file_sha256(peer, anchor, rel)
        if current != expected:
            failures.append(f"{rel}: current={current or 'missing'} expected={expected}")
        if anchored != expected:
            failures.append(f"{rel}: anchor={anchored or 'missing'} expected={expected}")
    if failures:
        raise MirrorPolicyError("mirrored proof file drift:\n" + "\n".join(failures))
    return anchor


def verify_policy_review(
    workspace: Path,
    peer: Path,
    policy: dict[str, Any],
    *,
    base_ref: str,
    peer_base_ref: str,
) -> dict[str, Any]:
    """Validate a policy-only PR without admitting any feature changes."""
    current_changes = changed_paths(workspace, base_ref)
    outside_review_scope = sorted(current_changes - POLICY_REVIEW_PATHS)
    if not current_changes:
        raise MirrorPolicyError("policy review requires a policy-only diff")
    if outside_review_scope:
        raise MirrorPolicyError(
            "policy review is limited to policy-review paths; found: "
            + ", ".join(outside_review_scope)
        )
    resolve_commit(peer, peer_base_ref)
    anchor = verify_shared_files(workspace, peer, policy)
    return {
        "mode": "POLICY_REVIEW_ONLY",
        "policy_status": policy["status"],
        "admits_feature_changes": False,
        "peer_anchor_sha": anchor,
        "changed_paths_checked": sorted(current_changes),
        "mirrored_files_checked": len(policy["mirrored_files"]),
        "financial_or_consensus_authority": False,
    }


def verify_fallback(
    workspace: Path,
    peer: Path,
    policy: dict[str, Any],
    *,
    base_ref: str,
    peer_base_ref: str,
    peer_branch: str,
) -> dict[str, Any]:
    """Verify the x3lang-only fallback without changing either checkout."""
    # Re-validate even when callers construct a policy programmatically.
    if policy["status"] != APPROVED:
        raise MirrorPolicyError(
            f"peer branch {peer_branch!r} is missing and mirror policy is UNAPPROVED; "
            "create the matching peer branch or obtain owner approval for this policy in a reviewed PR"
        )
    resolve_commit(peer, peer_base_ref)
    current_changes = changed_paths(workspace, base_ref)
    protected = tuple(policy["protected_peer_paths"])
    allowed = tuple(policy["x3lang_only_prefixes"])
    for rel in sorted(current_changes):
        if rel in MIRRORED_PATHS or rel.startswith(protected):
            raise MirrorPolicyError(
                f"matching peer branch is required because proof path changed: {rel}"
            )
        if not rel.startswith(allowed):
            raise MirrorPolicyError(
                f"x3lang-only fallback does not cover changed path: {rel}"
            )
    anchor = verify_shared_files(workspace, peer, policy)
    return {
        "mode": "OWNER_APPROVED_X3LANG_ONLY_FALLBACK",
        "peer_branch": peer_branch,
        "peer_anchor_sha": anchor,
        "changed_paths_checked": sorted(current_changes),
        "mirrored_files_checked": len(policy["mirrored_files"]),
        "financial_or_consensus_authority": False,
    }


def run_strict_peer_check(
    workspace: Path,
    peer: Path,
    policy: dict[str, Any],
    *,
    branch_ref: str,
    base_ref: str,
    peer_base_ref: str,
) -> int:
    git(peer, "checkout", "--detach", branch_ref)
    command = [
        sys.executable,
        str(workspace / "scripts/proof/check_x3lang_mirror.py"),
        "--peer-repo",
        str(peer),
        "--base-ref",
        base_ref,
        "--peer-base-ref",
        peer_base_ref,
    ]
    result = subprocess.run(command, cwd=workspace)
    if result.returncode == 0:
        print(
            json.dumps(
                {
                    "mode": "STRICT_SAME_BRANCH_MIRROR",
                    "policy_status": policy["status"],
                    "mirrored_files_manifest_ignored_for_branch_check": True,
                },
                sort_keys=True,
            )
        )
    return result.returncode


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--workspace", type=Path, default=Path.cwd())
    parser.add_argument("--peer-repo", type=Path, required=True)
    parser.add_argument("--peer-url", required=True)
    parser.add_argument("--branch", required=True)
    parser.add_argument("--base-ref", default="origin/main")
    parser.add_argument("--peer-base-ref", default="origin/master")
    parser.add_argument(
        "--policy", type=Path, default=Path(".github/mirror-policy.json")
    )
    args = parser.parse_args()
    workspace = args.workspace.resolve()
    peer = args.peer_repo.resolve()
    policy_path = args.policy if args.policy.is_absolute() else workspace / args.policy
    try:
        if not valid_branch_name(args.branch):
            raise MirrorPolicyError(f"invalid peer branch name: {args.branch!r}")
        policy = load_policy(policy_path)
        ensure_peer_checkout(peer, args.peer_url)
        branch_ref = f"refs/remotes/origin/{args.branch}"
        try:
            resolved_branch = resolve_commit(peer, branch_ref)
        except MirrorPolicyError:
            current_changes = changed_paths(workspace, args.base_ref)
            if current_changes and current_changes <= POLICY_REVIEW_PATHS:
                result = verify_policy_review(
                    workspace,
                    peer,
                    policy,
                    base_ref=args.base_ref,
                    peer_base_ref=args.peer_base_ref,
                )
                print(json.dumps(result, indent=2, sort_keys=True))
                return 0
            policy = require_policy_matches_base(
                workspace,
                args.base_ref,
                str(args.policy),
            )
            result = verify_fallback(
                workspace,
                peer,
                policy,
                base_ref=args.base_ref,
                peer_base_ref=args.peer_base_ref,
                peer_branch=args.branch,
            )
            print(json.dumps(result, indent=2, sort_keys=True))
            return 0
        print(f"STRICT PEER BRANCH {resolved_branch}")
        return run_strict_peer_check(
            workspace,
            peer,
            policy,
            branch_ref=branch_ref,
            base_ref=args.base_ref,
            peer_base_ref=args.peer_base_ref,
        )
    except (MirrorPolicyError, subprocess.CalledProcessError) as exc:
        print(f"EXACT-HEAD MIRROR REFUSED: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
