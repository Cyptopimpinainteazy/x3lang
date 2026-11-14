#!/usr/bin/env python3
"""Source-mutation check for the NEAR light-client verifier (issue #56).

Each mutant changes exactly one security-relevant expression in a disposable
copy of x3-lang/bridge and reruns the light-client test modules. The check
fails if the unmutated baseline fails, if any mutant's source pattern is
missing or ambiguous (source drift), or if any mutant survives.

The repository checkout is never modified.
"""
from __future__ import annotations

import json
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

BRIDGE = Path(__file__).resolve().parent
REPO = BRIDGE.parent.parent
CEREMONY = Path("evidence") / "near-light-client-ceremony-2026-11-06"
TEST_MODULES = (
    "test_near_light_client_vectors",
    "test_near_authenticated",
    "test_near_outcome",
    "test_near_merkle",
    "test_near_validator_commitment",
)

# (id, file, exact original text, replacement, defect it models)
MUTANTS = (
    (
        "threshold_at_two_thirds",
        "near_authenticated.py",
        "if approved_stake * 3 <= total_stake * 2:",
        "if approved_stake * 3 < total_stake * 2:",
        "accept exactly 2/3 of stake",
    ),
    (
        "partial_stake_denominator",
        "near_authenticated.py",
        "        signature = approvals[index] if index < len(approvals) else None\n"
        "        if signature is None:\n"
        "            continue\n",
        "        if index >= len(approvals):\n"
        "            total_stake -= stake\n"
        "            continue\n"
        "        signature = approvals[index]\n"
        "        if signature is None:\n"
        "            continue\n",
        "count only producers that have an approval slot (zip denominator)",
    ),
    (
        "approval_height_plus_one",
        "near_authenticated.py",
        '(height + 2).to_bytes(8, "little")',
        '(height + 1).to_bytes(8, "little")',
        "reconstruct the approval message for the wrong target height",
    ),
    (
        "skip_signature_check",
        "near_authenticated.py",
        "            public_key.verify(\n"
        '                _signature_bytes(signature, "validator approval"), message\n'
        "            )",
        '            _signature_bytes(signature, "validator approval")',
        "count stake for any well-formed signature",
    ),
    (
        "same_epoch_uses_next_set",
        "near_authenticated.py",
        '        validators = state["current_validators"]\n        if validators is None:',
        '        validators = state["next_validators"]\n        if validators is None:',
        "approve a same-epoch block with the next epoch's producers",
    ),
    (
        "same_epoch_bp_hash_unchecked",
        "near_authenticated.py",
        '        if new["next_bp_hash"] != old["next_bp_hash"]:',
        "        if False:",
        "accept a conflicting next validator commitment within one epoch",
    ),
    (
        "timestamp_unchecked",
        "near_authenticated.py",
        '    if new["timestamp"] <= old["timestamp"]:',
        "    if False:",
        "accept a successor whose timestamp does not advance",
    ),
    (
        "epoch_skip_allowed",
        "near_authenticated.py",
        '    raise AuthenticatedLightClientError("candidate epoch does not succeed authenticated head")',
        '    return state["next_validators"]',
        "accept a candidate from an epoch that does not succeed the head",
    ),
    (
        "genesis_unpinned",
        "near_authenticated.py",
        "    if genesis_hash != NETWORK_GENESIS[network]:",
        "    if False:",
        "accept a network name paired with another network's genesis",
    ),
    (
        "inner_lite_encoding_unchecked",
        "near_merkle.py",
        "    check_inner_lite_encoding(inner)\n    height",
        "    height",
        "hash a header while ignoring unauthenticated or V2 inner-lite fields",
    ),
    (
        "shard_root_not_rehashed",
        "near_outcome.py",
        "compute_merkle_root(hashlib.sha256(shard_root).digest(), root_path)",
        "compute_merkle_root(shard_root, root_path)",
        "skip the sha256(shard outcome root) leaf of the block outcome tree",
    ),
    (
        "failure_encoded_as_success",
        "near_outcome.py",
        "        return STATUS_FAILURE, b'\\x01'",
        "        return STATUS_FAILURE, b'\\x02'",
        "encode PartialExecutionStatus::Failure with another variant tag",
    ),
    (
        "shared_keys_rejected",
        "near_validator_commitment.py",
        "        key_raw = decode_hash(key.removeprefix('ed25519:'))\n",
        "        key_raw = decode_hash(key.removeprefix('ed25519:'))\n"
        "        if any(v.get('public_key') == key"
        " for v in validators[:validators.index(validator)]):\n"
        "            raise ValueError('duplicate validator key')\n",
        "refuse real mainnet validator sets in which two accounts share a key",
    ),
)


def _run_tests(bridge: Path) -> tuple[bool, str]:
    proc = subprocess.run(
        [sys.executable, "-m", "unittest", *TEST_MODULES],
        cwd=bridge,
        capture_output=True,
        text=True,
        env={"PYTHONDONTWRITEBYTECODE": "1", "PATH": "/usr/bin:/bin"},
        timeout=600,
    )
    summary = [line for line in proc.stderr.splitlines() if line.startswith(("OK", "FAILED"))]
    return proc.returncode == 0, summary[-1] if summary else f"exit {proc.returncode}"


def _sandbox(root: Path) -> Path:
    bridge = root / "x3-lang" / "bridge"
    shutil.copytree(
        BRIDGE, bridge, ignore=shutil.ignore_patterns("__pycache__", "target", "out", "cache")
    )
    shutil.copytree(REPO / CEREMONY, root / CEREMONY)
    return bridge


def main() -> int:
    report = {"baseline": None, "mutants": []}
    with tempfile.TemporaryDirectory(prefix="x3-lc-baseline-") as tmp:
        passed, summary = _run_tests(_sandbox(Path(tmp)))
    report["baseline"] = {"passed": passed, "summary": summary}
    if not passed:
        print(json.dumps(report, indent=2))
        print("baseline failed; mutation results would be meaningless", file=sys.stderr)
        return 1
    failures = 0
    for mutant_id, filename, original, replacement, defect in MUTANTS:
        with tempfile.TemporaryDirectory(prefix=f"x3-lc-{mutant_id}-") as tmp:
            bridge = _sandbox(Path(tmp))
            target = bridge / filename
            source = target.read_text()
            occurrences = source.count(original)
            if occurrences != 1:
                failures += 1
                report["mutants"].append(
                    {"id": mutant_id, "result": "SOURCE_DRIFT", "occurrences": occurrences}
                )
                continue
            target.write_text(source.replace(original, replacement))
            passed, summary = _run_tests(bridge)
        result = "SURVIVED" if passed else "KILLED"
        failures += passed
        report["mutants"].append(
            {"id": mutant_id, "file": filename, "defect": defect, "result": result, "tests": summary}
        )
    print(json.dumps(report, indent=2))
    killed = sum(item["result"] == "KILLED" for item in report["mutants"])
    print(f"{killed}/{len(MUTANTS)} light-client mutants killed", file=sys.stderr)
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
