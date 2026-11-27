"""Live execution adapter for an explicitly approved, trusted local wallet gateway.

The gateway owns keys and a durable exactly-once funding journal. Signed gateway reports
are trusted-host attestations, not cryptographic proofs of cross-chain solver causality.
This task never arms this adapter or requests any live quote, Guard write or transfer.
"""
import base64
import json
import os
import secrets
import socket
import stat
import time
from datetime import datetime, timezone
from hashlib import sha256
from pathlib import Path

from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PublicKey

import near_intents as ni
from near_liquidity.oneclick import fetch_oneclick_quote, quote_from_oneclick
from near_liquidity.quote import canonical_json
from oneclick_catalog import catalog

from .adapters import _floor
from .engine import Leg, uint
from .execution import prepare_live_quote
from .trader import FundingDisabled, decode_leg


class WalletGateway:
    """Bounded Unix transport with peer UID checks and a pinned Ed25519 response key."""
    def __init__(self, path, *, public_key_hex, timeout=5):
        self.path = Path(path)
        self.key = Ed25519PublicKey.from_public_bytes(bytes.fromhex(public_key_hex))
        if not 0 < timeout <= 30:
            raise ValueError('gateway timeout must be 0..30 seconds')
        self.timeout = timeout

    def request(self, method, payload):
        if method not in {'authorize_and_fund', 'observe', 'inventory'}:
            raise ValueError('unsupported wallet gateway method')
        metadata = self.path.lstat()
        if not stat.S_ISSOCK(metadata.st_mode) or metadata.st_uid != os.getuid() or metadata.st_mode & 0o077:
            raise ValueError('wallet gateway must be a private socket owned by this user')
        envelope = {'schema': 'x3.arbitrage.wallet.v1', 'method': method,
                    'challenge': secrets.token_hex(16), 'payload': payload}
        wire = canonical_json(envelope)
        if len(wire) > 900000:
            raise ValueError('wallet request exceeds size limit')
        deadline = time.monotonic() + self.timeout
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as connection:
            connection.settimeout(self.timeout)
            connection.connect(str(self.path))
            import struct
            _, uid, _ = struct.unpack('3i', connection.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED, 12))
            if uid != os.getuid():
                raise ValueError('wallet gateway peer UID mismatch')
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError('wallet request deadline exceeded')
            connection.settimeout(remaining)
            connection.sendall(wire + b'\n')
            chunks = bytearray()
            while b'\n' not in chunks:
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    raise TimeoutError('wallet request deadline exceeded')
                connection.settimeout(remaining)
                chunk = connection.recv(min(65536, 1000001 - len(chunks)))
                if not chunk:
                    raise ValueError('truncated wallet response')
                chunks.extend(chunk)
                if len(chunks) > 1000000:
                    raise ValueError('wallet response exceeds size limit')
        response = json.loads(chunks)
        body = response['body']
        self.key.verify(base64.b64decode(response['signature'], validate=True), canonical_json(body))
        if body.get('request_sha256') != sha256(wire).hexdigest():
            raise ValueError('wallet response does not bind this request')
        if not isinstance(body.get('result'), dict):
            raise ValueError('wallet result must be an object')
        return body['result']


def _write_snapshot(path, text):
    data = text.encode('utf-8')
    try:
        descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    except FileExistsError:
        if path.is_symlink() or path.read_bytes() != data:
            raise ValueError('execution snapshot was replaced or changed')
        return
    with os.fdopen(descriptor, 'wb') as stream:
        stream.write(data)
        stream.flush()
        os.fsync(stream.fileno())


def validate_prepared_receipt(receipt, source, prepared, tokens, *, now):
    """Revalidate cached receipts from source bytes; receipt bookkeeping grants nothing."""
    import tempfile
    response = receipt['quote_response']
    with tempfile.TemporaryDirectory(prefix='x3-live-receipt-') as directory:
        path = Path(directory) / 'leg.x3'
        path.write_text(source, encoding='utf-8')
        _, compiled = ni.compile_program(str(path), now=ni.parse_timestamp(response['timestamp']))
    stable = ('intent', 'origin', 'destination', 'amount', 'recipient', 'refund_to',
              'min_receive', 'slippage_bps', 'fee_cap_bps', 'nonce', 'mandate_owner')
    if receipt.get('program_sha256') != sha256(source.encode()).hexdigest() or any(
            receipt.get('leg', {}).get(field) != compiled.get(field) for field in stable):
        raise ValueError('live receipt policy differs from prepared .x3 source')
    expected = decode_leg(prepared['leg'])
    request = receipt['quote_request']
    for side, field, asset in (('origin', 'originAsset', expected.asset_in),
                                ('destination', 'destinationAsset', expected.asset_out)):
        token = receipt['tokens'][side]
        if request[field] != asset.asset_id or (token['assetId'], token['blockchain'], token['decimals']) != (
                asset.asset_id, asset.network, asset.decimals):
            raise ValueError('live receipt exact token identity differs from prepared quote')
    signed = ni._signed_quote(receipt)
    if ni.parse_timestamp(request['deadline']) > ni.parse_timestamp(compiled['deadline']):
        raise ValueError('live quote deadline exceeds compiled duration')
    ni.verify_quote(compiled, request, response, now=datetime.fromtimestamp(now, timezone.utc))
    live = quote_from_oneclick(request, response, tokens, now=now)
    if _floor(live) < expected.minimum_out:
        raise ValueError('live quote net of withdrawal costs misses compiled delivery floor')
    return signed


class LiveExchange:
    mode = 'live'
    def __init__(self, *, gateway=None, artifact_directory=None, guard_contract=None,
                 approved_cycle=None, funding_enabled=False, client=None):
        self.gateway, self.directory, self.guard_contract = gateway, artifact_directory, guard_contract
        self.approved_cycle, self.funding_enabled = approved_cycle, funding_enabled
        # Anonymous discovery never silently reuses a credential from the environment.
        self.client = client or ni.OneClickClient(timeout=8, api_key='')

    def _authorized(self, spec, state, now):
        if (self.funding_enabled is not True or self.approved_cycle != state['identity'] or
                self.gateway is None or not self.directory or not self.guard_contract):
            raise FundingDisabled('live funding is disabled; exact-cycle approval and a trusted wallet gateway are required')
        if now >= spec['expires_at']:
            raise FundingDisabled('exact-cycle approval expired')

    def quote(self, spec, state, amount, *, now):
        self._authorized(spec, state, now)
        if state['leg_index'] == 0:
            from .trader import intent_policy
            import tempfile
            with tempfile.TemporaryDirectory(prefix='x3-live-preflight-') as directory:
                tokens = self.client.tokens()
                views = ni.near_views_for(ni.NearRpc())
                for index in (0, 1):
                    policy, compiled_leg = intent_policy(state['programs'][index], spec['tokens'], now)
                    request = ni.build_quote_request(compiled_leg, spec['tokens'], dry=True)
                    path = Path(directory) / f'mandate-{index}.x3'
                    path.write_text(spec['mandates'][index], encoding='utf-8')
                    result = ni.check_against_mandate(compiled_leg, request, policy['program_sha256'],
                        str(path), tokens=tokens, guard_contract=self.guard_contract, near_views=views)
                    if result['approvals_needed']:
                        raise ValueError('owner approvals required before any outbound funding')
        wanted = decode_leg(spec['route']['legs'][state['leg_index']])
        tokens = list(catalog(self.client.tokens()).values())
        for asset in (wanted.asset_in, wanted.asset_out):
            matching = [t for t in tokens if (t['assetId'], t['blockchain'], t['decimals']) ==
                        (asset.asset_id, asset.network, asset.decimals)]
            if len(matching) != 1:
                raise ValueError('scanned token identity absent from live catalog')
        index = state['leg_index']
        request = {'dry': True, 'swapType': 'EXACT_INPUT', 'originAsset': wanted.asset_in.asset_id,
                   'destinationAsset': wanted.asset_out.asset_id, 'amount': str(amount),
                   'depositType': 'ORIGIN_CHAIN', 'recipientType': 'DESTINATION_CHAIN',
                   'refundType': 'ORIGIN_CHAIN', 'recipient': spec['addresses'][1-index],
                   'refundTo': spec['addresses'][index], 'slippageTolerance': 50,
                   'deadline': datetime.fromtimestamp(min(now+120, spec['expires_at']), timezone.utc).isoformat()}
        quote = fetch_oneclick_quote(self.client, request, tokens, now=now)
        expected = quote.expected_net_output()
        if expected is None:
            raise ValueError('unknown quote fees')
        return Leg('oneclick', quote.asset_in, quote.asset_out, amount, expected, _floor(quote),
                   quote.issued_at, quote.expires_at, 'provider_dry_quote', quote.provenance.reference)

    def _paths(self, state):
        directory = Path(self.directory)
        directory.mkdir(mode=0o700, parents=True, exist_ok=True)
        info = directory.lstat()
        if not stat.S_ISDIR(info.st_mode) or info.st_uid != os.getuid() or info.st_mode & 0o077:
            raise ValueError('private execution artifact directory required')
        operation = state['prepared']['operation']
        return directory / (operation + '.x3'), directory / (operation + '.mandate.x3'), directory / (operation + '.receipt.json')

    def submit(self, spec, state, *, now):
        self._authorized(spec, state, now)
        program, mandate, receipt_path = self._paths(state)
        prepared = state['prepared']
        _write_snapshot(program, prepared['source'])
        _write_snapshot(mandate, spec['mandates'][state['leg_index']])
        if not receipt_path.exists():
            prepare_live_quote(program, mandate=mandate, guard_contract=self.guard_contract,
                               receipt=receipt_path, explicit_live=True, client=self.client, now=now)
        if receipt_path.is_symlink():
            raise ValueError('receipt cannot be a symlink')
        receipt = ni._load_receipt(str(receipt_path))
        quote = validate_prepared_receipt(receipt, prepared['source'], prepared, spec['tokens'], now=now)
        if receipt['program_sha256'] != sha256(prepared['source'].encode()).hexdigest():
            raise ValueError('live receipt program differs from authorized source')
        # Recheck active on-chain mandate and rolling usage immediately before gateway use.
        check = ni.check_against_mandate(receipt['leg'], receipt['quote_request'], receipt['program_sha256'],
                    str(mandate), tokens=self.client.tokens(), guard_contract=self.guard_contract,
                    near_views=ni.near_views_for(ni.NearRpc()))
        if check['approvals_needed']:
            raise ValueError('owner approvals required; no autonomous approval')
        if int(ni.parse_timestamp(quote['deadline']).timestamp()) - int(time.time()) < 10:
            raise ValueError('signed funding quote expired')
        payload = {'cycle': state['identity'], 'operation': prepared['operation'],
                   'strategy_expires_at': spec['expires_at'], 'quote_hash': receipt['quote_hash'],
                   'program_source': prepared['source'], 'mandate_source': spec['mandates'][state['leg_index']],
                   'receipt': receipt, 'guard_contract': self.guard_contract,
                   'guard_calls': [{'method': ni.guard_method(receipt, step), 'args': ni.guard_args(receipt, step)}
                                   for step in ('register', 'quote')],
                   'max_cycle_external_cost_units': str(spec['max_external_cost_units']),
                   'minimum_cycle_profit_units': str(spec['minimum_profit_units']),
                   'funding': dict(receipt['deposit'])}
        result = self.gateway.request('authorize_and_fund', payload)
        if result.get('operation') != prepared['operation'] or result.get('quote_hash') != receipt['quote_hash']:
            raise ValueError('wallet funding response binding mismatch')
        if result.get('status') != 'submitted' or not isinstance(result.get('funding_tx'), str) or not result['funding_tx']:
            raise ValueError('wallet did not confirm submission; reconcile before any retry')
        funded_ms = uint(result.get('funded_at_ms'), 'funding time')
        if not int(ni.parse_timestamp(receipt['quote_response']['timestamp']).timestamp()*1000) <= funded_ms <= min(
                int(ni.parse_timestamp(quote['deadline']).timestamp()*1000), int(time.time()*1000)):
            raise ValueError('funding timestamp outside signed quote window')
        return result

    def observe(self, spec, state, *, now):
        # Observation is read-only and remains possible after approval/quote expiry.
        if self.gateway is None:
            raise ValueError('wallet gateway unavailable')
        result = self.gateway.request('observe', {'cycle': state['identity'],
                                      'operation': state['prepared']['operation']})
        if result.get('operation') != state['prepared']['operation']:
            raise ValueError('wallet observation operation mismatch')
        if result.get('status') in ('settled', 'refunded'):
            _, _, receipt_path = self._paths(state)
            if receipt_path.is_symlink():
                raise ValueError('receipt cannot be a symlink')
            receipt = ni._load_receipt(str(receipt_path))
            ni._signed_quote(receipt)
            if (result.get('quote_hash') != receipt['quote_hash'] or
                    result.get('program_sha256') != receipt['program_sha256'] or
                    result.get('recipient') != receipt['leg']['recipient'] or
                    result.get('refund_to') != receipt['leg']['refund_to'] or
                    result.get('amount_in') != str(receipt['deposit']['amount'])):
                raise ValueError('wallet settlement does not bind the authorized quote and receivers')
            leg = decode_leg(state['prepared']['leg'])
            wanted = leg.asset_out if result['status'] == 'settled' else leg.asset_in
            if result.get('asset_out') != wanted.to_json() or result.get('finality') != 'finalized':
                raise ValueError('wallet observation has wrong asset or lacks finality')
            uint(result.get('amount_out'), 'observed amount', positive=True)
            if not result.get('funding_tx') or not result.get('payout_tx'):
                raise ValueError('wallet observation lacks transaction references')
            funded_ms = uint(result.get('funded_at_ms'), 'funding time')
            earliest = int(ni.parse_timestamp(receipt['quote_response']['timestamp']).timestamp() * 1000)
            latest = int(ni.parse_timestamp(receipt['deposit']['deadline']).timestamp() * 1000)
            if not earliest <= funded_ms <= min(latest, int(time.time() * 1000)):
                raise ValueError('funding timestamp outside authorized quote window')
            result['evidence'] = 'pinned trusted-executor attestation; not solver causality proof'
        return result

    def inventory(self, spec, state):
        result = self.gateway.request('inventory', {'cycle': state['identity']})
        balances = result['balances']
        if not isinstance(balances, dict):
            raise ValueError('wallet inventory must be an asset map')
        return {asset: str(uint(amount, 'wallet balance')) for asset, amount in balances.items()}
