"""Bounded public market discovery. No exchange credentials or wallet operations."""
import json
import time
import urllib.request
from datetime import datetime, timezone
from fractions import Fraction

import near_intents as ni
from near_liquidity.oneclick import fetch_oneclick_quote
from near_liquidity.quote import AssetRef, FeeCharge, checked_sub
from oneclick_catalog import catalog

from .engine import Leg, Route, uint


def demo_routes(*, now):
    """Constructed examples, intentionally including losses and unknown costs."""
    tokens = [
        {'assetId': 'demo:sol-usdc', 'blockchain': 'sol', 'symbol': 'USDC', 'decimals': 6},
        {'assetId': 'demo:near-usdc', 'blockchain': 'near', 'symbol': 'USDC', 'decimals': 6},
        {'assetId': 'demo:eth-usdc', 'blockchain': 'eth', 'symbol': 'USDC', 'decimals': 6},
    ]
    assets = [AssetRef(t['blockchain'], t['assetId'], 6) for t in tokens]
    rows = []
    for label, src, dst, output, cost in (
        ('Solana → NEAR → Solana', assets[0], assets[1], 102_500_000, 500_000),
        ('Ethereum → NEAR → Ethereum', assets[2], assets[1], 104_000_000, 6_000_000),
        ('NEAR → Solana → NEAR', assets[1], assets[0], 103_000_000, None),
    ):
        legs = (Leg('oneclick', src, dst, 100_000_000, 102_000_000, 101_000_000, now, now + 120),
                Leg('oneclick', dst, src, 101_000_000, output + 200_000, output, now, now + 120))
        rows.append(Route(label, 'cross-chain', legs, cost, 'Constructed demo external-cost estimate'))
    return rows, tokens


def validate_config(config):
    if not isinstance(config, dict) or set(config) != {'routes'}:
        raise ValueError('configuration requires only routes')
    if not isinstance(config['routes'], list) or not 1 <= len(config['routes']) <= 4:
        raise ValueError('routes must contain 1..4 explicit round trips')
    for row in config['routes']:
        fields = {'label', 'origin_asset', 'destination_asset', 'amount', 'origin_address',
                  'destination_address', 'external_cost_units', 'cost_source'}
        if not isinstance(row, dict) or set(row) != fields:
            raise ValueError('route requires exact assets, addresses, amount and external-cost provenance')
        for key in ('label', 'origin_asset', 'destination_asset', 'origin_address', 'destination_address', 'cost_source'):
            if not isinstance(row[key], str) or not 1 <= len(row[key]) <= 160:
                raise ValueError('invalid route ' + key)
        uint(row['amount'], 'amount', positive=True)
        if row['external_cost_units'] is not None:
            uint(row['external_cost_units'], 'external_cost_units')
        if row['origin_asset'] == row['destination_asset']:
            raise ValueError('round trip must cross assets or chains')
    return config


def _floor(quote):
    floor = quote.min_amount_out
    # Existing adapter conservatively treats ambiguous withdrawal fees as additional.
    for fee in quote.fees:
        if fee.charge in (FeeCharge.ADDITIONAL, FeeCharge.UNKNOWN):
            if fee.asset != quote.asset_out.key or fee.amount is None:
                raise ValueError('fee cannot be valued in output asset')
            floor = checked_sub(floor, fee.amount, 'quote floor after fees')
    return uint(floor, 'net floor', positive=True)


def scan_cross_chain(config, *, client=None, now=None):
    validate_config(config)
    client = client or ni.OneClickClient(timeout=8, api_key='')
    now = int(time.time()) if now is None else now
    tokens = list(catalog(client.tokens()).values())
    token_map = {token['assetId']: token for token in tokens}
    routes, errors = [], []
    for row in config['routes']:
        try:
            origin, destination = token_map[row['origin_asset']], token_map[row['destination_asset']]
            amount, legs = uint(row['amount'], 'amount', positive=True), []
            for token_in, token_out, sender, recipient in (
                (origin, destination, row['origin_address'], row['destination_address']),
                (destination, origin, row['destination_address'], row['origin_address']),
            ):
                request = {
                    'dry': True, 'swapType': 'EXACT_INPUT', 'originAsset': token_in['assetId'],
                    'destinationAsset': token_out['assetId'], 'amount': str(amount),
                    'depositType': 'ORIGIN_CHAIN', 'recipientType': 'DESTINATION_CHAIN',
                    'refundType': 'ORIGIN_CHAIN', 'recipient': recipient, 'refundTo': sender,
                    'deadline': datetime.fromtimestamp(now + 120, timezone.utc).isoformat(),
                    'slippageTolerance': 50,
                }
                quote = fetch_oneclick_quote(client, request, tokens, now=now)
                floor = _floor(quote)
                expected = quote.expected_net_output()
                if expected is None:
                    raise ValueError('expected output costs unknown')
                legs.append(Leg('oneclick', quote.asset_in, quote.asset_out, amount,
                                expected, floor, quote.issued_at, quote.expires_at,
                                'provider_dry_quote', quote.provenance.reference))
                amount = floor
            routes.append(Route(row['label'], 'cross-chain', tuple(legs),
                                None if row['external_cost_units'] is None else uint(row['external_cost_units'], 'cost'),
                                row['cost_source']))
        except (ValueError, KeyError, ni.NearIntentsError) as exc:
            errors.append({'provider': 'oneclick', 'route': row['label'],
                           'code': getattr(exc, 'code', 'INVALID_PROVIDER_RESPONSE'),
                           'message': 'Route refused; check token IDs, quote signature, minimums and provider availability.'})
    return routes, tokens, errors


def _fraction(text):
    if not isinstance(text, str) or len(text) > 80:
        raise ValueError('price/size must be bounded decimal strings')
    import re
    if not re.fullmatch(r'(0|[1-9][0-9]*)(\.[0-9]+)?', text):
        raise ValueError('invalid orderbook decimal')
    value = Fraction(text)
    if value <= 0:
        raise ValueError('orderbook values must be positive')
    return value


def fill_book(levels, amount, *, buy, decimals):
    """Walk full supplied depth; round spend up and proceeds down, never extrapolate."""
    uint(amount, 'book amount', positive=True)
    if type(decimals) is not int or not 0 <= decimals <= 18:
        raise ValueError('unsupported base decimals')
    if not isinstance(levels, list) or not 1 <= len(levels) <= 500:
        raise ValueError('invalid orderbook depth')
    remaining, output, previous = amount, 0, None
    for level in levels:
        if not isinstance(level, list) or len(level) < 2:
            raise ValueError('invalid orderbook level')
        price, size = _fraction(level[0]), _fraction(level[1])
        if previous is not None and ((buy and price < previous) or (not buy and price > previous)):
            raise ValueError('orderbook levels are not sorted')
        previous = price
        capacity = int(size * 10 ** decimals)
        ratio = price * 10 ** 6 / 10 ** decimals
        if buy:
            take = min(capacity, int(remaining / ratio))
            spent = -(-(take * ratio).numerator // (take * ratio).denominator)
            remaining -= spent
            output += take
            # Dust smaller than one base unit cannot be spent; conservatively omit it.
            if remaining < ratio:
                return uint(output, 'book output', positive=True)
        else:
            take = min(capacity, remaining)
            remaining -= take
            output += int(take * ratio)
            if remaining == 0:
                return uint(output, 'book output', positive=True)
    raise ValueError('insufficient orderbook depth')


def public_json(url):
    # Callers select fixed provider URLs; user-supplied endpoints are not accepted.
    request = urllib.request.Request(url, headers={'User-Agent': 'X3-Arbitrage/0.1'})
    with urllib.request.urlopen(request, timeout=8) as response:
        data = response.read(2_000_001)
    if len(data) > 2_000_000:
        raise ValueError('provider response too large')
    return json.loads(data)


def scan_cex(*, now=None, fetch=public_json):
    """Public BTC/USD inventory spreads, with explicit unknown transfer/rebalance costs."""
    now = int(time.time()) if now is None else now
    books, errors = {}, []
    for venue, url in (
        ('coinbase', 'https://api.exchange.coinbase.com/products/BTC-USD/book?level=2'),
        ('kraken', 'https://api.kraken.com/0/public/Depth?pair=XBTUSD&count=100'),
    ):
        try:
            body = fetch(url)
            if venue == 'kraken':
                if body.get('error') or len(body.get('result', {})) != 1:
                    raise ValueError('Kraken refused orderbook')
                body = next(iter(body['result'].values()))
            # Coinbase L2 may return tens of thousands of levels; bound work to top 100.
            for side in ('asks', 'bids'):
                if not isinstance(body.get(side), list):
                    raise ValueError('missing orderbook side')
                body[side] = body[side][:100]
            # Validate both sides before using either book.
            fill_book(body['asks'], 1_000_000, buy=True, decimals=8)
            fill_book(body['bids'], 1000, buy=False, decimals=8)
            books[venue] = body
        except Exception:
            errors.append({'provider': venue, 'code': 'ORDERBOOK_UNAVAILABLE',
                           'message': 'Public orderbook request failed or depth was invalid.'})
    routes = []
    usd = AssetRef('cex-inventory', 'USD', 6)
    btc = AssetRef('cex-inventory', 'BTC', 8)
    for buyer, seller in (('coinbase', 'kraken'), ('kraken', 'coinbase')):
        if buyer not in books or seller not in books:
            continue
        try:
            amount = 1_000_000_000
            # Conservative operator assumption: 100bps taker fee per trade, NOT account-specific.
            bought = fill_book(books[buyer]['asks'], amount * 99 // 100, buy=True, decimals=8)
            sold = fill_book(books[seller]['bids'], bought, buy=False, decimals=8) * 99 // 100
            routes.append(Route(f'BTC/USD · {buyer} → {seller}', 'cex-inventory', (
                Leg(buyer, usd, btc, amount, bought, bought, now, now + 30, 'public_orderbook'),
                Leg(seller, btc, usd, bought, sold, sold, now, now + 30, 'public_orderbook'),
            ), None, '100bps taker assumption per side included; inventory/rebalance costs unknown'))
        except ValueError:
            errors.append({'provider': f'{buyer}/{seller}', 'code': 'INSUFFICIENT_DEPTH',
                           'message': 'Full order size cannot be priced from returned depth.'})
    return routes, errors
