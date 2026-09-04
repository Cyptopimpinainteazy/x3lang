"""Basic simulator that consumes planner output and computes rough estimates.

This is intentionally simple: it uses per-step heuristics to compute estimated gas,
bridge fees and expected profit impact (via slippage). The goal is to provide
early feedback against constraints.
"""
from typing import Dict, Any


DEFAULTS = {
    'swap_slippage_pct': 0.003,  # 0.3%
    'swap_fee_usd': {
        'solana': 0.1,
        'ethereum': 1.0,
    },
    'bridge_fee_usd': 1.0
}


import importlib.util
import os


def _load_registry():
    path = os.path.join(os.path.dirname(__file__), 'registry.py')
    spec = importlib.util.spec_from_file_location('registry', path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


registry = _load_registry()


def simulate(plan: Dict[str, Any]) -> Dict[str, Any]:
    estimates = plan.get('estimates', {})
    total_gas = 0.0
    total_bridge = 0.0
    total_slippage_cost = 0.0

    # Determine starting amount (if numeric present)
    start_amount = None
    fr = plan.get('from') or {}
    amt = fr.get('amount')
    try:
        if amt is not None:
            start_amount = float(amt)
    except Exception:
        start_amount = None

    for s in plan.get('steps', []):
        if s.get('action') == 'swap':
            # assume swap happens on a chain indicated by the asset notation
            # naive mapping: if 'from' asset == 'USDC' and plan.from.chain == 'solana' -> solana
            chain = plan.get('from', {}).get('chain', 'solana')
            gas = DEFAULTS['swap_fee_usd'].get(chain, 0.5)
            total_gas += gas
            # slippage cost = start_amount * slippage_pct (approx USD if amount in USD)
            if start_amount is not None:
                # price impact model using liquidity
                dex = s.get('dex') or 'unknown'
                pair = (s.get('from'), s.get('to'))
                liquidity = registry.DEX_LIQUIDITY.get(dex, {}).get(pair, 100000.0)
                # slippage increases with trade size relative to liquidity
                slippage_pct = DEFAULTS['swap_slippage_pct'] + 0.01 * (start_amount / (liquidity + 1))
                total_slippage_cost += start_amount * slippage_pct
        elif s.get('action') == 'bridge':
            # bridge fee may depend on asset
            total_bridge += DEFAULTS['bridge_fee_usd']

        # add small per-step failure simulation metadata
        # For now no-op; the runner will use emitted payloads to decide simulated failures

    estimates['estimated_gas_usd'] = round(total_gas, 4)
    estimates['estimated_bridge_fee_usd'] = round(total_bridge, 4)
    estimates['estimated_slippage_usd'] = round(total_slippage_cost, 4)
    # simple expected profit: start_amount - fees - slippage (if start_amount exists)
    expected_profit = None
    if start_amount is not None:
        expected_profit = round(start_amount - (total_gas + total_bridge + total_slippage_cost), 4)
    else:
        expected_profit = None

    estimates['expected_profit_usd'] = expected_profit
    plan['estimates'] = estimates
    return plan


if __name__ == '__main__':
    import json, sys
    j = json.load(sys.stdin)
    print(json.dumps(simulate(j), indent=2))
