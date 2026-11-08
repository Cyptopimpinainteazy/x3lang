"""Bind local proof runners to committed checkout bytes, without invoking build tools.

This is deliberately a whole-checkout check: Cargo path dependencies, build scripts
and included fixtures need not live beside the selected tests. Git ignore rules and
index flags cannot exempt source. Generated Cargo output and Python/Node environment
caches are excluded only from the untracked-file scan; tracked files are always checked.

This does not attest the toolchain, dependencies, caches or inherited environment,
and requires a trusted checkout with no concurrent writers. Before/after checks are
not a filesystem snapshot and cannot detect an edit that is reverted during a suite.
"""
from __future__ import annotations

import hashlib
import os
from pathlib import Path
import stat
import subprocess


class SourceBindingError(ValueError):
    """The checkout cannot support a proof attributed to the requested revision."""


def _git(root: Path, *args: str) -> bytes:
    try:
        return subprocess.check_output(
            ["git", "--no-replace-objects", "-c", "core.fsmonitor=false",
             "-c", "core.untrackedCache=false", "-C", str(root), *args],
            stderr=subprocess.PIPE,
        )
    except (OSError, subprocess.CalledProcessError) as exc:
        raise SourceBindingError(f"cannot inspect Git checkout ({args[0]})") from exc


def verify_committed_source(
    root: Path, *, output_dir: Path, expected_revision: str | None = None,
) -> str:
    """Return HEAD only if its tracked bytes/modes and index match the checkout.

    Reports must be outside the checkout or in a Cargo target directory. Granting
    an arbitrary --output-dir exemption would let it conceal untracked build input.
    Symlinks and submodules require separate provenance and are refused here.
    """
    root = root.resolve()
    actual_root = Path(os.fsdecode(_git(root, "rev-parse", "--show-toplevel")).strip()).resolve()
    if actual_root != root:
        raise SourceBindingError("runner is not at the Git repository root")
    revision = _git(root, "rev-parse", "--verify", "HEAD^{commit}").decode().strip()
    if expected_revision is not None and revision != expected_revision:
        raise SourceBindingError(f"HEAD changed during proof: {expected_revision} -> {revision}")

    entries = {}
    cargo_outputs = {Path("target")}
    for entry in _git(root, "ls-tree", "-r", "-z", "--full-tree", revision).split(b"\0"):
        if not entry:
            continue
        metadata, raw_path = entry.split(b"\t", 1)
        mode, _, object_id = metadata.decode().split()
        relative = Path(os.fsdecode(raw_path))
        entries[relative] = (mode, object_id)
        if relative.name == "Cargo.toml":
            cargo_outputs.add(relative.parent / "target")

    # Compare the actual index, including intent-to-add and unmerged entries.
    # A plain cached diff can hide intent-to-add files from both itself and the
    # untracked scan, even though Cargo can discover e.g. their new build.rs.
    indexed = set()
    for entry in _git(root, "ls-files", "--stage", "-z").split(b"\0"):
        if not entry:
            continue
        metadata, raw_path = entry.split(b"\t", 1)
        mode, object_id, stage = metadata.decode().split()
        relative = Path(os.fsdecode(raw_path))
        if stage != "0" or entries.get(relative) != (mode, object_id):
            raise SourceBindingError(f"staged input differs from HEAD: {relative}")
        indexed.add(relative)
    missing = entries.keys() - indexed
    if missing:
        raise SourceBindingError(f"tracked input missing from index: {sorted(missing)[0]}")

    output = output_dir.resolve()
    if output.is_relative_to(root) and not any(
        output.is_relative_to(root / directory) for directory in cargo_outputs
    ):
        raise SourceBindingError(
            "--output-dir must be outside the checkout or inside a Cargo target directory"
        )

    # Deliberately omit --exclude-standard: .gitignore and local excludes must not
    # hide a new module, build.rs, Cargo config, manifest or included fixture.
    exclusions = [f"--exclude=/{directory.as_posix()}/" for directory in sorted(cargo_outputs)]
    exclusions += [f"--exclude={name}/" for name in (
        "__pycache__", ".pytest_cache", ".venv", "node_modules",
    )]
    untracked = _git(root, "ls-files", "--others", "-z", *exclusions)
    if untracked:
        relative = os.fsdecode(untracked.split(b"\0", 1)[0])
        raise SourceBindingError(f"untracked input is not bound to HEAD: {relative}")

    algorithm = _git(root, "rev-parse", "--show-object-format").decode().strip()
    for relative, (mode, object_id) in entries.items():
        path = root / relative
        try:
            if mode not in ("100644", "100755"):
                raise SourceBindingError(f"unsupported tracked symlink/submodule: {relative}")
            info = path.lstat()
            if not stat.S_ISREG(info.st_mode) or path.resolve() != path:
                raise SourceBindingError(f"tracked input is not a regular checkout file: {relative}")
            if bool(info.st_mode & stat.S_IXUSR) != (mode == "100755"):
                raise SourceBindingError(f"tracked input mode differs from HEAD: {relative}")
            data = path.read_bytes()
        except OSError as exc:
            raise SourceBindingError(f"cannot read tracked input: {relative}") from exc
        # Compare actual bytes rather than git diff's index stat cache. This also
        # catches assume-unchanged/skip-worktree edits and does not run Git filters.
        actual = hashlib.new(algorithm, b"blob " + str(len(data)).encode() + b"\0" + data).hexdigest()
        if actual != object_id:
            raise SourceBindingError(f"tracked input differs from HEAD: {relative}")
    return revision
