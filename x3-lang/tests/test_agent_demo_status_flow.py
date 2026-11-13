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


def run_deposit(tmp_path, *, status, proof, deposit=0, refund=REFUND_SIGNATURE, first_swap=False, recorded=None):
    script_path = SCRIPT.with_name("first-swap.sh") if first_swap else SCRIPT
    script = script_path.read_text(encoding="utf-8")
    assert '\ncase "${1:-}" in' in script
    setup = tmp_path / "functions.sh"
    setup.write_text(script.split('\ncase "${1:-}" in', 1)[0], encoding="utf-8")
    receipt = tmp_path / "receipt.json"
    receipt.write_text(json.dumps({"deposit": {"address": "test-deposit-address", "tx_hash": recorded}}),
                       encoding="utf-8")
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
"$X3_TEST_ENTRYPOINT" "$X3_TEST_SIGNATURE" "$X3_TEST_REFUND"
"""
    env = os.environ | {
        "X3_TEST_LIBRARY": str(setup),
        "X3_TEST_RECEIPT": str(receipt),
        "X3_TEST_TRACE": str(trace),
        "X3_TEST_STATUS": str(status),
        "X3_TEST_ENTRYPOINT": "cmd_deposited" if first_swap else "cmd_deposit",
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


@pytest.mark.parametrize(("status", "proof"), [(2, 2)])
def test_terminal_outcome_records_only_with_matching_chain_proof(tmp_path, status, proof):
    result, entries = run_deposit(tmp_path, status=status, proof=proof)
    assert result.returncode == 0, result.stderr
    assert entries == ["submit-deposit", "status", "verify-deposit",
                       "verify-settlement", "CONFIRM", "GUARD"]


@pytest.mark.parametrize(("status", "proof", "deposit"), [
    (0, 0, 0), (3, 0, 0), (4, 0, 0), (0, 2, 0), (2, 0, 0),
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


def test_first_swap_never_records_provider_success_even_with_legacy_zero_exit(tmp_path):
    result, entries = run_deposit(tmp_path, status=0, proof=0, first_swap=True)
    assert result.returncode != 0
    assert "Payout transaction verification is not implemented" in result.stderr
    assert "CONFIRM" not in entries
    assert "GUARD" not in entries


def test_pending_deposit_resumes_without_notifying_again(tmp_path):
    result, entries = run_deposit(tmp_path, status=4, proof=5, recorded=SIGNATURE)
    assert result.returncode != 0
    assert "Do not deposit again" in result.stderr
    assert entries == ["status"]


def test_different_signature_cannot_replace_recorded_deposit(tmp_path):
    result, entries = run_deposit(tmp_path, status=0, proof=0, recorded=REFUND_SIGNATURE)
    assert result.returncode != 0
    assert "different signature" in result.stderr
    assert entries == []


@pytest.mark.parametrize("mode", ["0", "1"])
@pytest.mark.parametrize("entrypoint", ["register", "quote", "outcome", "cmd_refuse", "cmd_post_revoke",
                                        "cmd_pay_dry", "cmd_pay_live"])
def test_prepaid_storage_mode_reaches_every_agent_command(tmp_path, mode, entrypoint):
    library = tmp_path / "functions.sh"
    library.write_text(SCRIPT.read_text().split('\ncase "${1:-}" in', 1)[0])
    trace = tmp_path / "trace"
    shell = r'''
source "$X3_TEST_LIBRARY"
load_state() { :; }
confirm() { :; }
send_expected_refusal() { :; }
adapter() { printf '%s\n' "$*" >> "$X3_TEST_TRACE"; printf '{"command":"true"}\n'; }
agent() {
  printf '%s\n' "$*" >> "$X3_TEST_TRACE"
  if [[ " $* " == *" --live "* ]]; then printf '{}\n' > "$RECEIPT"; fi
  printf '{"command":"true"}\n'
}
json_get() { if [[ "$2" == command ]]; then printf 'true\n'; else printf '%064d\n' 0; fi; }
printf '{}\n' > "$ALLOWED"
printf '{}\n' > "$DRY_REPORT"
case "$X3_TEST_ENTRYPOINT" in
  register|quote|outcome) guard_cmd "$X3_TEST_ENTRYPOINT" ;;
  cmd_pay_live) cmd_pay_live "$(printf '%064d' 0)" ;;
  *) "$X3_TEST_ENTRYPOINT" ;;
esac
'''
    env = os.environ | {
        "X3_TEST_LIBRARY": str(library), "X3_TEST_TRACE": str(trace),
        "X3_TEST_ENTRYPOINT": entrypoint, "X3_AGENT_RUN_DIR": str(tmp_path),
        "X3_AGENT_PREPAID_STORAGE": mode, "CONTRACT": "guard.sandbox", "AGENT_ACCOUNT": "agent.sandbox",
        "AGENT_KEY": "fixture-only.json", "RECIPIENT": "recipient.sandbox", "AMOUNT": "5",
        "UNITS": "5000000", "SOLANA": "test-refund", "MIN_RECEIVE": "4900000", "OWNER": "owner.sandbox",
    }
    result = subprocess.run(["bash", "-c", shell], env=env, capture_output=True, text=True, timeout=10)
    assert result.returncode == 0, result.stderr
    calls = trace.read_text().splitlines()
    assert len(calls) == 1
    assert ("--prepaid-agent-storage" in calls[0].split()) == (mode == "1")


def test_prepaid_storage_mode_rejects_invalid_configuration_before_any_command(tmp_path):
    env = os.environ | {"X3_AGENT_PREPAID_STORAGE": "auto", "X3_AGENT_RUN_DIR": str(tmp_path)}
    result = subprocess.run(["bash", str(SCRIPT), "show"], env=env, capture_output=True, text=True, timeout=10)
    assert result.returncode != 0
    assert "X3_AGENT_PREPAID_STORAGE must be 0 or 1" in result.stderr
