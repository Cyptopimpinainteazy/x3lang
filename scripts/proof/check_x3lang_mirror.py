#!/usr/bin/env python3
"""Fail closed when shared X3Lang files or feature changes diverge across repositories."""
from __future__ import annotations

import argparse
from contextlib import ExitStack
import hashlib
import os
from pathlib import Path
import stat
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]
SHARED = (
    "scripts/proof/checkout_integrity.py",
    "scripts/proof/x3lang_execution_proof.py",
    "scripts/proof/x3lang_runtime_equivalence.py",
    "scripts/proof/source_binding.py",
    "scripts/proof/check_x3lang_mirror.py",
    "x3-lang/compiler/tests/test_control_flow_e2e.rs",
    "x3-lang/compiler/tests/test_bytecode_version_gate.rs",
    "x3-lang/vm/tests/test_nonce_replay.rs",
    "x3-lang/vm/tests/trading_receipts.rs",
)
SCOPED = ("x3-lang/", "scripts/proof/")


class NonDirectoryParent(ValueError):
    def __init__(self, relative: str):
        super().__init__("proof input parent is not a regular directory")
        self.relative = relative


def git_bytes(repo: Path, *args: str) -> bytes:
    # Git warnings belong to diagnostics, never parsed hashes, paths or blob bytes.
    return subprocess.check_output(
        ["git", "-C", str(repo), *args], stderr=subprocess.PIPE,
    )


def git(repo: Path, *args: str) -> str:
    return git_bytes(repo, *args).decode().strip()


def changes(repo: Path, base: str | None) -> set[str]:
    if not base:
        return set()
    upstream = git(repo, "merge-base", base, "HEAD")
    # Disabling rename folding inventories both paths; do not filter out type changes.
    names = git_bytes(repo, "diff", "--no-renames", "--name-only", "-z",
                      upstream, "HEAD", "--", *SCOPED)
    return {os.fsdecode(name) for name in names.split(b"\0") if name}


def worktree_identity(repo: Path, relative: str) -> tuple[str, str] | None:
    """Read through pinned directory descriptors without following replaced links."""
    rel = Path(relative)
    if rel.is_absolute() or not rel.parts or ".." in rel.parts:
        raise ValueError("invalid repository-relative input path")
    if not all(hasattr(os, flag) for flag in ("O_NOFOLLOW", "O_DIRECTORY", "O_NONBLOCK")):
        raise ValueError("mirror checks require descriptor-relative no-follow file access")

    def version(info):
        return (info.st_dev, info.st_ino, info.st_mode, info.st_size,
                info.st_mtime_ns, info.st_ctime_ns)

    try:
        with ExitStack() as descriptors:
            directory_flags = os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW
            root_fd = os.open(repo, directory_flags)
            descriptors.callback(os.close, root_fd)
            root_info = os.fstat(root_fd)
            parent_fd = root_fd
            parents = []
            for index, part in enumerate(rel.parts[:-1]):
                before = os.stat(part, dir_fd=parent_fd, follow_symlinks=False)
                if not stat.S_ISDIR(before.st_mode):
                    raise NonDirectoryParent(str(Path(*rel.parts[:index + 1])))
                child_fd = os.open(part, directory_flags, dir_fd=parent_fd)
                descriptors.callback(os.close, child_fd)
                opened = os.fstat(child_fd)
                if (before.st_dev, before.st_ino) != (opened.st_dev, opened.st_ino):
                    raise ValueError("proof input parent changed while opening")
                parents.append((parent_fd, part, child_fd))
                parent_fd = child_fd

            name = rel.name
            before = os.stat(name, dir_fd=parent_fd, follow_symlinks=False)
            if stat.S_ISLNK(before.st_mode):
                stored = os.fsencode(os.readlink(name, dir_fd=parent_fd))
                result = "120000", hashlib.sha256(stored).hexdigest()
            elif stat.S_ISREG(before.st_mode):
                # Nonblocking open also prevents a replacement FIFO from hanging the check.
                fd = os.open(name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=parent_fd)
                descriptors.callback(os.close, fd)
                opened = os.fstat(fd)
                if not stat.S_ISREG(opened.st_mode) or version(before) != version(opened):
                    raise ValueError("proof input changed while opening")
                digest = hashlib.sha256()
                while chunk := os.read(fd, 64 * 1024):
                    digest.update(chunk)
                if version(opened) != version(os.fstat(fd)):
                    raise ValueError("proof input changed while reading")
                result = ("100755" if opened.st_mode & stat.S_IXUSR else "100644"), digest.hexdigest()
            else:
                raise ValueError("unsupported file type")

            if version(before) != version(os.stat(name, dir_fd=parent_fd, follow_symlinks=False)):
                raise ValueError("proof input path changed while reading")
            for ancestor_fd, part, child_fd in parents:
                current = os.stat(part, dir_fd=ancestor_fd, follow_symlinks=False)
                opened = os.fstat(child_fd)
                if not stat.S_ISDIR(current.st_mode) or (current.st_dev, current.st_ino) != (opened.st_dev, opened.st_ino):
                    raise ValueError("proof input parent changed while reading")
            current_root = os.stat(repo, follow_symlinks=False)
            if not stat.S_ISDIR(current_root.st_mode) or (current_root.st_dev, current_root.st_ino) != (root_info.st_dev, root_info.st_ino):
                raise ValueError("proof checkout root changed while reading")
            return result
    except FileNotFoundError:
        return None


def committed_identity(repo: Path, relative: str) -> tuple[str, str] | None:
    """Read the exact HEAD mode and stored blob, including literal unusual filenames."""
    records = git_bytes(repo, "ls-tree", "-z", "HEAD", "--", ":(literal)" + relative)
    entries = [entry for entry in records.split(b"\0") if entry]
    if not entries:
        return None
    if len(entries) != 1:
        raise ValueError("proof path does not identify exactly one committed file")
    metadata, name = entries[0].split(b"\t", 1)
    mode, kind, object_id = metadata.split()
    if name != os.fsencode(relative) or kind != b"blob" or mode not in (b"100644", b"100755", b"120000"):
        raise ValueError("proof input is not a supported committed file type")
    blob = git_bytes(repo, "cat-file", "blob", object_id.decode())
    return mode.decode(), hashlib.sha256(blob).hexdigest()


def identity(repo: Path, relative: str) -> tuple[str, str] | None:
    """Require actual kind/bytes to match HEAD; then compare committed identities."""
    committed = committed_identity(repo, relative)
    try:
        actual = worktree_identity(repo, relative)
    except NonDirectoryParent as exc:
        # A committed directory-to-file/link replacement deletes its descendants.
        # Validate that ancestor itself; never follow it or accept an uncommitted
        # substitution merely because the descendant is absent from HEAD.
        if committed is not None or identity(repo, exc.relative) is None:
            raise
        return None
    if actual != committed:
        raise ValueError("working-tree input mode/bytes or uncommitted deletion differs from HEAD")
    return committed


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--peer-repo", required=True, type=Path)
    parser.add_argument("--base-ref", help="Local base ref, e.g. origin/master")
    parser.add_argument("--peer-base-ref", help="Peer base ref, e.g. origin/master")
    args = parser.parse_args()
    peer = args.peer_repo.resolve()
    if not (peer / ".git").exists():
        parser.error("peer is not a Git checkout")
    try:
        other = Path(git(peer, "rev-parse", "--show-toplevel"))
        current = Path(git(ROOT, "rev-parse", "--show-toplevel"))
        changed = changes(ROOT, args.base_ref) | changes(peer, args.peer_base_ref)
    except subprocess.CalledProcessError as exc:
        parser.error(f"cannot establish comparable Git revisions: {exc}")
    if other == current:
        parser.error("refusing to compare a repository to itself")
    paths = sorted(set(SHARED) | changed)
    failures = []
    for rel in paths:
        try:
            left, right = identity(ROOT, rel), identity(peer, rel)
        except (OSError, ValueError, subprocess.CalledProcessError) as exc:
            failures.append((rel, f"cannot compare file: {exc}"))
            continue
        if left is None and right is None and rel not in SHARED:
            print(f"MATCH DELETE {rel}")
            continue
        if left is None or right is None:
            failures.append((rel, "missing on one or both repositories"))
            continue
        if left[0] != right[0]:
            failures.append((rel, f"Git mode mismatch {left[0]} != {right[0]}"))
        elif rel in SHARED and left[0] == "120000":
            failures.append((rel, "required proof input must be a regular file"))
        elif left[1] != right[1]:
            a, b = left[1], right[1]
            failures.append((rel, f"SHA mismatch {a[:12]} != {b[:12]}"))
        else:
            print(f"MATCH {rel} mode={left[0]} sha256={left[1]}")
    if failures:
        for rel, why in failures:
            print(f"DRIFT {rel}: {why}", file=sys.stderr)
        return 1
    print(f"PROOF MIRROR MATCHED {len(paths)} files; {len(changed)} changed language/proof paths checked")
    return 0


if __name__ == "__main__":
    sys.exit(main())
