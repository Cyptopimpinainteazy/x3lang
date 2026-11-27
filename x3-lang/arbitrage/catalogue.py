"""Provider-listed token discovery and exact-decimal route construction; never funding."""
import re
import near_intents as ni
from oneclick_catalog import catalog
from near_liquidity.quote import formatted_to_units
from .adapters import validate_config


def token_report(tokens, *, now):
    if not isinstance(tokens, list) or len(tokens) > 5000:
        raise ValueError('provider catalog exceeds bounded token limit')
    validated = catalog(tokens)
    chains = {}
    assets = []
    inverse = {network: chain for chain, network in ni.CHAIN_TO_BLOCKCHAIN.items()}
    for token in validated.values():
        network, symbol = token['blockchain'], token.get('symbol', '')
        if not isinstance(symbol, str) or len(symbol) > 128:
            symbol = ''
        deprecated = 'DEPRECATED' in symbol.upper()
        reason = None
        chain = inverse.get(network)
        if deprecated:
            reason = 'provider symbol marks this token deprecated'
        elif chain is None:
            reason = 'chain discovery only; existing .x3 adapter has no lowering for this network'
        elif not re.fullmatch(r'[A-Za-z][A-Za-z0-9_]{0,31}', symbol):
            reason = 'symbol is not expressible by the current .x3 export grammar'
        else:
            try:
                resolved = ni.resolve_asset(tokens, chain, symbol)
                if (resolved['assetId'], resolved['decimals']) != (token['assetId'], token['decimals']):
                    reason = 'symbol resolution differs from exact listed asset'
            except ni.NearIntentsError:
                reason = 'ambiguous or unsupported compiler symbol resolution'
        assets.append({'asset_id': token['assetId'], 'network': network,
                       'symbol': symbol, 'decimals': token['decimals'],
                       'contract_address': token.get('contractAddress') if isinstance(token.get('contractAddress'), str) else None,
                       'deprecated': deprecated, 'x3_export_candidate': reason is None,
                       'limitation': reason, 'executable': False})
        chains[network] = chains.get(network, 0) + 1
    assets.sort(key=lambda asset: (asset['network'], asset['deprecated'], asset['symbol'].lower(), asset['asset_id']))
    return {'assets': assets, 'chains': chains, 'asset_count': len(assets), 'loaded_at': now,
            'source': ni.DEFAULT_BASE_URL + '/v0/tokens',
            'claim': 'Provider listings only: not vetted assets, executable liquidity, approval or custody.'}


def build_route(parameters, tokens):
    required = {'origin_asset', 'destination_asset', 'amount', 'origin_address',
                'destination_address', 'external_cost', 'cost_source'}
    if not isinstance(parameters, dict) or set(parameters) != required:
        raise ValueError('exact token IDs, wallet addresses, decimal amount and cost fields required')
    lookup = catalog(tokens)
    try:
        origin, destination = lookup[parameters['origin_asset']], lookup[parameters['destination_asset']]
    except (KeyError, TypeError):
        raise ValueError('select exact assets from the loaded provider catalog') from None
    if origin['blockchain'] == destination['blockchain']:
        raise ValueError('select two different chains for cross-chain discovery')
    for token in (origin, destination):
        if 'DEPRECATED' in str(token.get('symbol', '')).upper():
            raise ValueError('deprecated listings cannot create routes')
    for field in ('amount', 'external_cost'):
        if not isinstance(parameters[field], str) or len(parameters[field]) > 80:
            raise ValueError('bounded decimal amount strings required')
    amount = formatted_to_units(parameters['amount'], origin['decimals'], 'origin amount')
    if not amount:
        raise ValueError('origin amount must be positive')
    cost = None if parameters['external_cost'] == '' else formatted_to_units(
        parameters['external_cost'], origin['decimals'], 'external cost')
    row = {'label': f"{origin.get('symbol', origin['assetId'])} ({origin['blockchain']}) → "
                    f"{destination.get('symbol', destination['assetId'])} ({destination['blockchain']}) → origin",
           'origin_asset': origin['assetId'], 'destination_asset': destination['assetId'],
           'amount': str(amount), 'origin_address': parameters['origin_address'],
           'destination_address': parameters['destination_address'], 'external_cost_units': cost,
           'cost_source': parameters['cost_source'] if cost is not None else 'Unknown external costs; route is observation-only'}
    validate_config({'routes': [row]})
    return row
