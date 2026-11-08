"""Boundary condition tests for mandate verification to catch off-by-one mutations.

These tests specifically target the boundary conditions that can be weakened
by off-by-one mutations (e.g., <= changed to <). These represent realistic P0/P1
bugs where exact limits become inclusive instead of exclusive or vice versa.
"""

import pytest
from datetime import datetime, timedelta, timezone
from pathlib import Path

from test_near_mandate import (  # noqa: F401
    NOW_MS, NOW, ROOT, SOL_ADDR, SOL_USDC, NEAR_USDC, TOKENS,
    BASE, _compiled, _failed, _policy, tokens, agent_intent, mandate_file,
)

import near_intents as ni

nm = ni.mandates()


# MandateTerms stores caps as U128. Both compilers also return a four-digit UTC
# timestamp, whose last millisecond is below u64 and JavaScript's safe integer limit.
U128_MAX = 2**128 - 1
UTC_MAX_MS = 253_402_300_799_999


def numeric_mandate_file(tmp_path, changes):
    lines = list(BASE)
    for clause, value in changes.items():
        lines = [line for line in lines if not line.startswith(clause + " ")]
        if value is not None:
            lines.append(f"{clause} {value}")
    return mandate_file(tmp_path, *lines)


NUMERIC_REFUSALS = [
    pytest.param({"per_payment": U128_MAX + 1, "per_24h": U128_MAX + 1},
                 "X3_MANDATE_LIMIT", "per_payment", id="u128-payment-overflow"),
    pytest.param({"per_24h": U128_MAX + 1},
                 "X3_MANDATE_LIMIT", "per_24h", id="u128-daily-overflow"),
    pytest.param({"per_payment": "9" * 5000},
                 "X3_MANDATE_LIMIT", "per_payment", id="payment-integer-digit-limit"),
    pytest.param({"per_24h": "9" * 5000},
                 "X3_MANDATE_LIMIT", "per_24h", id="daily-integer-digit-limit"),
    pytest.param({"approve": f"above {U128_MAX + 1} by 1 of x3-owner.near"},
                 "X3_MANDATE_APPROVAL", "approve", id="u128-approval-overflow"),
    pytest.param({"approve": "above " + "9" * 5000 + " by 1 of x3-owner.near"},
                 "X3_MANDATE_APPROVAL", "approve", id="approval-integer-digit-limit"),
    pytest.param({"require slippage": "<= 10001bps"},
                 "X3_MANDATE_BOUND", "require slippage", id="slippage-10001"),
    pytest.param({"require fees": "<= 10001 bps"},
                 "X3_MANDATE_BOUND", "require fees", id="fees-10001"),
    pytest.param({"require slippage": "<= 100.01%"},
                 "X3_MANDATE_BOUND", "require slippage", id="slippage-over-100-percent"),
    pytest.param({"require fees": "< 10002 bps"},
                 "X3_MANDATE_BOUND", "require fees", id="strict-fees-still-over-10000"),
    pytest.param({"expires": "1000000000d"},
                 "X3_MANDATE_EXPIRES", "expires", id="timedelta-overflow"),
    pytest.param({"expires": "3000000d"},
                 "X3_MANDATE_EXPIRES", "expires", id="datetime-addition-overflow"),
    pytest.param({"expires": f"{UTC_MAX_MS - NOW_MS + 1}ms"},
                 "X3_MANDATE_EXPIRES", "expires", id="utc-last-millisecond-plus-one"),
    pytest.param({"expires": f"{2**64}ms"},
                 "X3_MANDATE_EXPIRES", "expires", id="u64-overflow"),
    pytest.param({"expires": "9" * 400 + "d"},
                 "X3_MANDATE_EXPIRES", "expires", id="non-finite-js-duration"),
    pytest.param({"expires": "9" * 5000 + "ms"},
                 "X3_MANDATE_EXPIRES", "expires", id="python-integer-digit-limit"),
]


NUMERIC_CONTROLS = [
    pytest.param({"per_payment": U128_MAX, "per_24h": U128_MAX},
                 {"per_payment_cap": str(U128_MAX), "daily_cap": str(U128_MAX)}, id="u128-max"),
    pytest.param({"per_24h": U128_MAX}, {"daily_cap": str(U128_MAX)}, id="u128-daily-max"),
    pytest.param({"per_payment": f"000_{U128_MAX:_}", "per_24h": "0" * 5000 + str(U128_MAX)},
                 {"per_payment_cap": str(U128_MAX), "daily_cap": str(U128_MAX)}, id="u128-formatted-max"),
    pytest.param({"per_payment": U128_MAX, "per_24h": U128_MAX,
                  "approve": f"above {U128_MAX - 1} by 1 of x3-owner.near"},
                 {"approval_above": str(U128_MAX - 1)}, id="u128-approval-below-cap"),
    pytest.param({"approve": "above " + "0" * 5000 + "30_000_000 by 1 of x3-owner.near"},
                 {"approval_above": "30000000"}, id="approval-formatted-below-cap"),
    pytest.param({"expires": "1ms"}, {"expires_at_ms": NOW_MS + 1}, id="one-millisecond"),
    pytest.param({"expires": "30d"}, {"expires_at_ms": NOW_MS + 30 * 86_400_000}, id="ordinary-expiry"),
    pytest.param({"expires": f"{UTC_MAX_MS - NOW_MS}ms"},
                 {"expires_at_ms": UTC_MAX_MS}, id="utc-last-millisecond"),
]


@pytest.mark.parametrize("changes,code,field", NUMERIC_REFUSALS)
def test_compiler_refuses_out_of_range_mandate_numbers(tmp_path, changes, code, field):
    path = numeric_mandate_file(tmp_path, changes)
    with pytest.raises(ni.NearIntentsError) as err:
        nm.compile_mandate(path, tokens(), now=NOW)
    assert err.value.code == code
    assert err.value.field == field
    if field in {"per_payment", "per_24h", "approve", "expires"}:
        assert err.value.detail == {"line": nm.parse_mandate(path)["line_of"][field]}


@pytest.mark.parametrize("changes,expected", NUMERIC_CONTROLS)
def test_compiler_preserves_valid_mandate_number_boundaries(tmp_path, changes, expected):
    compiled = nm.compile_mandate(numeric_mandate_file(tmp_path, changes), tokens(), now=NOW)
    assert {key: compiled["terms"][key] for key in expected} == expected
    if compiled["terms"]["expires_at_ms"] == UTC_MAX_MS:
        assert compiled["expires"] == "9999-12-31T23:59:59.999Z"


BPS_CONTROLS = [("<= 0bps", 0), ("<= 10000bps", 10000), ("< 10001bps", 10000),
                ("<= 10000.9bps", 10000), ("< 10000bps", 9999), ("<= 100%", 10000)]


@pytest.mark.parametrize("kind,term", [("slippage", "max_slippage_bps"), ("fees", "max_fee_bps")])
@pytest.mark.parametrize("bound,expected", BPS_CONTROLS)
def test_compiler_preserves_bps_boundaries_and_strict_rounding(tmp_path, kind, term, bound, expected):
    path = numeric_mandate_file(tmp_path, {f"require {kind}": bound})
    assert nm.compile_mandate(path, tokens(), now=NOW)["terms"][term] == expected


@pytest.mark.parametrize("kind,term", [("slippage", "max_slippage_bps"), ("fees", "max_fee_bps")])
def test_compiler_keeps_the_strictest_bps_clause(tmp_path, kind, term):
    path = mandate_file(tmp_path, *BASE, f"require {kind} <= 10001bps")
    assert nm.compile_mandate(path, tokens(), now=NOW)["terms"][term] == 100


@pytest.mark.parametrize("duration", ["1000000000d", f"{2**64}ms", "9" * 5000 + "ms"],
                         ids=["timedelta-overflow", "u64-overflow", "integer-digit-limit"])
def test_expiry_parser_never_leaks_numeric_overflow(duration):
    with pytest.raises(ni.NearIntentsError) as err:
        nm.parse_expiry(duration, line=12)
    assert err.value.code == "X3_MANDATE_EXPIRES"
    assert err.value.field == "expires" and err.value.detail == {"line": 12}


@pytest.mark.parametrize("duration,ms", [("1ms", 1), ("2s", 2000), ("3m", 180000),
                                        ("4h", 14400000), ("30d", 2592000000)])
def test_expiry_parser_preserves_valid_units(duration, ms):
    assert nm.parse_expiry(duration) == timedelta(milliseconds=ms)


def test_expiry_at_last_millisecond_uses_integer_arithmetic(tmp_path):
    now = datetime(9999, 12, 31, 23, 59, 59, 998999, tzinfo=timezone.utc)
    compiled = nm.compile_mandate(numeric_mandate_file(tmp_path, {"expires": "1ms"}), tokens(), now=now)
    assert compiled["expires"] == "9999-12-31T23:59:59.999Z"
    assert compiled["terms"]["expires_at_ms"] == UTC_MAX_MS


def test_expiry_before_unix_epoch_is_not_a_u64(tmp_path):
    path = numeric_mandate_file(tmp_path, {"expires": "1ms"})
    with pytest.raises(ni.NearIntentsError) as err:
        nm.compile_mandate(path, tokens(), now=datetime(1969, 12, 31, tzinfo=timezone.utc))
    assert err.value.code == "X3_MANDATE_EXPIRES"


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
