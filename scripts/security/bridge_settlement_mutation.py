#!/usr/bin/env python3
"""Mutation gate for the issue #42 NEAR-finality-bound settlement condition.

Each mutant changes exactly one source expression of the BSC escrow or the
NEAR contract in a DISPOSABLE COPY, then reruns that component's own test
suite. The real worktree is never edited.

  KILLED    the suite compiled and at least one test failed
  SURVIVED  the suite compiled and every test passed   -> gate fails
  INVALID   the mutant did not apply exactly once or did not compile -> gate fails

Usage (from the repository root):
  python3 scripts/security/bridge_settlement_mutation.py --targets bsc,near
  # offline sandbox with a pre-downloaded solc:
  python3 scripts/security/bridge_settlement_mutation.py --targets bsc \
      --forge-arg=--offline --forge-arg=--use --forge-arg=/path/to/solc-0.8.24
"""
from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time
from dataclasses import asdict, dataclass
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
BSC = ROOT / "x3-lang" / "bridge" / "bsc"
NEAR = ROOT / "x3-lang" / "bridge" / "near"
ESCROW = "src/NearSettledBnbEscrow.sol"
NEAR_LIB = "src/lib.rs"


@dataclass(frozen=True)
class Mutant:
    id: str
    target: str  # "bsc" | "near"
    file: str
    original: str
    mutated: str
    defect: str


MUTANTS = [
    # ---- BSC: the withdrawal condition itself --------------------------------
    Mutant("bsc_settle_skips_verifier", "bsc", ESCROW,
           "        _requireVerified(windowId, outcome, proof);\n", "",
           "settle pays on an unverified outcome"),
    Mutant("bsc_lock_skips_registration", "bsc", ESCROW,
           "        _requireVerified(windowId, OUTCOME_REGISTERED, registrationProof);\n", "",
           "BNB locks against a window never proven to exist"),
    Mutant("bsc_accepts_any_nonzero_return", "bsc", ESCROW,
           "bytes32(ret) == bytes32(NEAR_SETTLEMENT_VERIFIED)", "uint256(bytes32(ret)) != 0",
           "a verifier returning ABI true or dirty magic is trusted"),
    Mutant("bsc_refunded_pays_lp", "bsc", ESCROW,
           "        if (outcome == OUTCOME_CLAIMED) {", "        if (outcome != OUTCOME_NOT_FUNDED) {",
           "NEAR Refunded pays the LP"),
    Mutant("bsc_registered_is_terminal", "bsc", ESCROW,
           "outcome == OUTCOME_CLAIMED || outcome == OUTCOME_REFUNDED || outcome == OUTCOME_NOT_FUNDED",
           "outcome == OUTCOME_REGISTERED || outcome == OUTCOME_CLAIMED || outcome == OUTCOME_REFUNDED"
           " || outcome == OUTCOME_NOT_FUNDED",
           "a registration proof settles the swap"),
    Mutant("bsc_commitment_ignores_outcome", "bsc", ESCROW,
           "abi.encodePacked(SETTLEMENT_DOMAIN, windowId, outcome)",
           "abi.encodePacked(SETTLEMENT_DOMAIN, windowId, OUTCOME_CLAIMED)",
           "any outcome proof is read as Claimed"),
    Mutant("bsc_depositor_not_bound", "bsc", ESCROW,
           "sourceBindingFor(block.chainid, address(this), depositor, t.lp, bnbAmount, t.salt)",
           "sourceBindingFor(block.chainid, address(this), address(0), t.lp, bnbAmount, t.salt)",
           "anyone can lock into another depositor's window"),
    Mutant("bsc_amount_not_bound", "bsc", ESCROW,
           "sourceBindingFor(block.chainid, address(this), depositor, t.lp, bnbAmount, t.salt)",
           "sourceBindingFor(block.chainid, address(this), depositor, t.lp, 1 ether, t.salt)",
           "partial BNB funding matches the registered window"),
    Mutant("bsc_no_reentrancy_guard", "bsc", ESCROW,
           '        require(!entered, "reentrant");\n', "",
           "a payout callback can re-enter settle"),
    # ---- NEAR: what the BSC escrow relies on ---------------------------------
    Mutant("near_window_id_not_checked", "near", NEAR_LIB,
           "        assert_eq!(\n            id,\n            derive_window_id(",
           "        assert_ne!(\n            id.clone() + \"x\",\n            derive_window_id(",
           "a window id can be bound to other terms"),
    Mutant("near_unbound_swap_emits_claimed", "near", NEAR_LIB,
           "        if bound {\n            emit_settlement(&id, SettlementOutcome::Claimed);",
           "        if true {\n            emit_settlement(&id, SettlementOutcome::Claimed);",
           "a direct lock squatting a window id emits Claimed"),
    Mutant("near_claim_logs_refunded", "near", NEAR_LIB,
           "emit_settlement(&id, SettlementOutcome::Claimed);",
           "emit_settlement(&id, SettlementOutcome::Refunded);",
           "a NEAR claim is reported as a refund"),
    Mutant("near_claim_allowed_at_expiry", "near", NEAR_LIB,
           'assert!(env::block_timestamp() < swap.expires_at_ns, "swap expired");',
           'assert!(env::block_timestamp() <= swap.expires_at_ns, "swap expired");',
           "claim and refund both possible at the expiry instant"),
    Mutant("near_refund_initiator_only", "near", NEAR_LIB,
           '        assert!(env::block_timestamp() >= swap.expires_at_ns, "not expired");\n',
           '        assert!(env::block_timestamp() >= swap.expires_at_ns, "not expired");\n'
           '        assert_eq!(swap.initiator, env::predecessor_account_id(), "not initiator");\n',
           "only the LP can drive an expired window to Refunded"),
]


@dataclass
class Outcome:
    id: str
    target: str
    status: str
    detail: str
    seconds: float


def _run(cmd: list[str], cwd: Path, env: dict[str, str], timeout: int) -> tuple[int, str]:
    try:
        proc = subprocess.run(cmd, cwd=cwd, env=env, capture_output=True, text=True, timeout=timeout)
    except subprocess.TimeoutExpired:
        return -1, "TIMEOUT"
    return proc.returncode, proc.stdout + proc.stderr


def _classify(target: str, code: int, output: str) -> tuple[str, str]:
    if output == "TIMEOUT":
        return "INVALID", "timeout"
    if target == "bsc":
        if "Compiler run failed" in output or "Error (" in output:
            return "INVALID", "compile error"
        failed = re.findall(r"\[FAIL[^\]]*\] (\w+)\(", output)
        if code != 0 and failed:
            return "KILLED", ",".join(sorted(set(failed)))
        if code == 0 and "[PASS]" in output:
            return "SURVIVED", "all tests passed"
        return "INVALID", f"exit {code} without test failures"
    if re.search(r"error(\[E\d+\])?: ", output) and "test result" not in output:
        return "INVALID", "compile error"
    failed = re.findall(r"^test (\S+) \.\.\. FAILED", output, re.M) or re.findall(r"^---- (\S+) stdout ----", output, re.M)
    if code != 0 and failed:
        return "KILLED", ",".join(failed)
    if code == 0 and "test result: ok" in output:
        return "SURVIVED", "all tests passed"
    return "INVALID", f"exit {code} without test failures"


def _baseline(target: str, cmd: list[str], src: Path, env: dict[str, str], timeout: int) -> None:
    code, out = _run(cmd, src, env, timeout)
    if code != 0:
        sys.stderr.write(out[-4000:])
        raise SystemExit(f"baseline {target} suite is red; refusing to score mutants")


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--targets", default="bsc,near")
    ap.add_argument("--forge", default=shutil.which("forge") or "forge")
    ap.add_argument("--forge-arg", action="append", default=[])
    ap.add_argument("--cargo", default=shutil.which("cargo") or "cargo")
    ap.add_argument("--timeout", type=int, default=900)
    ap.add_argument("--json", type=Path)
    args = ap.parse_args()
    targets = {t.strip() for t in args.targets.split(",") if t.strip()}
    if not targets <= {"bsc", "near"}:
        raise SystemExit("unknown target")

    env = dict(os.environ)
    results: list[Outcome] = []
    with tempfile.TemporaryDirectory(prefix="x3-settlement-mutation-") as tmp:
        tmp_path = Path(tmp)
        env["CARGO_TARGET_DIR"] = str(tmp_path / "cargo-target")
        commands = {
            "bsc": [args.forge, "test", *args.forge_arg],
            "near": [args.cargo, "test", "--locked"],
        }
        sources = {"bsc": BSC, "near": NEAR}
        for target in sorted(targets):
            pristine = tmp_path / f"{target}-pristine"
            shutil.copytree(sources[target], pristine, ignore=shutil.ignore_patterns("out", "cache", "target"))
            _baseline(target, commands[target], pristine, env, args.timeout)
        for m in MUTANTS:
            if m.target not in targets:
                continue
            start = time.monotonic()
            work = tmp_path / f"{m.id}"
            shutil.copytree(tmp_path / f"{m.target}-pristine", work, ignore=shutil.ignore_patterns("out", "cache"))
            path = work / m.file
            text = path.read_text()
            if text.count(m.original) != 1:
                results.append(Outcome(m.id, m.target, "INVALID",
                                       f"original found {text.count(m.original)} times", 0.0))
                continue
            path.write_text(text.replace(m.original, m.mutated))
            code, out = _run(commands[m.target], work, env, args.timeout)
            status, detail = _classify(m.target, code, out)
            results.append(Outcome(m.id, m.target, status, detail, round(time.monotonic() - start, 1)))
            print(f"{status:9} {m.id:34} {detail}", flush=True)
            shutil.rmtree(work, ignore_errors=True)

    killed = sum(r.status == "KILLED" for r in results)
    print(f"\n{killed}/{len(results)} killed; survived={sum(r.status == 'SURVIVED' for r in results)} "
          f"invalid={sum(r.status == 'INVALID' for r in results)}")
    if args.json:
        args.json.write_text(json.dumps([asdict(r) for r in results], indent=2) + "\n")
    return 0 if results and killed == len(results) else 1


if __name__ == "__main__":
    raise SystemExit(main())
