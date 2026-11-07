#!/usr/bin/env python3
"""Reproducible X3/NEAR judge evidence gate.

This is a read-only test harness, not a transaction signer. It deliberately
distinguishes (1) local test results, (2) an authenticated signed quote,
(3) independently *re-read* Solana deposit and NEAR balance facts, and
(4) the *unproven* causal link between the two chains.
An imported receipt's claimed outcome is never upgraded to chain evidence.
"""
from __future__ import annotations

import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import time
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
LANG = ROOT / "x3-lang"


def _git(*parts: str) -> str:
    return subprocess.check_output(
        ["git", *parts], cwd=ROOT, text=True, stderr=subprocess.DEVNULL
    ).strip()


def _step(name: str, argv: list[str], cwd: Path, timeout: int = 300) -> dict:
    return {"name": name, "argv": argv, "cwd": str(cwd), "timeout": timeout}


def plan_tests(mode: str) -> list[dict]:
    """Commands use repo-local paths, with no wallet credentials or network writes."""
    if mode not in ("quick", "full"):
        raise ValueError("unsupported judge test mode")
    python = sys.executable
    node = os.environ.get("NODE") or "node"
    python_files = ["x3-lang/tests"] if mode == "full" else [
        "x3-lang/tests/test_near_mandate.py",
        "x3-lang/tests/test_chain_evidence.py",
        "x3-lang/tests/test_near_intents.py",
        "x3-lang/tests/test_judge_evidence.py",
        "x3-lang/tests/test_near_agent.py",
        "x3-lang/tests/test_value_fee.py",
        "scripts/hackathon/test_testnet_guard_evidence.py",
    ]
    jobs = [
        _step("python_policy_and_chain_evidence_tests",
              [python, "-m", "pytest", "-q", *python_files], ROOT),
        _step("browser_signature_and_guard_tests",
              [node, "--test", "x3-lang/near/demo/test/x3guard.test.mjs"], ROOT),
        _step("allocator_supply_chain_gate",
              ["bash", "scripts/check-near-allocator.sh"], ROOT),
        _step("near_guard_mandate_and_quote_tests",
              ["cargo", "test", "--locked", "--quiet"], LANG / "near" / "x3-guard", 420),
    ]
    if mode == "full":
        jobs.extend([
            _step("rust_vm_tests",
                  ["cargo", "test", "-p", "x3-lang-vm", "--locked", "--quiet"], LANG, 600),
            _step("rust_format",
                  ["cargo", "fmt", "--all", "--", "--check"], LANG),
            _step("rust_vm_clippy",
                  ["cargo", "clippy", "-p", "x3-lang-vm", "--all-targets", "--locked",
                   "--", "-D", "warnings"], LANG, 420),
            _step("near_guard_wasm_compile",
                  ["cargo", "check", "--target", "wasm32-unknown-unknown", "--locked"],
                  LANG / "near" / "x3-guard", 600),
        ])
    return jobs


def run_tests(mode: str) -> list[dict]:
    # Subprocess test runners must never inherit operational wallet or API credentials.
    env = {k: v for k, v in os.environ.items()
           if k in ('PATH', 'HOME', 'LANG', 'LC_ALL', 'TERM', 'TMPDIR',
                    'RUSTUP_HOME', 'CARGO_HOME', 'PYTHONHOME', 'NODE')}
    env['PYTHONPATH'] = str(LANG)
    results = []
    for step in plan_tests(mode):
        started = time.monotonic()
        try:
            run = subprocess.run(
                step["argv"], cwd=step["cwd"], env=env, text=True,
                stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                timeout=step["timeout"], check=False,
            )
            outcome = "passed" if run.returncode == 0 else "failed"
            code = run.returncode
            log_digest = hashlib.sha256(run.stdout.encode("utf-8")).hexdigest()
        except subprocess.TimeoutExpired:
            outcome, code, log_digest = "timeout", None, None
        except FileNotFoundError:
            outcome, code, log_digest = "tool_missing", None, None
        results.append({
            "name": step["name"],
            "status": outcome,
            "exit_code": code,
            "duration_seconds": round(time.monotonic() - started, 2),
            # Hash only: command output can contain file paths, addresses or secrets.
            "output_sha256": log_digest,
        })
        print(f"{step['name']}: {outcome}", file=sys.stderr)
    return results


def inspect_receipt(receipt_path: Path | None, *, refresh_chain: bool) -> dict:
    """Inspect a locally provided receipt without copying potentially sensitive fields.

    Local 'verified' flags are self-reported JSON and carry ZERO proof weight.
    With --refresh-chain, signed quote and receipt-field hashes are checked,
    but chain readers are intentionally NOT invoked: unauthenticated local
    leg/token/baseline fields could select unrelated transfers or balances.
    Live receipts remain unverified until those fields can be independently bound.
    """
    out: dict[str, Any] = {
        "status": "not_supplied" if receipt_path is None else "unverified",
        "claimed_outcome": None,
        "quote_signature_valid": False,
        "receipt_binding_valid": False,
        "live_quote": False,
        "rpc_refresh_requested": bool(refresh_chain),
        "deposit_independently_rechecked": False,
        "delivery_independently_rechecked": False,
        "refund_independently_rechecked": False,
        "cross_chain_causality_proven": False,
        "caveat": "NEAR balance increase does not identify the sender or establish solver causality",
    }
    if receipt_path is None:
        return out
    with open(receipt_path, 'rb') as handle:
        raw = handle.read(5_000_001)
    if len(raw) > 5_000_000:
        raise ValueError("receipt exceeds safe size limit")
    out["receipt_file_sha256"] = hashlib.sha256(raw).hexdigest()
    try:
        receipt = json.loads(raw)
    except (UnicodeDecodeError, json.JSONDecodeError, RecursionError):
        return out
    if not isinstance(receipt, dict) or receipt.get("kind") != "x3.near_intents.v1":
        return out
    claimed = receipt.get("verified_outcome")
    if claimed in ("settled", "refunded"):
        out["claimed_outcome"] = claimed
    response = receipt.get("quote_response")
    if not isinstance(response, dict):
        return out
    sys.path.insert(0, str(LANG))
    import near_intents as near  # already tested signer/chain readers; no new crypto

    valid, _reason = near.check_oneclick_quote_signature(response)
    out["quote_signature_valid"] = bool(valid)
    try:
        bound = (
            near.oneclick_quote_hash(response) == receipt.get("quote_hash")
            and near.signed_quote_payload(response) == receipt.get("signed_payload")
            and near.chain_evidence.sha256_json(response) == receipt.get("quote_response_sha256")
            and isinstance(receipt.get("quote_request"), dict)
            # The dry/live bit controls which quote fields the 1Click signature
            # authenticates. A local receipt must NEVER promote a signed dry
            # quote to a live deposit instruction.
            and receipt["quote_request"].get("dry") is (response.get("quoteRequest") or {}).get("dry")
            and all(
                near._same(k, receipt["quote_request"].get(k),
                           (response.get("quoteRequest") or {}).get(k))
                for k in near._ECHO_FIELDS
            )
        )
    except (ValueError, TypeError, KeyError, AttributeError):
        bound = False
    out["receipt_binding_valid"] = bool(bound)
    request = receipt.get("quote_request")
    signed_request = response.get("quoteRequest")
    out["live_quote"] = (isinstance(request, dict)
                         and isinstance(signed_request, dict)
                         and request.get("dry") is False
                         and signed_request.get("dry") is False)
    if not refresh_chain or not (valid and bound and out["live_quote"]):
        return out

    # SECURITY: A signed 1Click quote authenticates quote terms, not local receipt
    # fields (leg, tokens, settlement_baseline). The chain readers currently use
    # these unauthenticated fields when selecting transfer/balance evidence.
    # Until that binding can be reconstructed independently from signed terms and
    # immutable historical RPC evidence, NEVER elevate this receipt to chain proof.
    out['status'] = 'unverified_receipt_chain_binding'
    out['caveat'] = ('Live quote signature is valid, but the receipt leg, token mapping, '
                     'and historical settlement baseline are not independently authenticated; '
                     'no deposit/delivery/refund verification was attempted')

    return out


def _safe_write(directory: Path, filename: str, contents: str) -> None:
    path = directory / filename
    # Directories and files are private by default; user reviews before publishing.
    with os.fdopen(os.open(str(path), os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600),
                   "w", encoding="utf-8") as handle:
        handle.write(contents)
    os.chmod(path, 0o600)


def _render_markdown(report: dict) -> str:
    rows = [
        "# X3 Shield — Reproducible Judge Evidence",
        "",
        f"Source commit: `{report['git_head']}`",
        f"Worktree changed at inspection: `{report['dirty_worktree']}`",
        f"Generated UTC: {report['generated_utc']}",
        "",
        "## Local executable evidence",
        "",
        "| Gate | Result |",
        "|---|---|",
    ]
    if not report["tests"]:
        rows.append("| Tests | NOT RUN (explicitly skipped) |")
    else:
        rows.extend(f"| {t['name']} | **{t['status']}** |" for t in report["tests"])
    data = report["receipt"]
    rows.extend([
        "",
        "## On-chain evidence — do not conflate statuses",
        "",
        f"- Receipt supplied: {'yes' if data['status'] != 'not_supplied' else 'no'}",
        f"- Live signed quote checks: `{data['quote_signature_valid'] and data['receipt_binding_valid'] and data['live_quote']}`",
        f"- Solana deposit re-read: `{data['deposit_independently_rechecked']}`",
        f"- NEAR recipient delivery re-read: `{data['delivery_independently_rechecked']}`",
        f"- Solana refund re-read: `{data['refund_independently_rechecked']}`",
        f"- Solver-level causal attribution established: **NO**",
        f"- Evidence class: **{data['status']}**",
        "",
        "A displayed status or imported JSON 'verified' field is **not evidence**.",
        "Quote signatures prove signed terms, not that funds moved.",
        "Solana finalized transfer and NEAR balance increases are observations,",
        "not automatically proof that a particular solver linked both transfers.",
        "",
        "See docs/hackathon/JUDGE_GUIDE.md for exact reproduction steps and",
        "PRIOR_WORK.md for development-history disclosure.",
        "",
    ])
    return "\n".join(rows)


def main(argv: list[str] | None = None) -> int:
    p = argparse.ArgumentParser(description="Run tests and classify real NEAR/Solana evidence without signing or paying")
    p.add_argument("--mode", choices=("quick", "full"), default="quick")
    p.add_argument("--skip-tests", action="store_true")
    p.add_argument("--receipt", type=Path, help="optional PRIVATE local X3 1Click receipt")
    p.add_argument("--refresh-chain", action="store_true",
                   help="recheck quote signature and receipt bindings; RPC transaction proof is not yet supported")
    p.add_argument("--refund-tx", help="currently unsupported by judge; explicit use fails closed")
    p.add_argument("--require-live", action="store_true",
                   help="fail closed (exit 5) until independently bound live chain evidence is implemented")
    p.add_argument("--report-dir", type=Path, help="write sanitized evidence.json and evidence.md (private by default)")
    args = p.parse_args(argv)

    if args.refresh_chain and args.receipt is None:
        p.error("--refresh-chain requires --receipt")
    if args.refund_tx is not None:
        p.error("--refund-tx is not supported by this judge gate: source/refund chain binding is unimplemented")

    results = [] if args.skip_tests else run_tests(args.mode)
    try:
        receipt = inspect_receipt(args.receipt, refresh_chain=args.refresh_chain)
    except (OSError, ValueError) as exc:
        print(f"receipt refused: {type(exc).__name__}", file=sys.stderr)
        return 2
    report = {
        "schema": "x3.judge_evidence.v1",
        "generated_utc": datetime.now(timezone.utc).isoformat(),
        "repository": "Cyptopimpinainteazy/x3lang",
        "git_head": _git("rev-parse", "HEAD"),
        "dirty_worktree": bool(_git("status", "--porcelain")),
        "test_mode": "skipped" if args.skip_tests else args.mode,
        "tests": results,
        "receipt": receipt,
        "all_executed_tests_passed": all(t["status"] == "passed" for t in results) if results else False,
        "claimed_onchain_proven": False,
    }
    if args.report_dir:
        args.report_dir.mkdir(parents=True, exist_ok=True, mode=0o700)
        os.chmod(args.report_dir, 0o700)
        _safe_write(args.report_dir, "evidence.json", json.dumps(report, indent=2, sort_keys=True) + "\n")
        _safe_write(args.report_dir, "evidence.md", _render_markdown(report))
    else:
        print(_render_markdown(report))

    if any(t["status"] != "passed" for t in results):
        return 1
    # No imported-receipt chain rechecker currently passes an independent
    # quote-to-transfer binding gate. Do not compare against an unreachable
    # status name or accidentally promote local JSON claims to live evidence.
    if args.require_live:
        return 5
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
