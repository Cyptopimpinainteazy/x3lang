import importlib.util
import sys
import unittest
from pathlib import Path
from decimal import Decimal

SPEC = importlib.util.spec_from_file_location("x3_mc", Path(__file__).with_name("monte_carlo.py"))
mc = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = mc
SPEC.loader.exec_module(mc)

def market(**overrides):
    args=dict(source_amount_units=10**16, source_decimals=18,
              destination_decimals=24, source_usd_price=Decimal("600"),
              destination_usd_price=Decimal("2"), available_destination_units=10*10**24,
              execution_cost_usd=Decimal(".05"), annualized_volatility=0,
              settlement_delay_seconds=180, failure_probability=0,
              failure_loss_usd=Decimal("2"), price_observed_at=1000,
              now_seconds=1005, price_verified=True, inventory_verified=True)
    args.update(overrides)
    return mc.MarketInputs(**args)

def policy(**overrides):
    args=dict(max_fee_bps=200, fee_step_bps=10, min_expected_profit_usd=Decimal(".01"),
              max_probability_loss=Decimal(".01"),
              max_cvar95_loss_usd=Decimal(".01"), simulations=1000)
    args.update(overrides)
    return mc.RiskPolicy(**args)

class MonteCarloTests(unittest.TestCase):
    def test_fee_must_cover_execution_cost_and_margin(self):
        result=mc.price_quote(market(),policy(),seed=9)
        self.assertEqual(result.fee_bps,100)
        self.assertEqual(result.destination_amount_units,297*10**22)
        self.assertFalse(result.executable)
        self.assertEqual(result.simulations,1000)

    def test_repeatability_for_evidence(self):
        self.assertEqual(mc.price_quote(market(annualized_volatility=.2), policy(max_fee_bps=300), seed=7),
                         mc.price_quote(market(annualized_volatility=.2), policy(max_fee_bps=300), seed=7))

    def test_missing_verified_liquidity_fails_closed(self):
        with self.assertRaises(mc.RiskRejected):
            mc.price_quote(market(inventory_verified=False),policy())

    def test_stale_price_fails_closed(self):
        with self.assertRaises(mc.RiskRejected):
            mc.price_quote(market(now_seconds=1100),policy())

    def test_not_enough_destination_inventory(self):
        with self.assertRaises(mc.RiskRejected):
            mc.price_quote(market(available_destination_units=10**20),policy())

    def test_unaffordable_risk_rejected_even_when_price_is_available(self):
        with self.assertRaises(mc.RiskRejected):
            mc.price_quote(market(failure_probability=1, failure_loss_usd=Decimal("100")),policy())

    def test_invalid_inputs_fail_closed(self):
        with self.assertRaises(mc.RiskRejected):
            mc.price_quote(market(source_amount_units=0),policy())
        with self.assertRaises(mc.RiskRejected):
            mc.price_quote(market(source_usd_price=Decimal("NaN")),policy())
        with self.assertRaises(mc.RiskRejected):
            mc.price_quote(market(price_verified=False),policy())

if __name__ == "__main__":
    unittest.main()
