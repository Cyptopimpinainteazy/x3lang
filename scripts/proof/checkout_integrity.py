"""Bind local proof results to a complete, clean Git checkout before and after execution."""
from __future__ import annotations

from pathlib import Path
import subprocess


class InputIntegrityError(RuntimeError):
    """The checkout cannot support a committed-source proof claim."""


def _git(root: Path, *args: str) -> bytes:
    try:
        return subprocess.check_output(
            ["git", "-c", "core.fsmonitor=false", "-c", "core.untrackedCache=false",
             "-C", str(root), *args], stderr=subprocess.STDOUT,
        )
    except (OSError, subprocess.CalledProcessError) as exc:
        raise InputIntegrityError(f"cannot inspect proof checkout: {exc}") from exc


def snapshot(root: Path) -> dict[str, str]:
    """Reject hidden index entries and every tracked or nonignored untracked change.

    Build outputs covered by the repository's ignore rules are permitted. This is
    a source-state check, not a claim that the host or build environment is hermetic.
    """
    top = Path(_git(root, "rev-parse", "--show-toplevel").decode().strip()).resolve()
    if top != root.resolve():
        raise InputIntegrityError("proof root must be the Git checkout root")
    hidden = [entry[2:].decode(errors="replace") for entry in
              _git(root, "ls-files", "-v", "-z").split(b"\0")
              if entry and (entry[:1] == b"S" or entry[:1].islower())]
    if hidden:
        raise InputIntegrityError(
            "complete checkout required; skip-worktree/assume-unchanged entries: "
            + repr(hidden[:8])
        )
    changes = _git(root, "status", "--porcelain=v1", "-z", "--untracked-files=all",
                   "--ignore-submodules=none")
    if changes:
        paths = [entry.decode(errors="replace") for entry in changes.split(b"\0") if entry]
        raise InputIntegrityError("uncommitted proof checkout inputs: " + repr(paths[:8]))
    return {
        "revision": _git(root, "rev-parse", "HEAD").decode().strip(),
        "tree": _git(root, "rev-parse", "HEAD^{tree}").decode().strip(),
    }


def begin(root: Path, output: Path) -> dict[str, str]:
    if output.resolve().is_relative_to(root.resolve()):
        raise InputIntegrityError("proof output directory must be outside the checkout")
    return snapshot(root)


def finish(root: Path, before: dict[str, str]) -> dict[str, str]:
    after = snapshot(root)
    if after != before:
        raise InputIntegrityError("committed HEAD/tree changed during proof execution")
    return after
