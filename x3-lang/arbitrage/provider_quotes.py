"""Unsigned LI.FI size-specific observations, never transaction preparation.

Official contract checked 2026-11-20:
https://docs.li.fi/api-reference/get-a-quote-for-a-token-transfer
GET /v1/quote binds chain-qualified tokens, amounts and both addresses. Although
LI.FI returns transactionRequest, this adapter discards all executable fields.
No integrator fees, API key, approval or execution endpoint is configured here.
The quote query explicitly requests fee=0.
"""
from copy import deepcopy
from decimal import Decimal
import json
import re
import time
import urllib.parse
import urllib.request

from near_liquidity.quote import formatted_to_units

BASE_URL = 'https://li.quest/v1/quote'
MAX_BYTES = 1_048_576
MAX_INPUT = 1_000_000 * 10 ** 6  # bounded at one million USDC per observation
_MARKETS = (
    ('ethereum', 1, '0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48', '0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2'),
    ('arbitrum', 42161, '0xaf88d065e77c8cC2239327C5EDb3A432268e5831', '0x82aF49447D8a07e3bd95BD0d56f35241523fBab1'),
    ('base', 8453, '0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913', '0x4200000000000000000000000000000000000006'),
)


def market_catalog():
    markets = [{'id': network + '-usdc-weth', 'label': network.title() + ' USDC → WETH → USDC',
             'network': network, 'chain_id': chain_id,
             'origin_token': {'address': usdc.lower(), 'chainId': chain_id, 'decimals': 6, 'symbol': 'USDC'},
             'intermediate_token': {'address': weth.lower(), 'chainId': chain_id, 'decimals': 18, 'symbol': 'WETH'}}
            for network, chain_id, usdc, weth in _MARKETS]
    markets.append({'id': 'ethereum-base-usdc', 'label': 'Ethereum USDC → Base USDC → Ethereum USDC',
                    'network': 'ethereum-base', 'chain_id': 1,
                    'origin_token': deepcopy(markets[0]['origin_token']),
                    'intermediate_token': deepcopy(markets[2]['origin_token'])})
    return markets


class _NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        raise ValueError('quote redirect refused')


def _unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError('duplicate quote JSON property')
        result[key] = value
    return result


def _fetch(url):
    request = urllib.request.Request(url, headers={'Accept': 'application/json', 'User-Agent': 'X3-read-only-quotes/1'})
    with urllib.request.build_opener(_NoRedirect()).open(request, timeout=8) as response:
        raw = response.read(MAX_BYTES + 1)
        if response.status != 200 or len(raw) > MAX_BYTES:
            raise ValueError('quote response refused')
    return json.loads(raw, parse_float=Decimal, object_pairs_hook=_unique_object)


def _address(value, *, nonzero=False):
    if not isinstance(value, str) or not re.fullmatch(r'0x[0-9a-fA-F]{40}', value):
        raise ValueError('exact EVM address required')
    value = value.lower()
    if nonzero and value == '0x' + '0' * 40:
        raise ValueError('nonzero observation address required')
    return value


def _units(value, *, positive=False):
    if not isinstance(value, str) or not re.fullmatch(r'0|[1-9][0-9]{0,77}', value):
        raise ValueError('canonical bounded integer amount required')
    amount = int(value)
    if amount >= 2 ** 256 or (positive and amount == 0):
        raise ValueError('quote amount outside range')
    return amount


def _token(actual, expected):
    if not isinstance(actual, dict):
        raise ValueError('quote token identity missing')
    if (_address(actual.get('address')) != expected['address'] or
            type(actual.get('chainId')) is not int or actual['chainId'] != expected['chainId'] or
            type(actual.get('decimals')) is not int or actual['decimals'] != expected['decimals']):
        raise ValueError('quote asset identity changed')
    return deepcopy(expected)


def _costs(estimate, allowed_chains):
    result = []
    for field in ('feeCosts', 'gasCosts'):
        rows = estimate.get(field)
        if not isinstance(rows, list) or len(rows) > 32:
            raise ValueError('quote costs missing or unbounded')
        for row in rows:
            if not isinstance(row, dict) or not isinstance(row.get('token'), dict):
                raise ValueError('quote cost malformed')
            token = row['token']
            if (type(token.get('chainId')) is not int or token['chainId'] not in allowed_chains or
                    type(token.get('decimals')) is not int or not 0 <= token['decimals'] <= 36):
                raise ValueError('quote cost token identity malformed')
            included = row.get('included') if field == 'feeCosts' else False
            if included is not None and type(included) is not bool:
                raise ValueError('quote cost inclusion malformed')
            result.append({'kind': field, 'asset': {'address': _address(token.get('address')),
                           'chain_id': token['chainId'], 'decimals': token['decimals']},
                           'amount': str(_units(row.get('amount'))), 'included': included})
    return result


def _leg(market, source, destination, amount, address, fetch, now):
    source_chain, destination_chain = source['chainId'], destination['chainId']
    query = {'fromChain': str(source_chain), 'toChain': str(destination_chain), 'fromToken': source['address'],
             'toToken': destination['address'], 'fromAmount': str(amount), 'fromAddress': address,
             'toAddress': address, 'slippage': '0.005', 'fee': '0'}
    body = fetch(BASE_URL + '?' + urllib.parse.urlencode(query))
    if not isinstance(body, dict) or not isinstance(body.get('action'), dict) or not isinstance(body.get('estimate'), dict):
        raise ValueError('quote action and estimate required')
    action, estimate = body['action'], body['estimate']
    for field, expected_chain in (('fromChainId', source_chain), ('toChainId', destination_chain)):
        if type(action.get(field)) is not int or action[field] != expected_chain:
            raise ValueError('quote chain binding changed')
    token_in = _token(action.get('fromToken'), source)
    token_out = _token(action.get('toToken'), destination)
    if (_units(action.get('fromAmount'), positive=True) != amount or
            _units(estimate.get('fromAmount'), positive=True) != amount or
            _address(action.get('fromAddress')) != address or _address(action.get('toAddress')) != address):
        raise ValueError('quote amount or recipient changed')
    slippage = action.get('slippage')
    if isinstance(slippage, bool) or not isinstance(slippage, (int, float, Decimal)) or Decimal(str(slippage)) != Decimal('0.005'):
        raise ValueError('quote slippage changed')
    expected, floor = _units(estimate.get('toAmount'), positive=True), _units(estimate.get('toAmountMin'), positive=True)
    if floor > expected or floor < expected * 995 // 1000:
        raise ValueError('quote output floor outside requested slippage')
    tool = body.get('tool')
    if not isinstance(tool, str) or not re.fullmatch(r'[a-zA-Z0-9_.-]{1,80}', tool):
        raise ValueError('quote tool malformed')
    return {'provider': 'lifi', 'tool': tool, 'asset_in': token_in, 'asset_out': token_out,
            'amount_in': str(amount), 'expected_output': str(expected), 'minimum_output': str(floor),
            'costs': _costs(estimate, (source_chain, destination_chain)), 'observed_at': now,
            'commitment': 'unsigned_provider_observation', 'executable': False}


def quote_markets(parameters, *, fetch=None, now=None):
    """Get one bounded roundtrip observation from a fixed market and explicit address.

    The second quote consumes the first quote's minimum. This is a sequential
    what-if projection, not reserved liquidity or a quote after market impact.
    `gross_delta` includes whatever is reflected in returned minimums, but gas,
    approvals and fees lack independently authenticated final costs. Net profit
    deliberately remains unknown even if the unsigned response claims no fees.
    """
    if not isinstance(parameters, dict) or set(parameters) != {'market_id', 'amount', 'address'}:
        raise ValueError('only market_id, human USDC amount and observation address accepted')
    market = next((m for m in market_catalog() if m['id'] == parameters['market_id']), None)
    if market is None:
        raise ValueError('select an allowlisted quote market')
    address = _address(parameters['address'], nonzero=True)
    raw_amount = parameters['amount']
    if not isinstance(raw_amount, str) or len(raw_amount) > 40:
        raise ValueError('bounded decimal USDC amount required')
    amount = formatted_to_units(raw_amount, 6, 'quote amount')
    if not 0 < amount <= MAX_INPUT:
        raise ValueError('quote amount must be positive and at most one million USDC')
    now = int(time.time()) if now is None else now
    if type(now) is not int or now < 0:
        raise ValueError('integer observation timestamp required')
    fetch = fetch or _fetch
    quotes, errors = [], []
    try:
        forward = _leg(market, market['origin_token'], market['intermediate_token'], amount, address, fetch, now)
        reverse = _leg(market, market['intermediate_token'], market['origin_token'],
                       int(forward['minimum_output']), address, fetch, now)
        floor = int(reverse['minimum_output'])
        quotes.append({'provider': 'lifi', 'market_id': market['id'], 'network': market['network'],
                       'label': market['label'], 'legs': [forward, reverse], 'amount_in': str(amount),
                       'roundtrip_floor': str(floor), 'gross_delta': str(floor - amount), 'source_decimals': 6,
                       'net_profit': None, 'executable': False, 'paper_eligible': False,
                       'limitation': 'Unsigned sequential projection. Gas/approval/final fees and market impact unknown; no guaranteed net profit.'})
    except Exception:
        errors.append({'provider': 'lifi', 'market_id': market['id'], 'code': 'QUOTE_UNAVAILABLE',
                       'message': 'Complete route quote unavailable or failed exact identity/amount/slippage checks. No substitute quote used.'})
    return {'provider': 'lifi', 'market_id': market['id'], 'loaded_at': now,
            'quotes': quotes, 'errors': errors, 'source': BASE_URL, 'fees_enabled': False,
            'transactions_submitted': 0, 'claim': 'Read-only unsigned size-specific quotes; no signing or transaction preparation.'}
