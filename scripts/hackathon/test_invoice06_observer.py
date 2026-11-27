"""Exercise the observer's invoice_06 evidence predicates on public recorded receipts."""
import copy
import json
from pathlib import Path

import pytest

import observe_mainnet_invoice06 as observer


@pytest.fixture
def payout():
    archive = Path(__file__).resolve().parents[2] / "evidence/near-guard-mainnet-invoice06-2026-11-12/public-rpc-observation.json"
    return json.loads(archive.read_text())["near_transactions"]["payout"]


def test_archived_payout(payout):
    assert observer.final_success(payout)
    assert observer.invoice_payout_matches(observer.payout_events(payout))


@pytest.mark.parametrize("change", ["optimistic", "missing_receipts", "unknown_status", "failed_child", "dangling_child"])
def test_incomplete_execution(payout, change):
    if change == "optimistic":
        payout["final_execution_status"] = "EXECUTED_OPTIMISTIC"
    elif change == "missing_receipts":
        payout["receipts"] = []
    elif change == "dangling_child":
        payout["receipts"][0]["status"] = {"SuccessReceiptId": "missing"}
    else:
        payout["receipts"][0]["status"] = {"Failure": {}} if change == "failed_child" else "Unknown"
    assert not observer.final_success(payout)


@pytest.mark.parametrize("change", ["executor", "recipient", "amount", "sender", "event", "unstructured", "split", "duplicate", "reversal"])
def test_unrelated_log_is_insufficient(payout, change):
    receipts = [r for r in payout["receipts"] if r["executor"] == observer.NEAR_USDC]
    receipt = next(r for r in receipts if any('"ft_transfer"' in log for log in r["logs"]))
    log = next(log for log in receipt["logs"] if '"ft_transfer"' in log)
    raw = json.loads(log.removeprefix("EVENT_JSON:"))
    data = raw["data"][0]
    if change == "executor":
        receipt["executor"] = "other.near"
    elif change == "recipient":
        data["new_owner_id"] = "another.near"
    elif change == "amount":
        data["amount"] = "124995200"
    elif change == "sender":
        data["old_owner_id"] = "other.near"
    elif change == "event":
        raw["event"] = "ft_mint"
    elif change == "split":
        raw["data"].append({**data, "new_owner_id": "another.near"})
        data["amount"] = "1"
    elif change == "reversal":
        raw["data"].append({**data, "old_owner_id": observer.GUARD, "new_owner_id": "intents.near"})
    elif change == "duplicate":
        raw["data"].append(copy.copy(data))
    receipt["logs"] = ["x3lang.near 24995200"] if change == "unstructured" else ["EVENT_JSON:" + json.dumps(raw)]
    assert not observer.invoice_payout_matches(observer.payout_events(payout))
