"""Boundary condition tests for mandate verification to catch off-by-one mutations.

These tests specifically target the boundary conditions that can be weakened
by off-by-one mutations (e.g., <= changed to <). These represent realistic P0/P1
bugs where exact limits become inclusive instead of exclusive or vice versa.
"""

import pytest
from pathlib import Path

from test_near_mandate import (  # noqa: F401
    NOW_MS, NOW, ROOT, SOL_ADDR, SOL_USDC, NEAR_USDC, TOKENS,
    _compiled, _failed, _policy, tokens, agent_intent,
)

import near_intents as ni

nm = ni.mandates()


def test_per_payment_cap_boundary_exact_at_limit(tmp_path):
    """Payment amount exactly at per_payment_cap should be ALLOWED."""
    m = _compiled()
    # per_payment_cap is 50_000_000 in the example mandate
    p = _policy(tmp_path, amount=50_000_000)  # exactly at cap
    assert _failed(nm.check_mandate(m, p, NOW_MS)) == [], \
        "Exact per-payment limit should be allowed"


def test_per_payment_cap_boundary_over_limit(tmp_path):
    """Payment amount one unit over per_payment_cap should be REFUSED."""
    m = _compiled()
    # per_payment_cap is 50_000_000 in the example mandate
    p = _policy(tmp_path, amount=50_000_001)  # one over cap
    failed = _failed(nm.check_mandate(m, p, NOW_MS))
    assert "mandate_per_payment" in failed, \
        "Per-payment cap violation should be detected"


def test_daily_cap_boundary_exact_at_limit(tmp_path):
    """Total spend exactly at daily_cap should be ALLOWED."""
    m = _compiled()
    p = _policy(tmp_path, amount=25_000_000)  # half of daily_cap

    # Add one prior spend to reach exactly at cap
    m["spends"] = [{"at_ms": NOW_MS - 1_000_000, "amount": "175_000_000", "nonce": "prior"}]
    # 175_000_000 + 25_000_000 = 200_000_000 = daily_cap

    assert _failed(nm.check_mandate(m, p, NOW_MS)) == [], \
        "Exact daily cap should be allowed"


def test_daily_cap_boundary_over_limit(tmp_path):
    """Total spend one unit over daily_cap should be REFUSED."""
    m = _compiled()
    p = _policy(tmp_path, amount=25_000_000)

    # Add prior spend that pushes total just over cap
    m["spends"] = [{"at_ms": NOW_MS - 1_000_000, "amount": "175_000_001", "nonce": "prior"}]
    # 175_000_001 + 25_000_000 = 200_000_001 > daily_cap

    failed = _failed(nm.check_mandate(m, p, NOW_MS))
    assert "mandate_daily" in failed, \
        "Daily cap violation should be detected"


def test_deadline_boundary_exact_at_expiry(tmp_path):
    """Intent deadline exactly at mandate expiry should be ALLOWED."""
    m = _compiled()
    p = _policy(tmp_path)
    # Set deadline to exactly the mandate expiry time
    p["deadline_ms"] = m["terms"]["expires_at_ms"]

    assert _failed(nm.check_mandate(m, p, NOW_MS)) == [], \
        "Deadline exactly at mandate expiry should be allowed"


def test_deadline_boundary_just_after_expiry(tmp_path):
    """Intent deadline one ms after mandate expiry should be REFUSED."""
    m = _compiled()
    p = _policy(tmp_path)
    # Set deadline to one ms after mandate expiry
    p["deadline_ms"] = m["terms"]["expires_at_ms"] + 1

    failed = _failed(nm.check_mandate(m, p, NOW_MS))
    assert "mandate_deadline" in failed, \
        "Deadline after mandate expiry should be detected"


def test_slippage_boundary_exact_at_max(tmp_path):
    """Slippage exactly at max_slippage_bps should be ALLOWED."""
    m = _compiled()
    # max_slippage_bps is 100 in the example
    p = _policy(tmp_path, slippage="1%")  # exactly at max

    assert _failed(nm.check_mandate(m, p, NOW_MS)) == [], \
        "Slippage exactly at max should be allowed"


def test_slippage_boundary_over_max(tmp_path):
    """Slippage one bps over max_slippage_bps should be REFUSED."""
    m = _compiled()
    # max_slippage_bps is 100 in the example
    p = _policy(tmp_path, slippage="1.01%")  # 101 bps, over 100 max

    failed = _failed(nm.check_mandate(m, p, NOW_MS))
    assert "mandate_slippage" in failed, \
        "Slippage over max should be detected"


def test_approval_boundary_at_threshold(tmp_path):
    """Amount just above approval_above with approvals_threshold met should be ALLOWED.

    This tests the approval threshold boundary: the approval_above amount should trigger
    approvals, but an amount exactly at that value should not.
    """
    # This is already tested in test_near_mandate.py via approvals_needed,
    # but we document it here as a mutation-killing boundary test.
    m = _compiled()
    # approval_above is 30_000_000 in the example mandate
    assert nm.approvals_needed(m, 30_000_000) == 0, \
        "Amount at approval_above should NOT require approvals"
    assert nm.approvals_needed(m, 30_000_001) == 1, \
        "Amount above approval_above should require approvals"


def test_rolling_window_boundary_at_24_hours(tmp_path):
    """Spend exactly 24 hours old should NOT be counted in window."""
    m = _compiled()
    p = _policy(tmp_path, amount=100_000_000)

    # Add a spend exactly 24 hours (WINDOW_MS) old
    now = NOW_MS
    window = 86_400_000  # WINDOW_MS
    m["spends"] = [
        {"at_ms": now - window, "amount": "100_000_000", "nonce": "old"},
        {"at_ms": now - 1_000_000, "amount": "50_000_000", "nonce": "recent"}
    ]

    # The 24-hour-old spend should NOT be counted, only the recent one
    # So total = 50_000_000 + 100_000_000 = 150_000_000 <= daily_cap (200_000_000)
    failed = _failed(nm.check_mandate(m, p, NOW_MS))
    assert "mandate_daily" not in failed, \
        "24-hour-old spend should be outside rolling window"


def test_rolling_window_boundary_just_inside_24_hours(tmp_path):
    """Spend just inside 24 hours old should be counted in window."""
    m = _compiled()
    p = _policy(tmp_path, amount=100_000_000)

    # Add a spend just inside the 24-hour window (1 ms inside)
    now = NOW_MS
    window = 86_400_000  # WINDOW_MS
    m["spends"] = [
        {"at_ms": now - window + 1, "amount": "100_000_000", "nonce": "boundary"},
        {"at_ms": now - 1_000_000, "amount": "50_000_000", "nonce": "recent"}
    ]

    # The just-inside spend should be counted
    # So total = 100_000_000 + 50_000_000 + 100_000_000 = 250_000_000 > daily_cap
    failed = _failed(nm.check_mandate(m, p, NOW_MS))
    assert "mandate_daily" in failed, \
        "Spend just inside 24-hour window should be counted"


def test_mandate_expiry_boundary_exact_at_time(tmp_path):
    """Checking mandate at exact expiry time should FAIL.

    The mandate is active while now < expires_at_ms. At the exact moment of expiry,
    it should be expired.
    """
    m = _compiled()
    p = _policy(tmp_path)

    # Check at exact expiry time
    expiry_ms = m["terms"]["expires_at_ms"]
    failed = _failed(nm.check_mandate(m, p, expiry_ms))
    assert "mandate_active" in failed, \
        "Mandate should be expired at exact expiry time"


def test_mandate_expiry_boundary_one_ms_before(tmp_path):
    """Checking mandate one ms before expiry should PASS."""
    m = _compiled()
    p = _policy(tmp_path)

    # Check one ms before expiry
    expiry_ms = m["terms"]["expires_at_ms"]
    failed = _failed(nm.check_mandate(m, p, expiry_ms - 1))
    assert "mandate_active" not in failed, \
        "Mandate should still be active one ms before expiry"


def test_maximum_payment_window_reached(tmp_path):
    """Filling the payment window should reject the next payment."""
    m = _compiled()
    p = _policy(tmp_path, amount=1_000_000)

    # Add 64 spends (MAX_PAYMENTS_PER_WINDOW) all within the window
    # The next payment should fail due to count limit
    m["spends"] = [
        {"at_ms": NOW_MS - i * 1_000, "amount": "1", "nonce": f"payment_{i}"}
        for i in range(64)
    ]

    failed = _failed(nm.check_mandate(m, p, NOW_MS))
    assert "mandate_daily" in failed, \
        "Exceeding MAX_PAYMENTS_PER_WINDOW should fail"
