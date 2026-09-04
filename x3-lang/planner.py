"""Planner for minimal X3 Lang intents.

This planner enriches path steps with explicit source/destination chains, assets,
and structured metadata used by emitters and simulators.
"""
from typing import Dict, Any, List, Optional


SWAP_CHAIN_MAP = {
    'raydium': 'solana',
    'uniswap': 'ethereum'
}


def infer_chain(step: Dict[str, Any], previous_chain: Optional[str]) -> str:
    if step.get('type') == 'swap':
        return SWAP_CHAIN_MAP.get(step.get('dex'), previous_chain or 'solana')
    if step.get('type') == 'bridge':
        return previous_chain or 'solana'
    return previous_chain or 'solana'


def infer_output_asset(step: Dict[str, Any]) -> str:
    if step.get('type') == 'swap':
        return step.get('to')
    return step.get('asset')


def plan(intent: Dict[str, Any]) -> Dict[str, Any]:
    steps: List[Dict[str, Any]] = []
    prev_chain = intent.get('from', {}).get('chain')
    prev_asset = intent.get('from', {}).get('asset')
    path = intent.get('path', [])

    starting_amount = None
    raw_amount = intent.get('from', {}).get('amount')
    try:
        if raw_amount is not None:
            starting_amount = float(str(raw_amount))
    except (TypeError, ValueError):
        starting_amount = None

    for idx, p in enumerate(path):
        chain = infer_chain(p, prev_chain)
        output_asset = infer_output_asset(p)
        step: Dict[str, Any] = {
            'step': idx + 1,
            'type': p.get('type'),
            'action': p.get('type'),
            'chain': chain,
            'input_asset': prev_asset,
            'output_asset': output_asset,
            'raw': p
        }

        explicit_amount = p.get('amount')
        if p.get('type') == 'swap':
            step.update({
                'dex': p.get('dex'),
                'from': p.get('from'),
                'to': p.get('to'),
                'amount': explicit_amount or (str(starting_amount) if starting_amount is not None and idx == 0 else None)
            })
        elif p.get('type') == 'bridge':
            step.update({
                'via': p.get('via'),
                'asset': p.get('asset'),
                'amount': explicit_amount or (str(starting_amount) if starting_amount is not None else None)
            })
        else:
            step.update({'details': p})

        steps.append(step)
        prev_chain = chain
        prev_asset = output_asset

    estimates = {
        'estimated_gas_usd': 0.0,
        'estimated_bridge_fee_usd': 0.0,
        'estimated_slippage_usd': 0.0,
        'expected_profit_usd': None,
        'min_required_profit_usd': 0.0
    }
    c = intent.get('constraints') or {}
    mp = c.get('min_profit')
    if isinstance(mp, str):
        try:
            estimates['min_required_profit_usd'] = float(mp.split()[0])
        except Exception:
            pass

    return {
        'intent': intent.get('intent'),
        'from': intent.get('from'),
        'to': intent.get('to'),
        'steps': steps,
        'constraints': intent.get('constraints', {}),
        'metadata': {
            'chain_sequence': [step['chain'] for step in steps],
            'destination_chain': intent.get('to', {}).get('chain')
        },
        'estimates': estimates
    }


if __name__ == '__main__':
    import json, sys
    j = json.load(sys.stdin)
    print(json.dumps(plan(j), indent=2))
