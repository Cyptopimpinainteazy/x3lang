from dataclasses import dataclass
from decimal import Decimal

class RiskRejected(ValueError): pass
@dataclass(frozen=True)
class MarketInputs:
    source_amount_units: int
    source_decimals: int
    destination_decimals: int
    source_usd_price: Decimal
    destination_usd_price: Decimal
    available_destination_units: int
    execution_cost_usd: Decimal
    annualized_volatility: float
    settlement_delay_seconds: int
    failure_probability: float
    failure_loss_usd: Decimal
    price_observed_at: int
    now_seconds: int
    price_verified: bool
    inventory_verified: bool

@dataclass(frozen=True)
class RiskPolicy:
    max_fee_bps: int
    fee_step_bps: int
    min_expected_profit_usd: Decimal
    max_probability_loss: Decimal
    max_cvar95_loss_usd: Decimal
    simulations: int


# Deterministic scenario replay uses a fixed PRNG seed. This models risk, not oracle truth.
# No quote is executable until independently verified escrow and wallet authority exist.
from math import expm1, isfinite, sqrt
from random import Random

MODEL_VERSION = "x3.monte-carlo.quote-risk.v1"
SECONDS_PER_YEAR = 365 * 24 * 60 * 60


@dataclass(frozen=True)
class QuoteRisk:
    fee_bps: int
    destination_amount_units: int
    expected_profit_usd: Decimal
    probability_loss: Decimal
    cvar95_loss_usd: Decimal
    simulations: int
    seed: int
    model_version: str = MODEL_VERSION
    executable: bool = False


def _valid_decimal(value: Decimal, *, allow_zero: bool = False) -> bool:
    return isinstance(value, Decimal) and value.is_finite() and (
        value >= 0 if allow_zero else value > 0
    )


def price_quote(m: MarketInputs, p: RiskPolicy, *, seed: int = 42) -> QuoteRisk:
    """Search the lowest risk-compliant fee, using shared simulated scenarios.

    Source and destination asset prices and inventory must come from separately
    verified, fresh adapters. This function does NOT verify external evidence.
    Its output is non-executable and must never initiate or approve a trade.
    """
    if not (m.price_verified and m.inventory_verified):
        raise RiskRejected("verified oracle price and escrow inventory are required")
    if (m.now_seconds < m.price_observed_at or
            m.now_seconds - m.price_observed_at > 30):
        raise RiskRejected("stale or future-dated market evidence")
    if (m.source_amount_units <= 0 or m.available_destination_units <= 0 or
            m.source_amount_units >= 2**256):
        raise RiskRejected("invalid amount or available inventory")
    if any(not isinstance(d, int) or d < 0 or d > 30 for d in
           (m.source_decimals, m.destination_decimals)):
        raise RiskRejected("invalid token decimals")
    if not all((_valid_decimal(m.source_usd_price),
                _valid_decimal(m.destination_usd_price),
                _valid_decimal(m.execution_cost_usd, allow_zero=True),
                _valid_decimal(m.failure_loss_usd, allow_zero=True),
                _valid_decimal(p.min_expected_profit_usd, allow_zero=True),
                _valid_decimal(p.max_probability_loss, allow_zero=True),
                _valid_decimal(p.max_cvar95_loss_usd, allow_zero=True))):
        raise RiskRejected("invalid non-finite monetary input")
    if (not all(isfinite(f) for f in
                (m.annualized_volatility, m.failure_probability)) or
            not (0 <= m.annualized_volatility <= 5) or
            not (0 <= m.failure_probability <= 1) or
            not (0 <= m.settlement_delay_seconds <= 604800)):
        raise RiskRejected("invalid settlement risk inputs")
    if (not (1000 <= p.simulations <= 100000) or
            p.fee_step_bps <= 0 or
            not (0 <= p.max_fee_bps < 10000) or
            not (0 <= p.max_probability_loss <= 1)):
        raise RiskRejected("invalid risk policy")
    trade_usd = (
        Decimal(m.source_amount_units) * m.source_usd_price /
        (Decimal(10) ** m.source_decimals)
    )
    gross_out = int(
        (trade_usd / m.destination_usd_price *
         Decimal(10) ** m.destination_decimals).to_integral_value(
            rounding="ROUND_FLOOR"
        )
    )
    if gross_out <= 0:
        raise RiskRejected("destination amount rounds to zero")
    rng = Random(seed)
    scaled_sigma = m.annualized_volatility * sqrt(
        m.settlement_delay_seconds / SECONDS_PER_YEAR
    )
    base_costs = []
    for _ in range(p.simulations):
        shock = scaled_sigma * rng.gauss(0, 1)
        adverse_price_cost = (trade_usd * Decimal(
            str(expm1(shock - 0.5 * scaled_sigma * scaled_sigma))
        )) if scaled_sigma else Decimal(0)
        failure_cost = (m.failure_loss_usd if rng.random() < m.failure_probability
                        else Decimal(0))
        base_costs.append(m.execution_cost_usd + adverse_price_cost + failure_cost)
    for fee_bps in range(0, p.max_fee_bps + 1, p.fee_step_bps):
        amount_out = gross_out * (10000 - fee_bps) // 10000
        if amount_out <= 0 or amount_out > m.available_destination_units:
            continue
        fee_usd = trade_usd * Decimal(fee_bps) / Decimal(10000)
        profits = [fee_usd - cost for cost in base_costs]
        expected = sum(profits, Decimal(0)) / p.simulations
        loss_probability = Decimal(sum(x < 0 for x in profits)) / p.simulations
        losses = sorted((max(Decimal(0), -x) for x in profits), reverse=True)
        worst_tail = losses[:max(1, (p.simulations + 19) // 20)]
        cvar95 = sum(worst_tail, Decimal(0)) / len(worst_tail)
        if (expected >= p.min_expected_profit_usd and
                loss_probability <= p.max_probability_loss and
                cvar95 <= p.max_cvar95_loss_usd):
            return QuoteRisk(
                fee_bps, amount_out,
                expected.quantize(Decimal("0.000001")),
                loss_probability.quantize(Decimal("0.000001")),
                cvar95.quantize(Decimal("0.000001")),
                p.simulations, seed
            )
    raise RiskRejected("no quote meets the risk and inventory thresholds")
