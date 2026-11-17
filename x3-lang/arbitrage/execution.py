"""Compile individual legs; live preparation delegates to the existing mandate pipeline.

No signer, deposit submission or automatic multi-leg orchestration exists here.
"""
import re
import json
import time
from copy import deepcopy
from hashlib import sha256
import tempfile
from pathlib import Path

import near_intents as ni

from .engine import assess, uint


def export_leg(route, index, *, tokens, refund_to, recipient, now,
               slippage_bps=50, fee_bps=100, mandate_owner=None):
    if type(index) is not int or not 0 <= index < len(route.legs):
        raise ValueError('invalid leg index')
    if route.kind != 'cross-chain' or route.legs[index].provider != 'oneclick':
        raise ValueError('this route has no supported .x3 execution adapter')
    # A loss projection may still be inspected, but malformed/stale routes cannot be exported.
    assessment = assess(route, now=now)
    if set(assessment['reasons']) - {'BELOW_PROFIT_THRESHOLD'}:
        raise ValueError('route is not exportable: ' + ', '.join(assessment['reasons']))
    for name, value, maximum in (('slippage_bps', slippage_bps, 500), ('fee_bps', fee_bps, 10000)):
        if uint(value, name) > maximum:
            raise ValueError(f'{name} exceeds limit')
    if mandate_owner is not None and not re.fullmatch(r'[a-z0-9][a-z0-9_.-]{1,63}', mandate_owner):
        raise ValueError('invalid mandate owner')
    if not all(isinstance(address, str) and re.fullmatch(r'[A-Za-z0-9_.:-]{2,128}', address)
               for address in (refund_to, recipient)):
        raise ValueError('invalid receiver address')
    leg = route.legs[index]
    endpoints = []
    inverse = {network: chain for chain, network in ni.CHAIN_TO_BLOCKCHAIN.items()}
    for asset in (leg.asset_in, leg.asset_out):
        chain = inverse.get(asset.network)
        matches = [token for token in tokens if token.get('assetId') == asset.asset_id
                   and token.get('blockchain') == asset.network and token.get('decimals') == asset.decimals]
        if chain is None or len(matches) != 1:
            raise ValueError('exact asset identity or compiler chain is unsupported')
        symbol = matches[0].get('symbol', '')
        if not re.fullmatch(r'[A-Za-z][A-Za-z0-9_]{0,31}', symbol):
            raise ValueError('asset symbol cannot be expressed by the compiler')
        resolved = ni.resolve_asset(tokens, chain, symbol)
        if resolved['assetId'] != asset.asset_id or resolved['decimals'] != asset.decimals:
            raise ValueError('compiler asset resolution differs from scanned asset')
        endpoints.append(f'{chain}.{symbol}')
    origin, destination = endpoints
    ttl = min(leg.expires_at - now, 600)
    under = f'    under mandate {mandate_owner}\n' if mandate_owner else ''
    binding = {'route_id': route.id, 'leg': index, 'asset_in': leg.asset_in.to_json(),
               'asset_out': leg.asset_out.to_json(), 'amount': str(leg.amount_in),
               'minimum_out': str(leg.minimum_out), 'expires_at': leg.expires_at,
               'evidence': leg.evidence}
    source = (
        '// X3 Arbitrage: individual leg; multi-leg execution is not atomic.\n'
        f'// Route {route.id}; evidence {leg.evidence}. Refresh and re-evaluate before execution.\n'
        '// This program bounds one delivery; it does not enforce a round-trip profit.\n'
        f'intent arb_{route.id[:16]}_{index} {{\n'
        f'    from {origin} amount {leg.amount_in} receiver {refund_to}\n'
        f'    to {destination} receiver {recipient}\n'
        '    route {\n'
        f'        bridge NearIntents {origin} -> {destination} min_receive {leg.minimum_out};\n'
        '    }\n' + under +
        f'    require slippage <= {slippage_bps} bps\n'
        f'    require fees <= {fee_bps} bps\n'
        f'    require nonce unused arb_{route.id}_{index}\n'
        f'    timeout {ttl}s refund {origin}\n'
        '    on_fail rollback\n}\n')
    binding['program_sha256'] = sha256(source.encode('utf-8')).hexdigest()
    source = '// x3-arbitrage-binding: ' + json.dumps(binding, sort_keys=True, separators=(',', ':')) + '\n' + source
    with tempfile.TemporaryDirectory(prefix='x3-arb-export-') as directory:
        path = Path(directory) / 'leg.x3'
        path.write_text(source, encoding='utf-8')
        ni.compile_program(str(path))
    return source


def prepare_live_quote(program, *, mandate, guard_contract, receipt, explicit_live=False,
                       client=None, now=None):
    if explicit_live is not True:
        raise ValueError('explicit live-quote opt-in required; this creates a deposit quote')
    if not mandate or not guard_contract or not receipt:
        raise ValueError('mandate, guard contract and receipt path are required')
    now = int(time.time()) if now is None else now
    source = Path(program).read_text(encoding='utf-8')
    lines = [line.removeprefix('// x3-arbitrage-binding: ') for line in source.splitlines()
             if line.startswith('// x3-arbitrage-binding: ')]
    if len(lines) != 1:
        raise ValueError('live preparation requires one exported asset binding')
    binding = json.loads(lines[0])
    if not source.startswith('// x3-arbitrage-binding: ') or not isinstance(binding, dict):
        raise ValueError('invalid exported asset binding')
    body = source.split('\n', 1)[1]
    if sha256(body.encode('utf-8')).hexdigest() != binding.get('program_sha256'):
        raise ValueError('program changed since export; refresh and export again')
    if binding.get('evidence') != 'provider_dry_quote':
        raise ValueError('demo or orderbook data cannot prepare live quotes')
    if uint(binding.get('expires_at'), 'binding expiry') - now < 10:
        raise ValueError('scanned quote expired; refresh and export again')
    # Compile, plan and quote the validated bytes, never reopen the mutable user path.
    with tempfile.TemporaryDirectory(prefix='x3-arb-live-') as directory:
        snapshot_program = Path(directory) / 'leg.x3'
        snapshot_program.write_text(source, encoding='utf-8')
        snapshot_program.chmod(0o400)
        _, leg = ni.compile_program(str(snapshot_program))
        if not leg.get('mandate_owner'):
            raise ValueError('live preparation requires an intent under mandate')
        original = client or ni.OneClickClient()
        snapshot = deepcopy(original.tokens())

        class BoundCatalogClient:
            def tokens(self):
                return deepcopy(snapshot)

            def __getattr__(self, name):
                return getattr(original, name)

        bound = BoundCatalogClient()
        planned = ni.plan(str(snapshot_program), client=bound, live=True)
        for side, key in (('asset_in', 'origin'), ('asset_out', 'destination')):
            token = planned['tokens'][key]
            asset = binding[side]
            if (token['assetId'], token['blockchain'], token['decimals']) != (
                    asset['asset_id'], asset['network'], asset['decimals']):
                raise ValueError('live catalog changed the scanned asset identity')
        if (str(leg['amount']), str(leg['min_receive'])) != (binding['amount'], binding['minimum_out']):
            raise ValueError('program amount or floor changed from scanned binding')
        # ni.quote checks live mandate state, accounts, nonces, signed terms and fee bounds.
        # It obtains a deposit address; it neither signs nor deposits funds.
        return ni.quote(str(snapshot_program), client=bound, live=True,
                        mandate_path=str(mandate), guard_contract=guard_contract,
                        receipt_path=str(receipt))
