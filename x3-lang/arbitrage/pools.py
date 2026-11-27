"""Bounded, read-only pool observations. Neither prices nor liquidity authorize a swap.

DEX Screener's documented token-pairs endpoint supplies indexer estimates, not
size-specific quotes or verifiable chain freshness. NEAR uses existing agreed
final-block contract views. No credentials, fees, signing or deposits are used.
"""
import json
import re
import time
import urllib.request
from concurrent.futures import ThreadPoolExecutor
from decimal import Decimal, InvalidOperation

from near_liquidity.chain import AgreedNearView, RpcEndpoint
from near_liquidity.rhea import RHEA_EXCHANGE_CONTRACT, parse_pool

MAX_BYTES = 1_048_576
MAX_PAIRS = 500
PER_CHAIN = 20
# Canonical USDC contracts; the chain-qualified address, never its symbol, is matched.
TARGETS = (
    ('solana', 'EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v'),
    ('ethereum', '0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48'),
    ('arbitrum', '0xaf88d065e77c8cC2239327C5EDb3A432268e5831'),
    ('base', '0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913'),
)
RPC_URLS = ('https://rpc.mainnet.near.org', 'https://free.rpc.fastnear.com')


class _NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        raise ValueError('redirect refused')


def _request(method, url, headers, body, timeout):
    request = urllib.request.Request(url, data=body, headers=headers, method=method)
    with urllib.request.build_opener(_NoRedirect()).open(request, timeout=timeout) as response:
        raw = response.read(MAX_BYTES + 1)
        if len(raw) > MAX_BYTES:
            raise ValueError('pool response exceeds size limit')
        return response.status, raw


def _fetch(url):
    status, raw = _request('GET', url, {'Accept': 'application/json', 'User-Agent': 'X3-pool-discovery/1'}, None, 3)
    if status != 200:
        raise ValueError('pool response unavailable')
    return json.loads(raw, parse_float=Decimal)


def _address(value, network):
    if not isinstance(value, str):
        raise ValueError('invalid token or pool address')
    if network == 'solana':
        if not re.fullmatch(r'[1-9A-HJ-NP-Za-km-z]{32,44}', value):
            raise ValueError('invalid Solana address')
        alphabet = '123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz'
        number = 0
        for char in value:
            number = number * 58 + alphabet.index(char)
        size = (number.bit_length() + 7) // 8 + len(value) - len(value.lstrip('1'))
        if size != 32:
            raise ValueError('invalid Solana address size')
        return value
    if not re.fullmatch(r'0x[0-9a-fA-F]{40}', value):
        raise ValueError('invalid EVM address')
    return value.lower()


def _number(value, *, positive=False):
    if isinstance(value, bool) or not isinstance(value, (str, int, float, Decimal)):
        raise ValueError('invalid pool metric')
    if len(str(value)) > 80:
        raise ValueError('oversized pool metric')
    try:
        number = Decimal(str(value))
    except InvalidOperation:
        raise ValueError('invalid pool metric') from None
    if not number.is_finite() or number < 0 or number > 10 ** 18 or (positive and number == 0) or number.as_tuple().exponent < -36:
        raise ValueError('invalid pool metric range')
    return format(number, 'f')


def _token(value, network):
    if not isinstance(value, dict):
        raise ValueError('missing token identity')
    address = _address(value.get('address'), network)
    symbol = value.get('symbol')
    if not isinstance(symbol, str) or not 1 <= len(symbol) <= 64 or any(ord(c) < 32 for c in symbol):
        symbol = address
    return {'address': address, 'symbol': symbol}


def _indexed(network, anchor, fetch, now):
    url = f'https://api.dexscreener.com/token-pairs/v1/{network}/{anchor}'
    body = fetch(url)
    if not isinstance(body, list) or len(body) > MAX_PAIRS:
        raise ValueError('pool response requires bounded pair list')
    pools, rejected, banned = {}, 0, set()
    for raw in body:
        try:
            if not isinstance(raw, dict) or raw.get('chainId') != network:
                raise ValueError('pool chain mismatch')
            pair = _address(raw.get('pairAddress'), network)
            base, quote = _token(raw.get('baseToken'), network), _token(raw.get('quoteToken'), network)
            if base['address'] == quote['address'] or _address(anchor, network) not in (base['address'], quote['address']):
                raise ValueError('pool token mismatch')
            dex = raw.get('dexId')
            if not isinstance(dex, str) or not re.fullmatch(r'[a-zA-Z0-9_-]{1,64}', dex):
                raise ValueError('invalid DEX identifier')
            liquidity = _number(raw.get('liquidity', {}).get('usd'), positive=True)
            volume = raw.get('volume', {}).get('h24')
            volume = None if volume is None else _number(volume)
            row = {'id': f'{network}:{pair}', 'provider': 'dexscreener', 'network': network,
                   'dex': dex, 'pair_address': pair, 'base_token': base, 'quote_token': quote,
                   'liquidity_usd': liquidity, 'volume_24h_usd': volume,
                   'source_url': f'https://dexscreener.com/{network}/{pair}',
                   'observed_at': now, 'chain_timestamp': None, 'executable': False,
                   'limitation': 'Indexer estimate; upstream freshness, trade output and LP yield are unverified.'}
            if pair in banned:
                raise ValueError('conflicting duplicate pool')
            if pair in pools and pools[pair] != row:
                del pools[pair]
                banned.add(pair)
                raise ValueError('conflicting duplicate pool')
            pools[pair] = row
        except (ValueError, TypeError, AttributeError):
            rejected += 1
    rows = sorted(pools.values(), key=lambda p: (-Decimal(p['liquidity_usd']), p['id']))[:PER_CHAIN]
    return rows, rejected


def _near(now, view=None):
    view = view or AgreedNearView([RpcEndpoint(url) for url in RPC_URLS], transport=_request, timeout=3)
    block = view.final_block(now=now)
    page = view.view(block, RHEA_EXCHANGE_CONTRACT, 'get_pools', {'from_index': 0, 'limit': 100})
    if not isinstance(page.value, list) or len(page.value) > 100:
        raise ValueError('RHEA pool page is malformed')
    pools = []
    for index, raw in enumerate(page.value):
        pool = parse_pool(raw, index)
        if pool.shares_total_supply == 0 or not all(pool.amounts):
            continue
        tokens = [{'address': token, 'symbol': token} for token in pool.token_account_ids]
        pools.append({'id': f'near:rhea:{index}', 'provider': 'rhea-v2', 'network': 'near',
                      'dex': 'rhea', 'pair_address': str(index), 'base_token': tokens[0],
                      'quote_token': tokens[1], 'tokens': tokens,
                      'reserves_atomic': [str(amount) for amount in pool.amounts],
                      'liquidity_usd': None, 'volume_24h_usd': None,
                      'source_url': 'https://app.rhea.finance/',
                      'observed_at': now, 'chain_timestamp': block.timestamp_s,
                      'block_height': block.height, 'block_hash': block.hash,
                      'executable': False,
                      'limitation': 'Funded pools in first 100 IDs only; no USD valuation, size quote or LP yield.'})
    return pools[:PER_CHAIN]


def discover_pools(*, fetch=None, now=None, near_view=None):
    """Observe fixed USDC markets plus a bounded first RHEA page, never execute.

    `loaded_at` is retrieval time, not indexer data freshness. `fetch`/`near_view`
    are server-side injection seams for deterministic failure testing, not inputs
    accepted from HTTP clients. A failed source remains an explicit error.
    """
    now = int(time.time()) if now is None else now
    if type(now) is not int or now < 0:
        raise ValueError('integer observation timestamp required')
    fetch = fetch or _fetch
    pools, errors = [], []
    with ThreadPoolExecutor(max_workers=5) as executor:
        jobs = [(network, executor.submit(_indexed, network, anchor, fetch, now)) for network, anchor in TARGETS]
        near_job = executor.submit(_near, now, near_view)
        for network, job in jobs:
            try:
                rows, rejected = job.result()
                pools.extend(rows)
                if rejected:
                    errors.append({'provider': 'dexscreener', 'network': network, 'code': 'POOLS_REJECTED',
                                   'message': f'{rejected} malformed or unrelated pool rows rejected.'})
            except Exception:
                errors.append({'provider': 'dexscreener', 'network': network, 'code': 'POOLS_UNAVAILABLE',
                               'message': 'Pool index unavailable or invalid; no substitute data used.'})
        try:
            pools.extend(near_job.result())
        except Exception:
            errors.append({'provider': 'rhea-v2', 'network': 'near', 'code': 'POOLS_UNAVAILABLE',
                           'message': 'Fresh agreed RHEA pool state unavailable; no substitute data used.'})
    return {'pools': pools, 'pool_count': len(pools), 'errors': errors, 'loaded_at': now,
            'source': 'DEX Screener public token-pairs API and agreed NEAR RHEA contract views',
            'claim': 'Read-only pool discovery. Listing is not a trade quote, LP position, fee entitlement or approval.',
            'coverage': 'USDC markets on Solana, Ethereum, Arbitrum and Base; first 100 RHEA pool IDs on NEAR.',
            'fees_enabled': False, 'transactions_submitted': 0}
