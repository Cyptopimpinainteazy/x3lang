"""Value-based monetization must be opt-in, measurable and fail closed.

The module returns an OFF-CHAIN FEE PROPOSAL, never a wallet payment or
proof of cross-chain execution. Test data are only illustrative integers.
"""
import dataclasses
import pytest

from value_fee import (
    ComparableOffer,
    SavingsSharePolicy,
    ValueFeeError,
    calculate_value_fee,
)


def offer(*, output=10_000, costs=0, observed=100, ttl=60, quote="base"):
    return ComparableOffer(
        input_chain="solana",
        input_asset="USDC",
        input_units=25_000_000,
        destination_chain="near",
        destination_asset="USDC",
        recipient="user.near",
        guaranteed_output_units=output,
        all_known_costs_output_units=costs,
        quote_id=quote,
        quoted_at_seconds=observed,
        expires_at_seconds=observed + ttl,
    )


def policy(*, enabled=True, share=2500, minimum=1, cap=1000):
    return SavingsSharePolicy(
        opted_in=enabled,
        max_savings_share_bps=share,
        min_user_gain_units=minimum,
        max_service_fee_units=cap,
        service_id="x3-managed-quote-routing",
    )


def test_service_only_earns_a_share_of_verified_quote_improvement_preview():
    result = calculate_value_fee(
        offer(quote="reference"),
        offer(output=10_200, quote="candidate"),
        policy(),
        now_seconds=110,
    )
    assert result.eligible is True
    assert result.reference_net_units == 10_000
    assert result.candidate_pre_fee_units == 10_200
    assert result.gross_improvement_units == 200
    assert result.proposed_fee_units == 50
    assert result.user_net_units == 10_150
    assert result.user_benefit_units == 150
    assert result.collectible is False
    assert result.status == "preview_not_billing_authorization"


def test_all_known_network_costs_must_be_deducted_before_savings_share():
    result = calculate_value_fee(
        offer(output=10_000, costs=10, quote="reference"),
        offer(output=10_200, costs=80, quote="candidate"),
        policy(),
        now_seconds=105,
    )
    assert result.reference_net_units == 9_990
    assert result.candidate_pre_fee_units == 10_120
    assert result.gross_improvement_units == 130
    assert result.proposed_fee_units == 32  # round down, never charge dust up
    assert result.user_net_units == 10_088


def test_user_keeps_some_savings_even_when_owner_requests_full_share():
    result = calculate_value_fee(
        offer(quote="reference"), offer(output=10_200, quote="candidate"),
        policy(share=10_000), now_seconds=105,
    )
    assert result.eligible is True
    assert result.proposed_fee_units == 199
    assert result.user_benefit_units == 1


@pytest.mark.parametrize("amount", [10_000, 9_900])
def test_no_improvement_means_no_fee_or_route_authorization(amount):
    result = calculate_value_fee(
        offer(quote="reference"), offer(output=amount, quote="candidate"),
        policy(), now_seconds=105,
    )
    assert not result.eligible
    assert result.proposed_fee_units == 0
    assert not result.collectible
    assert result.user_benefit_units <= 0


def test_no_opt_in_means_no_fee_but_user_may_still_get_better_quote():
    result = calculate_value_fee(
        offer(quote="reference"), offer(output=10_500, quote="candidate"),
        policy(enabled=False), now_seconds=105,
    )
    assert result.eligible
    assert result.proposed_fee_units == 0
    assert result.user_net_units == 10_500


@pytest.mark.parametrize("changed", [
    {"input_units": 25_000_001},
    {"input_asset": "SOL"},
    {"destination_chain": "ethereum"},
    {"destination_asset": "WETH"},
    {"recipient": "attacker.near"},
])
def test_incomparable_quote_pairs_are_refused(changed):
    with pytest.raises(ValueFeeError, match="incomparable"):
        calculate_value_fee(
            offer(quote="reference"), dataclasses.replace(offer(quote="candidate"), **changed),
            policy(), now_seconds=105,
        )


@pytest.mark.parametrize("observed,ttl,now", [(100, 4, 105), (150, 60, 105)])
def test_expired_or_future_dated_quotes_are_refused(observed, ttl, now):
    with pytest.raises(ValueFeeError, match="fresh"):
        calculate_value_fee(
            offer(quote="reference"),
            offer(quote="candidate", observed=observed, ttl=ttl),
            policy(), now_seconds=now,
        )


def test_quote_comparison_rejects_unknown_costs_and_invalid_fees():
    with pytest.raises(ValueFeeError, match="cost"):
        calculate_value_fee(
            offer(quote="reference"),
            offer(quote="candidate", costs=-1),
            policy(), now_seconds=105,
        )
    with pytest.raises(ValueFeeError, match="invalid policy"):
        calculate_value_fee(
            offer(quote="reference"), offer(quote="candidate", output=20_000),
            policy(share=10_001), now_seconds=105,
        )


def test_max_absolute_fee_and_minimum_user_benefit_are_hard_caps():
    result = calculate_value_fee(
        offer(quote="reference"),
        offer(quote="candidate", output=15_000),
        policy(share=2500, cap=20, minimum=10),
        now_seconds=110,
    )
    assert result.proposed_fee_units == 20
    assert result.user_benefit_units == 4_980
    assert result.eligible


def test_signed_quotes_are_not_settlements_and_never_create_billable_invoice():
    result = calculate_value_fee(
        offer(quote="reference"),
        offer(quote="candidate", output=100_000),
        policy(), now_seconds=105,
    )
    assert result.status == "preview_not_billing_authorization"
    assert result.collectible is False
    assert "billable" not in dataclasses.asdict(result)


def test_fee_cli_shows_readable_nonbillable_preview():
    import json
    import subprocess
    import sys

    from pathlib import Path
    entry = Path(__file__).resolve().parents[1] / "value_fee.py"
    payload = {
        "reference": dataclasses.asdict(offer(quote="baseline")),
        "candidate": dataclasses.asdict(offer(output=10_200, quote="candidate")),
        "policy": dataclasses.asdict(policy()),
        "now_seconds": 105,
    }
    result = subprocess.run(
        [sys.executable, str(entry), "--preview-stdin"],
        input=json.dumps(payload), capture_output=True, text=True, check=False,
    )
    assert result.returncode == 0, result.stderr
    body = json.loads(result.stdout)
    assert body["proposed_fee_units"] == 50
    assert body["user_benefit_units"] == 150
    assert body["collectible"] is False
    assert body["status"] == "preview_not_billing_authorization"


def test_fee_cli_rejects_incomparable_quotes_without_creating_receipt():
    import json
    import subprocess
    import sys

    from pathlib import Path
    entry = Path(__file__).resolve().parents[1] / "value_fee.py"
    payload = {
        "reference": dataclasses.asdict(offer(quote="baseline")),
        "candidate": dataclasses.asdict(offer(output=10_200, quote="candidate", ttl=1)),
        "policy": dataclasses.asdict(policy()),
        "now_seconds": 105,
    }
    result = subprocess.run(
        [sys.executable, str(entry), "--preview-stdin"],
        input=json.dumps(payload), capture_output=True, text=True, check=False,
    )
    assert result.returncode == 2
    assert result.stdout == ""
    assert "fresh" in result.stderr.lower()
