"""Offline shell-level regressions: a reported outcome must never skip proof gates.

The test intercepts adapter and guard commands; it does not sign or send.
"""
import json
import os
import subprocess
from pathlib import Path

import pytest


SCRIPT = Path(__file__).resolve().parents[1] / "near" / "agent-demo.sh"
SIGNATURE = "5" * 88
REFUND_SIGNATURE = "6" * 88


def run_deposit(tmp_path, *, status, proof, deposit=0, refund=REFUND_SIGNATURE):
    script = SCRIPT.read_text(encoding="utf-8")
    assert '\ncase "${1:-}" in' in script
    setup = tmp_path / "functions.sh"
    setup.write_text(script.split('\ncase "${1:-}" in', 1)[0], encoding="utf-8")
    receipt = tmp_path / "receipt.json"
    receipt.write_text(json.dumps({"deposit": {"address": "test-deposit-address"}}), encoding="utf-8")
    trace = tmp_path / "trace"
    # Never execute the actual network adapters, owner keys or guard transaction.
    shell = r"""
source "$X3_TEST_LIBRARY"
RECEIPT="$X3_TEST_RECEIPT"
load_state() { :; }
adapter() {
  printf '%s\n' "$1" >> "$X3_TEST_TRACE"
  case "$1" in
    status) return "$X3_TEST_STATUS" ;;
    verify-deposit) return "$X3_TEST_DEPOSIT" ;;
    verify-settlement)
      if [[ "$X3_TEST_STATUS" == 2 ]] && [[ " $* " != *" --refund-tx "* ]]; then
        return 5
      fi
      return "$X3_TEST_PROOF"
      ;;
  esac
}
confirm() { printf 'CONFIRM\n' >> "$X3_TEST_TRACE"; }
guard_cmd() { printf "printf 'GUARD\\n' >> '%s'\n" "$X3_TEST_TRACE"; }
cmd_deposit "$X3_TEST_SIGNATURE" "$X3_TEST_REFUND"
"""
    env = os.environ | {
        "X3_TEST_LIBRARY": str(setup),
        "X3_TEST_RECEIPT": str(receipt),
        "X3_TEST_TRACE": str(trace),
        "X3_TEST_STATUS": str(status),
        "X3_TEST_DEPOSIT": str(deposit),
        "X3_TEST_PROOF": str(proof),
        "X3_TEST_SIGNATURE": SIGNATURE,
        "X3_TEST_REFUND": refund,
        "CONTRACT": "test-contract.near",
        "X3_AGENT_RUN_DIR": str(tmp_path),
    }
    result = subprocess.run(["bash", "-c", shell], env=env, text=True,
                            capture_output=True, timeout=10, check=False, input="")
    entries = trace.read_text().splitlines() if trace.exists() else []
    return result, entries


@pytest.mark.parametrize(("status", "proof"), [(0, 0), (2, 2)])
def test_terminal_outcome_records_only_with_matching_chain_proof(tmp_path, status, proof):
    result, entries = run_deposit(tmp_path, status=status, proof=proof)
    assert result.returncode == 0, result.stderr
    assert entries == ["submit-deposit", "status", "verify-deposit",
                       "verify-settlement", "CONFIRM", "GUARD"]


@pytest.mark.parametrize(("status", "proof", "deposit"), [
    (3, 0, 0), (4, 0, 0), (0, 2, 0), (2, 0, 0),
    (0, 5, 0), (2, 5, 0), (0, 0, 5), (2, 2, 5),
])
def test_no_outcome_transaction_for_failed_unverified_or_mismatched_proof(
        tmp_path, status, proof, deposit):
    result, entries = run_deposit(tmp_path, status=status, proof=proof, deposit=deposit)
    assert result.returncode != 0
    assert "CONFIRM" not in entries
    assert "GUARD" not in entries


def test_refund_without_signature_never_records_outcome(tmp_path):
    result, entries = run_deposit(tmp_path, status=2, proof=2, refund="")
    assert result.returncode != 0
    assert "verify-settlement" not in entries
    assert "GUARD" not in entries
