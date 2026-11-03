"""X3Lang value-fee arithmetic: transparent opt-in quote previews.

No token movements or invoice creation are performed here. Quoted improvement
is not *realized* savings; actual billing requires independent settlement
attribution, recipient authorization, provider cost reconciliation and a
non-replayable wallet/contract enforcement layer.
"""
from dataclasses import dataclass


class ValueFeeError(ValueError):
    """A fee comparison is malformed or lacks an honest common basis."""


@dataclass(frozen=True)
class ComparableOffer:
    """Integer base units of ONE destination token, including quoted known costs.

    Quotes may only be compared if source, destination, amount and receiver
    match. Identifiers/timestamps are required for replayable audit evidence;
    callers must validate signatures and freshness separately before trusting
    the offers. Do not pass USD float prices or mix asset decimals.
    """
    input_chain: str
    input_asset: str
    input_units: int
    destination_chain: str
    destination_asset: str
    recipient: str
    guaranteed_output_units: int
    all_known_costs_output_units: int
    quote_id: str
    quoted_at_seconds: int
    expires_at_seconds: int
    # Explicit evidence that the caller reconciled ALL known cost categories;
    # merely supplying zero cost cannot establish completeness.
    costs_complete: bool = False

    @property
    def net_units(self) -> int:
        return self.guaranteed_output_units - self.all_known_costs_output_units


@dataclass(frozen=True)
class SavingsSharePolicy:
    """Maximum operator share of *positive net improvement* (bps).

    Opt-in here means the policy was explicitly selected by the caller, not
    that a real wallet signed an authorization. Services still need binding
    user consent and chain-enforced fee caps before any fee transfer.
    """
    opted_in: bool
    max_savings_share_bps: int
    min_user_gain_units: int
    max_service_fee_units: int
    service_id: str


@dataclass(frozen=True)
class ValueFeePreview:
    eligible: bool
    reference_net_units: int
    candidate_pre_fee_units: int
    gross_improvement_units: int
    proposed_fee_units: int
    user_net_units: int
    user_benefit_units: int
    status: str = "preview_not_billing_authorization"
    collectible: bool = False


_CONTEXT_FIELDS = (
    "input_chain", "input_asset", "input_units", "destination_chain",
    "destination_asset", "recipient",
)


def _validate_offer(offer: ComparableOffer, now_seconds: int) -> None:
    if not isinstance(offer, ComparableOffer):
        raise ValueFeeError("incomparable quote: invalid offer type")
    if not all(isinstance(getattr(offer, f), str) and getattr(offer, f).strip()
               for f in ("input_chain", "input_asset", "destination_chain",
                         "destination_asset", "recipient", "quote_id")):
        raise ValueFeeError("incomparable quote: missing identity")
    if type(offer.input_units) is not int or offer.input_units <= 0:
        raise ValueFeeError("incomparable quote: input amount invalid")
    if type(offer.guaranteed_output_units) is not int or offer.guaranteed_output_units <= 0:
        raise ValueFeeError("incomparable quote: non-positive guaranteed output")
    if (type(offer.all_known_costs_output_units) is not int
            or offer.all_known_costs_output_units < 0
            or offer.all_known_costs_output_units > offer.guaranteed_output_units):
        raise ValueFeeError("quote cost is unknown, negative, or exceeds output")
    if offer.costs_complete is not True:
        raise ValueFeeError('quote cost reconciliation incomplete')
    if (type(offer.quoted_at_seconds) is not int or type(offer.expires_at_seconds) is not int
            or offer.quoted_at_seconds > now_seconds
            or now_seconds - offer.quoted_at_seconds > 120
            or offer.expires_at_seconds < now_seconds
            or offer.expires_at_seconds <= offer.quoted_at_seconds):
        raise ValueFeeError("quote freshness is unverifiable or expired")


def calculate_value_fee(
    reference: ComparableOffer,
    candidate: ComparableOffer,
    policy: SavingsSharePolicy,
    *,
    now_seconds: int,
) -> ValueFeePreview:
    """Quote a savings share only if user is strictly better off after fee.

    Refuses non-comparable or stale offers. This function does NOT validate
    offer signatures, prove baseline market fairness, or authorize billing.
    An adapter must obtain signed comparable quotes from independent trusted
    sources and enforce any resulting fee on-chain with the user's approval.
    """
    if not isinstance(policy, SavingsSharePolicy):
        raise ValueFeeError("invalid policy")
    if (type(policy.opted_in) is not bool or type(policy.max_savings_share_bps) is not int
            or not 0 <= policy.max_savings_share_bps <= 10_000
            or type(policy.max_service_fee_units) is not int
            or policy.max_service_fee_units < 0
            or type(policy.min_user_gain_units) is not int
            or policy.min_user_gain_units < 1
            or not isinstance(policy.service_id, str)
            or not policy.service_id.strip()):
        raise ValueFeeError("invalid policy: share, caps, consent, or service identity")
    if type(now_seconds) is not int:
        raise ValueFeeError("quote freshness timestamp must be an integer")
    _validate_offer(reference, now_seconds)
    _validate_offer(candidate, now_seconds)
    if any(getattr(reference, f) != getattr(candidate, f) for f in _CONTEXT_FIELDS):
        raise ValueFeeError("incomparable offers: input, output or recipient differs")
    if reference.quote_id == candidate.quote_id:
        raise ValueFeeError("incomparable offers: a single quote is not an independent reference")
    if abs(reference.quoted_at_seconds - candidate.quoted_at_seconds) > 30:
        raise ValueFeeError("quote freshness: comparison quotes are too far apart")

    before = reference.net_units
    candidate_net = candidate.net_units
    improvement = candidate_net - before
    if improvement <= 0 or improvement < policy.min_user_gain_units:
        return ValueFeePreview(
            eligible=False, reference_net_units=before,
            candidate_pre_fee_units=candidate_net, gross_improvement_units=improvement,
            proposed_fee_units=0, user_net_units=candidate_net,
            user_benefit_units=improvement,
        )

    # ALL calculations are in integer destination-asset base units.
    # A 100% share still cannot eliminate the user's strictly positive gain.
    proposed = 0
    if policy.opted_in:
        proposed = min(
            improvement * policy.max_savings_share_bps // 10_000,
            policy.max_service_fee_units,
            improvement - policy.min_user_gain_units,
        )
    user_net = candidate_net - proposed
    assert user_net >= before + policy.min_user_gain_units
    return ValueFeePreview(
        eligible=True, reference_net_units=before,
        candidate_pre_fee_units=candidate_net, gross_improvement_units=improvement,
        proposed_fee_units=proposed, user_net_units=user_net,
        user_benefit_units=user_net - before,
    )


def main() -> int:
    """Deterministic JSON-in/JSON-out local preview; never a payment command."""
    import argparse
    from dataclasses import asdict
    import json
    import sys

    cli = argparse.ArgumentParser(description="X3Lang opt-in savings share: non-billable preview")
    cli.add_argument("--preview-stdin", action="store_true", help="read JSON offer/policy details on stdin")
    args = cli.parse_args()
    if not args.preview_stdin:
        cli.error("use --preview-stdin; this program cannot collect fees")
    try:
        payload = json.load(sys.stdin)
        preview = calculate_value_fee(
            ComparableOffer(**payload["reference"]),
            ComparableOffer(**payload["candidate"]),
            SavingsSharePolicy(**payload["policy"]),
            now_seconds=payload["now_seconds"],
        )
    except (ValueFeeError, KeyError, TypeError, ValueError) as exc:
        print(f"X3_VALUE_FEE_REFUSED: {exc}", file=sys.stderr)
        return 2
    print(json.dumps(asdict(preview), sort_keys=True, separators=(",", ":")))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
