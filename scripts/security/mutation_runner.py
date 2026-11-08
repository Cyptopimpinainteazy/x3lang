#!/usr/bin/env python3
"""
X3 Shield mutation testing runner for mandate verification.

Mutates critical security checks in near_mandate.py and runs the existing test suite
to verify each mutation is caught (KILLED) or escapes (SURVIVED). Generates JSON and
Markdown evidence.
"""

import argparse
import hashlib
import json
import logging
import os
import shutil
import subprocess
import sys
import tempfile
import time
from dataclasses import dataclass, asdict
from pathlib import Path
from typing import List, Optional, Tuple

logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s [%(levelname)s] %(message)s"
)
log = logging.getLogger(__name__)


@dataclass
class Mutation:
    """A single code mutation."""
    id: str
    name: str
    description: str
    file: str
    line: int
    original: str
    mutated: str
    kind: str  # bypass | boundary
    severity: str = "P1"


@dataclass
class MutationResult:
    """Result of running tests against a mutation."""
    mutation_id: str
    mutation_name: str
    status: str  # KILLED, SURVIVED, INVALID, TIMEOUT, NOT_RUN
    test_exit_code: Optional[int]
    test_stdout: str
    test_stderr: str
    failed_tests: List[str]
    killed_by_tests: List[str]
    duration_seconds: float
    error_detail: Optional[str]
    source_sha256: Optional[str] = None
    mutated_sha256: Optional[str] = None


def get_mutations() -> List[Mutation]:
    """Security-relevant single-defect mutants: authorization bypasses + expiry boundary.

    Mutations are applied only in disposable copies. Each replacement matches a
    unique expression at a pinned line. The original production tree is untouched.
    """
    specs = [
        # id, threat, line, source expression, replacement, severity, kind
        ("recipient_bypass", "Allow forbidden recipient", 317,
         'policy["recipient"] in t["recipients"]', "True", "P0", "bypass"),
        ("origin_asset_bypass", "Allow forbidden origin asset", 313,
         'policy["origin_asset"] == t["asset"]', "True", "P0", "bypass"),
        ("destination_asset_bypass", "Allow forbidden destination asset", 315,
         'policy["destination_asset"] in t["destination_assets"]', "True", "P0", "bypass"),
        ("refund_bypass", "Allow unapproved refund destination", 318,
         'policy["refund_to"] == t["refund_to"]', "True", "P0", "bypass"),
        ("payment_cap_bypass", "Ignore per-payment cap", 321,
         'amount <= int(t["per_payment_cap"])', "True", "P0", "bypass"),
        ("daily_cap_bypass", "Ignore 24-hour cap and count", 323,
         'spent + amount <= int(t["daily_cap"]) and count < MAX_PAYMENTS_PER_WINDOW',
         "True", "P0", "bypass"),
        ("slippage_cap_bypass", "Ignore maximum slippage", 325,
         'int(policy["slippage_bps"]) <= int(t["max_slippage_bps"])', "True", "P0", "bypass"),
        ("fee_cap_bypass", "Ignore maximum fee", 328,
         'fee_cap is not None and int(fee_cap) <= int(t["max_fee_bps"])', "True", "P0", "bypass"),
        ("deadline_bypass", "Ignore intent deadline restriction", 330,
         'int(policy["deadline_ms"]) <= int(t["expires_at_ms"])', "True", "P0", "bypass"),
        ("mandate_active_bypass", "Ignore revocation and expiry", 311,
         'not revoked and now_ms < int(t["expires_at_ms"])', "True", "P0", "bypass"),
        ("approval_bypass", "Disable additional approval requirement", 337,
         'return int(mandate["terms"]["approval_threshold"]) if above is not None and amount > int(above) else 0',
         "return 0", "P0", "bypass"),
        ("expiry_one_ms_late", "Permit mandates at exact expiry instant", 311,
         'now_ms < int(t["expires_at_ms"])',
         'now_ms <= int(t["expires_at_ms"])', "P1", "boundary"),
    ]
    return [
        Mutation(id=ident, name=threat, description=threat,
                 file="near_mandate.py", line=line, original=original,
                 mutated=mutated, kind=kind, severity=severity)
        for ident, threat, line, original, mutated, severity, kind in specs
    ]


def apply_mutation(source_code: str, mutation: Mutation) -> str:
    """Apply a mutation to source code. Raises ValueError if original not found."""
    if source_code.count(mutation.original) != 1:
        raise ValueError(
            f"Expected exactly one occurrence for {mutation.id}, found "
            f"{source_code.count(mutation.original)}"
        )
    lines = source_code.split('\n')
    # Line numbers are 1-indexed in the mutation, but 0-indexed in the array
    line_idx = mutation.line - 1

    if line_idx < 0 or line_idx >= len(lines):
        raise ValueError(f"Line {mutation.line} out of range (file has {len(lines)} lines)")

    # Find the exact location of the mutation within the line
    line = lines[line_idx]
    if mutation.original not in line:
        raise ValueError(
            f"Could not find mutation pattern on line {mutation.line}.\n"
            f"Expected: {mutation.original!r}\n"
            f"Found: {line!r}"
        )

    if line.count(mutation.original) != 1:
        raise ValueError(f"Ambiguous occurrence on line {mutation.line}")
    lines[line_idx] = line.replace(mutation.original, mutation.mutated, 1)
    return '\n'.join(lines)


def run_tests_in_copy(
    repo_root: Path,
    mutated_code: str,
    mutation: Mutation,
    timeout_seconds: int = 60
) -> MutationResult:
    """Run tests against mutated code in an isolated temp directory."""
    start_time = time.time()

    try:
        with tempfile.TemporaryDirectory(prefix="x3_mutant_") as tmpdir:
            tmpdir = Path(tmpdir)

            # Copy the entire x3-lang tree
            src_tree = repo_root / "x3-lang"
            dst_tree = tmpdir / "x3-lang"
            shutil.copytree(src_tree, dst_tree, ignore=shutil.ignore_patterns("target", ".venv", "__pycache__", ".pytest_cache"))

            # Write the mutated file
            target_file = dst_tree / mutation.file
            if not target_file.exists():
                raise ValueError(f"Target file does not exist: {target_file}")
            target_file.write_text(mutated_code)

            # Run the test suite (both mandate and boundary tests)
            env = {**os.environ, "PYTHONPATH": str(dst_tree), "PYTHONDONTWRITEBYTECODE": "1"}
            result = subprocess.run(
                [sys.executable, "-m", "pytest", "tests/test_near_mandate.py", "tests/test_near_mandate_boundaries.py", "-q", "--tb=short"],
                cwd=dst_tree,
                env=env,
                capture_output=True,
                text=True,
                timeout=timeout_seconds
            )

            duration = time.time() - start_time

            # Parse results
            stdout = result.stdout
            stderr = result.stderr
            exit_code = result.returncode

            # Extract failed test names from output
            failed_tests = [
                line.strip() for line in stdout.splitlines()
                if line.startswith("FAILED ")
            ]
            collection_errors = any(
                line.startswith("ERROR ") for line in stdout.splitlines()
            )
            assertion_detected = (
                "AssertionError" in stdout or "E       assert " in stdout
                or "Failed: DID NOT RAISE" in stdout
            )

            # Exit code 2 (pytest infrastructure/collection error) and arbitrary
            # nonzero exits are never counted as killed. Only a test assertion
            # finding the mutated behavior counts as KILLED.
            if exit_code == 0:
                status, killed_by = "SURVIVED", []
            elif exit_code == 1 and failed_tests and not collection_errors and assertion_detected:
                status, killed_by = "KILLED", failed_tests
            else:
                status, killed_by = "INVALID", []

            return MutationResult(
                mutation_id=mutation.id,
                mutation_name=mutation.name,
                status=status,
                test_exit_code=exit_code,
                test_stdout=stdout,
                test_stderr=stderr,
                failed_tests=failed_tests,
                killed_by_tests=killed_by,
                duration_seconds=duration,
                error_detail=None if status != "INVALID" else f"Unrelated pytest failure (exit={exit_code})"
            )

    except subprocess.TimeoutExpired:
        duration = time.time() - start_time
        return MutationResult(
            mutation_id=mutation.id,
            mutation_name=mutation.name,
            status="TIMEOUT",
            test_exit_code=None,
            test_stdout="",
            test_stderr="Tests exceeded timeout",
            failed_tests=[],
            killed_by_tests=[],
            duration_seconds=duration,
            error_detail=f"Timeout after {timeout_seconds}s"
        )

    except Exception as e:
        duration = time.time() - start_time
        return MutationResult(
            mutation_id=mutation.id,
            mutation_name=mutation.name,
            status="INVALID",
            test_exit_code=None,
            test_stdout="",
            test_stderr=str(e),
            failed_tests=[],
            killed_by_tests=[],
            duration_seconds=duration,
            error_detail=str(e)
        )


def run_mutation_tests(
    repo_root: Path,
    mutations: Optional[List[str]] = None,
    output_dir: Optional[Path] = None,
    timeout_seconds: int = 60
) -> Tuple[List[MutationResult], dict]:
    """Run mutation tests and return results with summary statistics."""

    all_mutations = get_mutations()

    # Filter mutations if specified
    if mutations:
        all_mutations = [m for m in all_mutations if m.id in mutations]
        unknown = set(mutations) - {m.id for m in get_mutations()}
        if unknown:
            raise ValueError(f"Unknown mutation IDs: {sorted(unknown)}")
        if not all_mutations:
            raise ValueError("No mutations selected")

    log.info(f"Starting mutation tests with {len(all_mutations)} mutations")

    # Load the original source
    original_file = repo_root / "x3-lang" / "near_mandate.py"
    if not original_file.exists():
        raise FileNotFoundError(f"Source file not found: {original_file}")
    original_code = original_file.read_text()

    # Verify green baseline first
    log.info("Verifying green baseline with original code...")
    baseline_result = run_tests_in_copy(repo_root, original_code, all_mutations[0])
    if baseline_result.status != "SURVIVED":
        raise RuntimeError(
            "Fail-closed: unmutated baseline must pass before mutation score "
            f"can be calculated; got {baseline_result.status}: "
            f"{baseline_result.error_detail}"
        )

    results = []
    killed_count = 0
    survived_count = 0
    invalid_count = 0
    timeout_count = 0

    for i, mutation in enumerate(all_mutations, 1):
        log.info(f"[{i}/{len(all_mutations)}] Testing {mutation.id}: {mutation.name}")

        try:
            # Apply mutation
            mutated_code = apply_mutation(original_code, mutation)

            # Run tests
            result = run_tests_in_copy(repo_root, mutated_code, mutation, timeout_seconds)
            result.source_sha256 = hashlib.sha256(original_code.encode()).hexdigest()
            result.mutated_sha256 = hashlib.sha256(mutated_code.encode()).hexdigest()
            results.append(result)

            # Track statistics
            if result.status == "KILLED":
                killed_count += 1
                log.info(f"  ✓ KILLED by {len(result.killed_by_tests)} test(s)")
            elif result.status == "SURVIVED":
                survived_count += 1
                log.warning(f"  ✗ SURVIVED - mutation not caught by tests!")
            elif result.status == "TIMEOUT":
                timeout_count += 1
                log.warning(f"  ⚠ TIMEOUT: {result.error_detail}")
            else:
                invalid_count += 1
                log.warning(f"  ⚠ INVALID: {result.error_detail}")

        except ValueError as e:
            log.error(f"  ✗ INVALID: {e}")
            invalid_count += 1
            results.append(MutationResult(
                mutation_id=mutation.id,
                mutation_name=mutation.name,
                status="INVALID",
                test_exit_code=None,
                test_stdout="",
                test_stderr=str(e),
                failed_tests=[],
                killed_by_tests=[],
                duration_seconds=0,
                error_detail=str(e)
            ))

    # Calculate statistics
    runnable = killed_count + survived_count
    score = killed_count / runnable * 100 if runnable > 0 else 0

    summary = {
        "total_mutations": len(results),
        "killed": killed_count,
        "survived": survived_count,
        "invalid": invalid_count,
        "timeouts": timeout_count,
        "runnable": runnable,
        "mutation_score": score,
        "baseline_status": "PASS",
        "timestamp": time.strftime("%Y-%m-%d %H:%M:%S UTC", time.gmtime()),
        "git_head": get_git_head(repo_root),
        "test_command": "PYTHONPATH=x3-lang python3 -m pytest x3-lang/tests/test_near_mandate.py x3-lang/tests/test_near_mandate_boundaries.py -q",
    }

    return results, summary


def get_git_head(repo_root: Path) -> str:
    """Get current git HEAD SHA."""
    try:
        result = subprocess.run(
            ["git", "rev-parse", "HEAD"],
            cwd=repo_root,
            capture_output=True,
            text=True,
            timeout=5
        )
        return result.stdout.strip() if result.returncode == 0 else "unknown"
    except:
        return "unknown"


def save_results(results: List[MutationResult], summary: dict, output_dir: Path) -> None:
    """Save results to JSON and Markdown files."""
    output_dir.mkdir(parents=True, exist_ok=True)

    # Save JSON
    json_data = {
        "summary": summary,
        "results": [asdict(r) for r in results],
    }
    json_file = output_dir / "mutation_results.json"
    json_file.write_text(json.dumps(json_data, indent=2))
    log.info(f"Saved JSON results to {json_file}")

    # Save Markdown
    md = gen_markdown_report(results, summary)
    md_file = output_dir / "MUTATION_RESULTS.md"
    md_file.write_text(md)
    log.info(f"Saved Markdown report to {md_file}")


def gen_markdown_report(results: List[MutationResult], summary: dict) -> str:
    """Generate a Markdown report of mutation test results."""

    killed = [r for r in results if r.status == "KILLED"]
    survived = [r for r in results if r.status == "SURVIVED"]
    invalid = [r for r in results if r.status == "INVALID"]
    timed_out = [r for r in results if r.status == "TIMEOUT"]

    md = f"""# X3 Shield Mutation Testing Results

**Date:** {summary['timestamp']}
**Git HEAD:** `{summary['git_head']}`
**Baseline Status:** {summary['baseline_status']}

## Summary

| Metric | Value |
|--------|-------|
| Total Mutations | {summary['total_mutations']} |
| Killed | {summary['killed']} |
| Survived | {summary['survived']} |
| Invalid | {summary['invalid']} |
| Timed out | {summary['timeouts']} |
| Runnable | {summary['runnable']} |
| **Mutation Score** | **{summary['mutation_score']:.1f}%** |

## Test Command

```bash
{summary['test_command']}
```

## Mutations Killed

Tests successfully caught these mutations:

"""

    for r in killed:
        md += f"- **{r.mutation_name}** (`{r.mutation_id}`): Killed by {len(r.killed_by_tests)} test(s) in {r.duration_seconds:.2f}s\n"

    if survived:
        md += "\n## ⚠️ Mutations Survived (Security Risk)\n\nThese mutations were **NOT** caught by tests:\n\n"
        for r in survived:
            md += f"- **{r.mutation_name}** (`{r.mutation_id}`): Tests passed despite mutation!\n"

    if timed_out:
        md += "\n## Timed-out Mutations (Gate Blockers)\n\n"
        for r in timed_out:
            md += f"- {r.mutation_name}: {r.error_detail}\n"

    if invalid:
        md += "\n## Invalid Mutations\n\nThese mutations could not be properly applied or tested:\n\n"
        for r in invalid:
            md += f"- **{r.mutation_name}** (`{r.mutation_id}`): {r.error_detail}\n"

    md += f"""

## Detailed Results

"""

    for r in results:
        status_icon = "✓" if r.status == "KILLED" else "✗" if r.status == "SURVIVED" else "⚠"
        md += f"""### {status_icon} {r.mutation_name}

**Status:** `{r.status}`
**Duration:** {r.duration_seconds:.2f}s
**Exit Code:** {r.test_exit_code}

"""
        if r.error_detail:
            md += f"**Error:** {r.error_detail}\n\n"
        if r.killed_by_tests:
            md += f"**Killed by:**\n"
            for test in r.killed_by_tests[:5]:
                md += f"- {test}\n"
            md += "\n"

    return md


def main():
    parser = argparse.ArgumentParser(
        description="X3 Shield mutation testing gate for mandate verification"
    )
    parser.add_argument(
        "--repo-root",
        type=Path,
        default=Path.cwd(),
        help="Repository root directory (default: current directory)"
    )
    parser.add_argument(
        "--output-dir",
        type=Path,
        default=None,
        help="Output directory for results (default: docs/security/mutation_results)"
    )
    parser.add_argument(
        "--mutations",
        type=str,
        help="Comma-separated list of mutation IDs to test (default: all)"
    )
    parser.add_argument(
        "--timeout",
        type=int,
        default=60,
        help="Timeout per mutation test in seconds (default: 60)"
    )

    args = parser.parse_args()

    # Resolve output directory
    if args.output_dir is None:
        args.output_dir = args.repo_root / "docs" / "security" / "mutation_results"

    # Parse mutation IDs
    mutations = None
    if args.mutations:
        mutations = args.mutations.split(",")

    try:
        # Run tests
        results, summary = run_mutation_tests(
            args.repo_root,
            mutations=mutations,
            output_dir=args.output_dir,
            timeout_seconds=args.timeout
        )

        # Save results
        save_results(results, summary, args.output_dir)

        # Print summary
        print(f"\n{'='*70}")
        print(f"Mutation Testing Complete")
        print(f"{'='*70}")
        print(f"Total: {summary['total_mutations']} | Killed: {summary['killed']} | Survived: {summary['survived']} | Invalid: {summary['invalid']}")
        print(f"Mutation Score: {summary['mutation_score']:.1f}%")
        print(f"Results saved to: {args.output_dir}")

        # Exit with error if there are survivors (P0 gate)
        if summary['survived'] > 0:
            print(f"\n⚠️  GATE FAILURE: {summary['survived']} mutation(s) survived!")
            return 1

        if summary['invalid'] > 0 or summary['timeouts'] > 0:
            print(
                f"\nGATE FAILURE: {summary['invalid']} INVALID and "
                f"{summary['timeouts']} TIMEOUT mutations"
            )
            return 1

        return 0

    except Exception as e:
        log.error(f"Fatal error: {e}", exc_info=True)
        return 1


if __name__ == "__main__":
    sys.exit(main())
