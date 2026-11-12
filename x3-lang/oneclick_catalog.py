"""All-provider-chain discovery and dry quote transport; never deposit authorization.

Catalog presence is not compiler, custody, liquidity or finality support. Limits are
advisory snapshots from https://docs.near-intents.org/resources/swap-limits
(owner-provided 2026-11-11); the provider response remains authoritative.
"""
import argparse
import json
import re
from datetime import datetime, timezone
from decimal import Decimal, DecimalException, localcontext

import near_intents as ni


def catalog(tokens):
    """Validate exact asset IDs; discover chains without a hard-coded allowlist."""
    result = {}
    for token in tokens:
        if not isinstance(token, dict):
            raise ValueError('invalid token record')
        asset, chain, decimals = (token.get(k) for k in ('assetId', 'blockchain', 'decimals'))
        if not isinstance(asset, str) or not asset or not isinstance(chain, str) or not chain:
            raise ValueError('missing assetId or blockchain')
        if type(decimals) is not int or not 0 <= decimals <= 255:
            raise ValueError('invalid token decimals')
        if asset in result:
            raise ValueError('duplicate assetId; refusing ambiguous catalog')
        result[asset] = dict(token)
    return result


def limit_advisory(request, tokens):
    if any(not isinstance(request.get(k), str) for k in ('originAsset', 'destinationAsset', 'swapType')):
        raise ValueError('asset IDs and swapType must be strings')
    origin, destination = (tokens[request[k]] for k in ('originAsset', 'destinationAsset'))
    swap_type = request['swapType']
    if swap_type not in ('EXACT_INPUT', 'EXACT_OUTPUT'):
        raise ValueError('only EXACT_INPUT and EXACT_OUTPUT supported')
    amount = request['amount']
    if not isinstance(amount, str) or not re.fullmatch(r'[1-9][0-9]{0,77}', amount):
        raise ValueError('amount must be a positive base-unit integer string (max 78 digits)')
    chain_to_chain = (request.get('depositType') == 'ORIGIN_CHAIN'
                      and request.get('recipientType') == 'DESTINATION_CHAIN')
    minimum = 0
    if chain_to_chain:
        if 'bsc' in (origin['blockchain'], destination['blockchain']):
            minimum = 1000
        elif origin['blockchain'] == 'tron':
            minimum = 100
    result = dict(minimum_usd=str(minimum), advisory_only=True,
                  source='https://docs.near-intents.org/resources/swap-limits',
                  snapshot_date='2026-11-11', bridge_minimum_unknown=True,
                  status='no_documented_chain_minimum' if minimum == 0 else 'price_unknown')
    priced = origin if swap_type == 'EXACT_INPUT' else destination
    try:
        price = Decimal(str(priced.get('price')))
        if not price.is_finite() or price <= 0 or abs(price.adjusted()) > 100:
            return result
        with localcontext() as ctx:
            ctx.prec = 400
            usd = Decimal(amount) * price / (Decimal(10) ** priced['decimals'])
        result.update(estimated_usd=str(usd), price_updated_at=priced.get('priceUpdatedAt'))
        if minimum:
            result['status'] = 'below_documented_minimum' if usd < minimum else 'meets_documented_minimum'
    except (DecimalException, ValueError):
        pass
    return result


_FIELDS = {'dry', 'swapType', 'originAsset', 'destinationAsset', 'amount',
           'depositType', 'recipientType', 'refundType', 'recipient', 'refundTo',
           'deadline', 'slippageTolerance'}


def dry_quote(client, request):
    """Discovery only: no compiler/mandate approval, no signing, no live mode."""
    if not isinstance(request, dict) or set(request) - _FIELDS:
        raise ValueError('unknown quote fields')
    if request.get('dry') is not True:
        raise ValueError('dry must be true; live quotes require the existing policy pipeline')
    if set(request) != _FIELDS:
        raise ValueError('all explicit quote fields are required')
    for field, allowed in (('depositType', {'ORIGIN_CHAIN', 'INTENTS'}),
                           ('recipientType', {'DESTINATION_CHAIN', 'INTENTS'}),
                           ('refundType', {'ORIGIN_CHAIN', 'INTENTS'})):
        if not isinstance(request[field], str) or request[field] not in allowed:
            raise ValueError('invalid ' + field)
    for field in ('recipient', 'refundTo'):
        if not isinstance(request[field], str) or not request[field].strip() or len(request[field]) > 256:
            raise ValueError('invalid ' + field)
    if type(request['slippageTolerance']) is not int or not 0 <= request['slippageTolerance'] <= 500:
        raise ValueError('slippage must be 0..500 bps')
    if ni.parse_timestamp(request['deadline']) <= datetime.now(timezone.utc):
        raise ValueError('deadline has expired')
    tokens = catalog(client.tokens())
    try:
        limits = limit_advisory(request, tokens)
    except KeyError as exc:
        raise ValueError('asset is absent from current provider catalog') from exc
    result = dict(mode='dry_discovery', eligible_for_wallet_authorization=False,
                  policy_checked=False, settlement_verified=False, limits=limits)
    # Do not raise an amount, switch routes, or bypass a minimum automatically.
    try:
        result.update(status='provider_quote_unverified', response=client.quote(dict(request)))
    except ni.NearIntentsError as exc:
        result.update(status='provider_refused', error=dict(code=exc.code, message=exc.message, detail=exc.detail))
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--request', help='JSON file with an explicit dry quote request; omit to list tokens')
    args = parser.parse_args()
    client = ni.OneClickClient()
    try:
        if args.request:
            with open(args.request, encoding='utf-8') as stream:
                result = dry_quote(client, json.load(stream))
        else:
            tokens = catalog(client.tokens())
            result = dict(mode='provider_catalog', chains=sorted({t['blockchain'] for t in tokens.values()}),
                          tokens=list(tokens.values()), eligible_for_wallet_authorization=False)
        print(json.dumps(result, indent=2))
        return 1 if result.get('status') == 'provider_refused' else 0
    except (ValueError, ni.NearIntentsError, OSError) as exc:
        print(json.dumps({'error': str(exc), 'eligible_for_wallet_authorization': False}))
        return 2


if __name__ == '__main__':
    raise SystemExit(main())
