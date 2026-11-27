"""Editable .x3 mandates compiled through the existing two-leg policy pipeline.

Preview uses an isolated real SQLite database; it never reserves actual runner budgets,
requests quotes, authorizes a wallet or invokes a live backend.
"""
import tempfile
from pathlib import Path

import near_intents as ni
from near_liquidity.quote import formatted_to_units
from .trader import Strategy, Trader, paper_strategy_template

FIELDS = {'route_id', 'mandates', 'addresses', 'minimum_profit', 'max_external_cost', 'expires_at'}


def decimal_text(units, decimals):
    if units is None:
        return ''
    whole, fraction = divmod(units, 10 ** decimals)
    tail = str(fraction).rjust(decimals, '0').rstrip('0') if decimals else ''
    return str(whole) + ('.' + tail if tail else '')


def template(route, tokens, *, now, addresses=None):
    net = (route.legs[-1].minimum_out - route.legs[0].amount_in - route.external_cost_units
           if route.external_cost_units is not None else 0)
    minimum = max(1, min(10 ** route.legs[0].asset_in.decimals, net // 2))
    strategy = paper_strategy_template(route, tokens, now=now, wallets=addresses,
                                       minimum_profit_units=minimum)
    decimals = route.legs[0].asset_in.decimals
    return {'route_id': route.id, 'mandates': list(strategy.mandates), 'addresses': list(strategy.addresses),
            'minimum_profit': decimal_text(strategy.minimum_profit_units, decimals),
            'max_external_cost': decimal_text(strategy.max_external_cost_units, decimals),
            'expires_at': strategy.expires_at}


def strategy_from_draft(draft, route, tokens, *, now):
    if not isinstance(draft, dict) or set(draft) != FIELDS:
        raise ValueError('expected route, two .x3 mandates, two paper addresses and profit/cost/expiry limits')
    if draft['route_id'] != route.id:
        raise ValueError('draft differs from selected route')
    if not isinstance(draft['mandates'], list) or len(draft['mandates']) != 2 or not all(
            isinstance(source, str) and 1 <= len(source.encode()) <= 16384 for source in draft['mandates']):
        raise ValueError('exactly two bounded .x3 mandate sources required')
    if not isinstance(draft['addresses'], list) or len(draft['addresses']) != 2 or not all(
            isinstance(address, str) and 2 <= len(address) <= 128 for address in draft['addresses']):
        raise ValueError('two bounded simulated wallet addresses required')
    if type(draft['expires_at']) is not int or not 0 < draft['expires_at'] <= now + 600:
        raise ValueError('draft expiry exceeds the bounded strategy window')
    for name in ('minimum_profit', 'max_external_cost'):
        if not isinstance(draft[name], str) or len(draft[name]) > 80:
            raise ValueError('bounded decimal profit and cost limits required')
    decimals = route.legs[0].asset_in.decimals
    return Strategy(route, tokens, tuple(draft['mandates']), tuple(draft['addresses']),
                    formatted_to_units(draft['minimum_profit'], decimals, 'minimum profit'),
                    formatted_to_units(draft['max_external_cost'], decimals, 'external cost ceiling'),
                    draft['expires_at'])


def compile_draft(draft, route, tokens, *, now):
    strategy = strategy_from_draft(draft, route, tokens, now=now)
    try:
        with tempfile.TemporaryDirectory(prefix='x3-strategy-preview-') as directory:
            preview = Trader(Path(directory) / 'preview.sqlite', clock=lambda: now)
            result = preview.create('compile-preview', strategy, mode='paper')
            mandates = [preview.store.mandate(identity) for identity in result['mandate_ids']]
    except ni.NearIntentsError as exc:
        raise ValueError(f'{exc.code}: {exc.message}') from exc
    return {'valid': True, 'programs': result['programs'],
            'mandates': [{'owner': policy['owner'], 'agent': policy['agent'],
                          'terms': policy['terms']} for policy in mandates],
            'validation_scope': 'Compiler and declared .x3 policy. Registered budgets/revocation are rechecked on paper execution.',
            'eligible_for_wallet_authorization': False, 'real_transactions_submitted': 0}
