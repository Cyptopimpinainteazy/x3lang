"""Automatic two-leg state machine. Paper and live backends use the same policy path.

Paper mode simulates transfers, not settlement proof. Live custody is an explicitly armed,
trusted wallet gateway; no keys or automatic funding permissions are provided by this module.
"""
import fcntl
import json
import os
import re
import tempfile
import time
from dataclasses import asdict, dataclass
from copy import deepcopy
from datetime import datetime, timezone
from hashlib import sha256
from pathlib import Path

import near_intents as ni
from near_liquidity.quote import AssetRef, canonical_json
from oneclick_catalog import catalog

from .engine import Leg, Route, assess, uint, leg_json
from .execution import compile_leg
from .trading_store import TradingStore

TERMINAL = {'COMPLETED', 'REJECTED', 'HOLDING', 'REFUNDED', 'RESOLUTION_REQUIRED', 'DISARMED'}


def decode_leg(document):
    fields = dict(document)
    for side in ('asset_in', 'asset_out'):
        fields[side] = AssetRef(**fields[side])
    for field in ('amount_in', 'expected_out', 'minimum_out'):
        fields[field] = uint(fields[field], field, positive=True)
    return Leg(**fields)


@dataclass(frozen=True)
class Strategy:
    route: Route
    tokens: list
    mandates: tuple
    addresses: tuple
    minimum_profit_units: int
    max_external_cost_units: int
    expires_at: int

    def document(self):
        return asdict(self)


class FundingDisabled(ValueError):
    """A definitive refusal before any funding operation, never an ambiguous broadcast."""


def compile_policy(source, tokens, now):
    if not isinstance(source, str) or not 1 <= len(source.encode()) <= 16384:
        raise ValueError('bounded .x3 mandate source required')
    with tempfile.TemporaryDirectory(prefix='x3-arb-mandate-') as directory:
        path = Path(directory) / 'mandate.x3'
        path.write_text(source, encoding='utf-8')
        compiled = ni.mandates().compile_mandate(str(path), tokens,
                    now=datetime.fromtimestamp(now, timezone.utc))
    identity = sha256(canonical_json([compiled['owner'], compiled['agent'], compiled['terms']['asset'],
                                     compiled['terms']['program_sha256']])).hexdigest()
    return {'id': identity, 'source': source, 'compiled': compiled}


def intent_policy(source, tokens, now):
    with tempfile.TemporaryDirectory(prefix='x3-arb-intent-') as directory:
        path = Path(directory) / 'leg.x3'
        path.write_text(source, encoding='utf-8')
        _, leg = ni.compile_program(str(path), now=datetime.fromtimestamp(now, timezone.utc))
    request = ni.build_quote_request(leg, tokens, dry=True)
    return ni.policy_for(leg, request, sha256(source.encode()).hexdigest()), leg


def check_policy(store, mandate_id, policy, now, *, db=None, operation=None):
    m = ni.mandates()
    mandate = store.mandate(mandate_id, db=db, exclude_operation=operation)
    checks = m.check_mandate(mandate, policy, now * 1000)
    failed = [check['guard'] for check in checks if not check['ok']]
    if failed:
        raise ValueError('mandate denied: ' + ', '.join(failed))
    if m.approvals_needed(mandate, int(policy['amount'])):
        raise ValueError('owner approvals required; autonomous approval is forbidden')
    return checks


class Trader:
    def __init__(self, database, *, clock=time.time):
        self.store, self.clock = TradingStore(database), clock

    def create(self, key, strategy, *, mode='paper', scenario='success'):
        if not isinstance(key, str) or not re.fullmatch(r'[A-Za-z0-9_-]{1,100}', key):
            raise ValueError('invalid cycle key')
        if mode not in ('paper', 'live'):
            raise ValueError('invalid trading mode')
        now = int(self.clock())
        if not isinstance(strategy, Strategy) or len(strategy.route.legs) != 2 or strategy.route.kind != 'cross-chain':
            raise ValueError('exactly two cross-chain legs required')
        if scenario not in {'success', 'return_loss', 'outbound_refund', 'return_refund',
                            'timeout', 'rpc_failure', 'submit_timeout', 'outbound_overfill'}:
            raise ValueError('unknown execution scenario')
        if mode == 'live' and scenario != 'success':
            raise ValueError('failure scenarios belong to paper mode')
        spec = strategy.document()
        spec.update(mode=mode, scenario=scenario)
        if self.store.exists(key, spec):
            return self.get(key)
        for name in ('minimum_profit_units', 'max_external_cost_units', 'expires_at'):
            value = getattr(strategy, name)
            if type(value) is not int:
                raise ValueError(name + ' must be an integer')
            uint(value, name)
        if strategy.minimum_profit_units <= 0 or strategy.expires_at <= now:
            raise ValueError('positive minimum profit and future expiry required')
        result = assess(strategy.route, now=now)
        if not result['paper_eligible']:
            raise ValueError('route refused: ' + ', '.join(result['reasons']))
        if (strategy.route.legs[-1].minimum_out - strategy.route.legs[0].amount_in -
                strategy.max_external_cost_units < strategy.minimum_profit_units):
            raise ValueError('initial round-trip profit floor cannot be met')
        if strategy.route.external_cost_units > strategy.max_external_cost_units:
            raise ValueError('external cost budget exceeded')
        if len(strategy.mandates) != 2 or len(strategy.addresses) != 2:
            raise ValueError('two mandates and two wallet addresses required')
        catalog(strategy.tokens)
        policies = [compile_policy(source, strategy.tokens, now) for source in strategy.mandates]
        if ((policies[0]['compiled']['owner'], policies[0]['compiled']['agent']) ==
                (policies[1]['compiled']['owner'], policies[1]['compiled']['agent'])):
            raise ValueError('distinct owner/agent mandates required for opposite spending assets')
        if mode == 'live' and strategy.route.demo:
            raise ValueError('constructed demo route cannot be live')
        # Spec fingerprints omit compile-time absolute expiry; the registered mandate never
        # gets a fresh lifetime merely because the runner restarted or another cycle started.
        spec = strategy.document()
        spec.update(mode=mode, scenario=scenario)
        identity = sha256(canonical_json([key, spec])).hexdigest()
        compiled_sources = []
        for index, leg in enumerate(strategy.route.legs):
            source = compile_leg(leg, cycle_id=identity, index=index, tokens=strategy.tokens,
                                 refund_to=strategy.addresses[index], recipient=strategy.addresses[1-index],
                                 mandate_owner=policies[index]['compiled']['owner'], now=now)
            policy, _ = intent_policy(source, strategy.tokens, now)
            checks = ni.mandates().check_mandate(policies[index]['compiled'], policy, now * 1000)
            if not all(check['ok'] for check in checks) or ni.mandates().approvals_needed(policies[index]['compiled'], leg.amount_in):
                raise ValueError('initial .x3 mandate preflight refused')
            compiled_sources.append(source)
        initial = strategy.route.legs[0].amount_in
        origin = strategy.route.legs[0].asset_in.key
        inventory = {origin: str(initial + strategy.max_external_cost_units),
                     strategy.route.legs[0].asset_out.key: '0'}
        state = {'id': key, 'route_id': strategy.route.id, 'identity': identity, 'phase': 'QUOTE', 'mode': mode, 'leg_index': 0,
                 'completed_legs': [], 'funded_operations': [], 'prepared': None, 'polls': 0,
                 'mandate_ids': [p['id'] for p in policies], 'programs': compiled_sources,
                 'initial_equity_units': inventory[origin], 'profit_asset': strategy.route.legs[0].asset_in.to_json(), 'paper_profit_units': None,
                 'real_transactions_submitted': 0 if mode == 'paper' else None, 'inventory': inventory, 'reason': None,
                 'simulation_only': mode == 'paper'}
        def reserve_both(db):
            for index, source in enumerate(compiled_sources):
                policy, _ = intent_policy(source, strategy.tokens, now)
                operation = sha256(canonical_json([identity, index])).hexdigest()
                check_policy(self.store, policies[index]['id'], policy, now, db=db, operation=operation)
                db.execute('INSERT INTO trade_reservations VALUES(?,?,?,?,?)',
                           (operation, key, policies[index]['id'], policy['amount'], now * 1000))
        self.store.create(key, spec, state, policies, authorize=reserve_both)
        return self.get(key)

    def get(self, key):
        _, state = self.store.load(key)
        if state['mode'] == 'paper':
            state['inventory'] = self.store.inventory(key)
        return state

    def cycles(self):
        return [self.get(state['id']) for state in self.store.cycles()]

    def resume_observation(self, key):
        """Resume bounded read-only reconciliation; never reset an operation identity."""
        descriptor = os.open(str(self.store.path) + '.runner.lock',
                             os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW, 0o600)
        try:
            fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
            _, state = self.store.load(key)
            if state['phase'] != 'RESOLUTION_REQUIRED' or not state.get('prepared'):
                raise ValueError('only unresolved submitted operations can resume observation')
            state.update(phase='SUBMISSION_UNKNOWN', polls=0, reason=None)
            self.store.save(key, state)
        finally:
            os.close(descriptor)
        return self.get(key)

    def _program(self, spec, state, leg, now):
        index = state['leg_index']
        owner = self.store.mandate(state['mandate_ids'][index])['owner']
        return compile_leg(leg, cycle_id=state['identity'], index=index, tokens=spec['tokens'],
                           refund_to=spec['addresses'][index], recipient=spec['addresses'][1-index],
                           mandate_owner=owner, now=now)

    def tick(self, key, backend):
        # A short-lived OS lock serializes runners across processes. No SQLite transaction
        # is held across RPC. A persisted SUBMITTING state is reconciled after a crash.
        lock_path = str(self.store.path) + '.runner.lock'
        descriptor = os.open(lock_path, os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW, 0o600)
        try:
            try:
                fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except BlockingIOError:
                return self.get(key)
            spec, state = self.store.load(key)
            if backend.mode != state['mode']:
                raise ValueError('execution backend and cycle modes differ')
            if state['phase'] in TERMINAL:
                self.store.save(key, state, terminal=state['phase'] != 'RESOLUTION_REQUIRED')
                return self.get(key)
            now = int(self.clock())
            index = state['leg_index']
            if state['phase'] in ('QUOTE', 'PREPARED') and now >= spec['expires_at']:
                state.update(phase='HOLDING' if index else 'REJECTED', reason='strategy expired')
            elif state['phase'] == 'QUOTE':
                try:
                    amount = uint(spec['route']['legs'][0]['amount_in'], 'initial') if index == 0 else uint(
                        state['completed_legs'][0]['amount_out'], 'settled inventory', positive=True)
                    required = (uint(spec['route']['legs'][0]['minimum_out'], 'outbound floor') if index == 0 else
                                uint(spec['route']['legs'][0]['amount_in'], 'initial') +
                                spec['max_external_cost_units'] + spec['minimum_profit_units'])
                    leg = backend.quote(spec, state, amount, now=now)
                    intended = decode_leg(spec['route']['legs'][index])
                    if (leg.asset_in, leg.asset_out, leg.amount_in) != (intended.asset_in, intended.asset_out, amount):
                        raise ValueError('fresh quote changed chain, asset, decimals or input amount')
                    if leg.minimum_out < required:
                        raise ValueError('fresh quote does not meet delivery/profit floor; holding inventory')
                    source = self._program(spec, state, leg, now)
                    policy, _ = intent_policy(source, spec['tokens'], now)
                    operation = sha256(canonical_json([state['identity'], index])).hexdigest()
                    with self.store._connect() as db:
                        db.execute('BEGIN IMMEDIATE')
                        check_policy(self.store, state['mandate_ids'][index], policy, now, db=db, operation=operation)
                        db.execute('UPDATE trade_reservations SET amount=?,at_ms=? WHERE operation=?',
                                   (policy['amount'], now*1000, operation))
                    state['prepared'] = {'leg': leg_json(leg), 'source': source, 'policy': policy,
                                         'operation': operation, 'mandate_id': state['mandate_ids'][index]}
                    state['programs'][index] = source
                    state.update(phase='PREPARED', polls=0)
                except FundingDisabled as exc:
                    state.update(phase='DISARMED', reason=str(exc))
                except (ValueError, ni.NearIntentsError) as exc:
                    state.update(phase='HOLDING' if index else 'REJECTED', reason=str(exc)[:300])
                except Exception:
                    state.update(phase='HOLDING' if index else 'REJECTED', reason='fresh quote unavailable; no funding submitted')
            elif state['phase'] == 'PREPARED':
                try:
                    prepared = state['prepared']
                    leg = decode_leg(prepared['leg'])
                    if not 0 <= now - leg.observed_at <= 30 or leg.expires_at - now < 10:
                        raise ValueError('prepared quote expired; no funding submitted')
                    check_policy(self.store, prepared['mandate_id'], prepared['policy'], now, operation=prepared['operation'])
                    state['phase'] = 'SUBMITTING'
                    self.store.save(key, state)
                    submission = backend.submit(spec, state, now=now)
                    if state['mode'] == 'live':
                        if submission['operation'] != prepared['operation'] or submission['status'] != 'submitted':
                            raise ValueError('live submission response does not bind this operation')
                        self._confirmed_funding(state, submission['funded_at_ms'])
                    state['phase'] = 'SUBMITTED'
                except FundingDisabled as exc:
                    state.update(phase='DISARMED', reason=str(exc))
                except (ValueError, ni.NearIntentsError) as exc:
                    if state['phase'] == 'SUBMITTING':
                        state.update(phase='SUBMISSION_UNKNOWN', reason='submission requires reconciliation')
                    else:
                        state.update(phase='HOLDING' if index else 'REJECTED', reason=str(exc)[:300])
                except Exception:
                    state.update(phase='SUBMISSION_UNKNOWN', reason='submission requires reconciliation')
            elif state['phase'] in ('SUBMITTING', 'SUBMITTED', 'SUBMISSION_UNKNOWN'):
                if state['mode'] == 'live' and now < state.get('next_poll_at', 0):
                    return self.get(key)
                try:
                    observation = backend.observe(spec, state, now=now)
                    if observation['operation'] != state['prepared']['operation']:
                        raise ValueError('observation operation mismatch')
                    status = observation['status']
                    if status == 'not_submitted':
                        state.update(phase='PREPARED', reason=None)
                    elif status == 'settled':
                        leg = decode_leg(state['prepared']['leg'])
                        if observation['asset_out'] != leg.asset_out.to_json():
                            raise ValueError('settlement asset mismatch')
                        amount = uint(observation['amount_out'], 'settled amount', positive=True)
                        if amount < leg.minimum_out:
                            raise ValueError('settled output is below the compiled floor')
                        next_state = deepcopy(state)
                        next_state['inventory'] = backend.inventory(spec, state)
                        if not isinstance(next_state['inventory'], dict):
                            raise ValueError('inventory observation must be an asset map')
                        for balance in next_state['inventory'].values():
                            uint(balance, 'observed inventory')
                        next_state['completed_legs'].append(observation)
                        if state['mode'] == 'live':
                            self._confirmed_funding(next_state, observation['funded_at_ms'])
                        if index == 0:
                            next_state.update(leg_index=1, phase='QUOTE', prepared=None, polls=0, next_poll_at=0, reason=None)
                        else:
                            next_state.update(phase='COMPLETED', reason=None)
                            if state['mode'] == 'paper':
                                origin = decode_leg(spec['route']['legs'][0]).asset_in.key
                                next_state['paper_profit_units'] = str(int(next_state['inventory'][origin]) - int(state['initial_equity_units']))
                        state = next_state
                    elif status == 'refunded':
                        inventory = backend.inventory(spec, state)
                        state.update(phase='HOLDING' if index else 'REFUNDED', reason='confirmed refund; previous settled legs remain committed', inventory=inventory)
                        if state['mode'] == 'live':
                            self._confirmed_funding(state, observation['funded_at_ms'])
                    elif status in ('pending', 'unknown'):
                        state['polls'] += 1
                        if state['mode'] == 'live':
                            state['next_poll_at'] = now + 5
                        if state['polls'] >= (4 if state['mode'] == 'paper' else 60):
                            state.update(phase='RESOLUTION_REQUIRED', reason='bounded polling exhausted; never resubmit an ambiguous transfer')
                    else:
                        raise ValueError('unknown execution status')
                except Exception:
                    state['polls'] += 1
                    if state['mode'] == 'live':
                        state['next_poll_at'] = now + 5
                    if state['polls'] >= (4 if state['mode'] == 'paper' else 60):
                        state.update(phase='RESOLUTION_REQUIRED', reason='observation failed; no return funding permitted')
            self.store.save(key, state, terminal=state['phase'] in TERMINAL - {'RESOLUTION_REQUIRED'})
            return self.get(key)
        finally:
            os.close(descriptor)

    @staticmethod
    def _confirmed_funding(state, at_ms):
        prepared = state['prepared']
        operation = prepared['operation']
        if not any(item['operation'] == operation for item in state['funded_operations']):
            state['funded_operations'].append({'operation': operation, 'mandate': prepared['mandate_id'],
                'amount': prepared['policy']['amount'], 'at_ms': uint(at_ms, 'funding time')})
        state['real_transactions_submitted'] = len(state['funded_operations'])

    def run(self, key, backend, *, max_steps=16):
        if type(max_steps) is not int or not 1 <= max_steps <= 100:
            raise ValueError('max_steps must be 1..100')
        for _ in range(max_steps):
            result = self.tick(key, backend)
            if result['phase'] in TERMINAL or result.get('next_poll_at', 0) > int(self.clock()):
                return result
        return self.get(key)


class PaperExchange:
    """Real SQLite inventory/operation commits; only market execution is simulated."""
    mode = 'paper'
    def __init__(self, store):
        self.store = store

    def quote(self, spec, state, amount, *, now):
        leg = decode_leg(spec['route']['legs'][state['leg_index']])
        expected = leg.expected_out * amount // leg.amount_in
        floor = leg.minimum_out * amount // leg.amount_in
        if spec['scenario'] == 'return_loss' and state['leg_index'] == 1:
            expected = floor = int(spec['route']['legs'][0]['amount_in']) - 1
        return Leg(leg.provider, leg.asset_in, leg.asset_out, amount, expected, floor,
                   now, now + 60, 'constructed_demo', 'simulated fresh quote')

    def submit(self, spec, state, *, now):
        prepared = state['prepared']
        operation, policy = prepared['operation'], prepared['policy']
        fingerprint = sha256(canonical_json(prepared)).hexdigest()
        leg = decode_leg(prepared['leg'])
        with self.store._connect() as db:
            db.execute('BEGIN IMMEDIATE')
            prior = db.execute('SELECT fingerprint FROM paper_operations WHERE id=?', (operation,)).fetchone()
            if prior:
                if prior[0] != fingerprint:
                    raise ValueError('operation id reused for different terms')
                return
            check_policy(self.store, prepared['mandate_id'], policy, now, db=db, operation=operation)
            balance = int(db.execute('SELECT amount FROM paper_inventory WHERE cycle=? AND asset=?',
                                     (state['id'], leg.asset_in.key)).fetchone()[0])
            cost = int(spec['route']['external_cost_units']) if state['leg_index'] == 0 else 0
            if balance < leg.amount_in + cost:
                raise ValueError('insufficient paper balance including external costs')
            db.execute('UPDATE paper_inventory SET amount=? WHERE cycle=? AND asset=?',
                       (str(balance - leg.amount_in - cost), state['id'], leg.asset_in.key))
            document = {'operation': operation, 'status': 'submitted', 'cycle': state['id'],
                        'leg': state['leg_index'], 'quote': prepared['leg']}
            db.execute('INSERT INTO paper_operations VALUES(?,?,?)', (operation, fingerprint, json.dumps(document)))
            db.execute('DELETE FROM trade_reservations WHERE operation=?', (operation,))
            db.execute('INSERT INTO trade_spends VALUES(?,?,?,?)',
                       (operation, prepared['mandate_id'], str(leg.amount_in), now * 1000))
        if spec['scenario'] == 'submit_timeout':
            raise TimeoutError('simulated lost response after commit')

    def observe(self, spec, state, *, now):
        operation = state['prepared']['operation']
        with self.store._connect() as db:
            db.execute('BEGIN IMMEDIATE')
            row = db.execute('SELECT document FROM paper_operations WHERE id=?', (operation,)).fetchone()
            if row is None:
                return {'operation': operation, 'status': 'not_submitted'}
            document = json.loads(row[0])
            if document['status'] in ('settled', 'refunded'):
                return document
            if spec['scenario'] == 'rpc_failure':
                raise OSError('simulated RPC outage')
            if spec['scenario'] == 'timeout':
                return {'operation': operation, 'status': 'pending'}
            leg = decode_leg(document['quote'])
            refund = spec['scenario'] == ('outbound_refund' if state['leg_index'] == 0 else 'return_refund')
            asset = leg.asset_in if refund else leg.asset_out
            amount = max(0, leg.amount_in - 100000) if refund else leg.minimum_out
            if spec['scenario'] == 'outbound_overfill' and state['leg_index'] == 0:
                amount = leg.expected_out
            old = db.execute('SELECT amount FROM paper_inventory WHERE cycle=? AND asset=?',
                             (state['id'], asset.key)).fetchone()
            balance = int(old[0]) if old else 0
            db.execute('INSERT INTO paper_inventory(cycle,asset,amount) VALUES(?,?,?) '
                       'ON CONFLICT(cycle,asset) DO UPDATE SET amount=excluded.amount',
                       (state['id'], asset.key, str(balance + amount)))
            document.update(status='refunded' if refund else 'settled', amount_out=str(amount),
                            asset_out=asset.to_json(), evidence='simulated SQLite wallet settlement',
                            observed_at=now)
            db.execute('UPDATE paper_operations SET document=? WHERE id=?', (json.dumps(document), operation))
            return document

    def inventory(self, spec, state):
        return self.store.inventory(state['id'])


def example_strategy(route, tokens, *, now):
    """Constructed paper-only policy fixture; never a source of live wallet authority."""
    if not route.demo:
        raise ValueError('generated example mandates are for constructed demo routes only')
    addresses = {'sol': '4Nd1mzi8Y1QYxJt9wZWBYZpG7S4pYkZs6YzD3Vt9aBcD',
                 'near': 'alice.near', 'eth': '0x1111111111111111111111111111111111111111'}
    wallets = tuple(addresses[leg.asset_in.network] for leg in route.legs)
    inverse = {network: chain for chain, network in ni.CHAIN_TO_BLOCKCHAIN.items()}
    sources = []
    for index, leg in enumerate(route.legs):
        endpoints = []
        for asset in (leg.asset_in, leg.asset_out):
            token = [t for t in tokens if t['assetId'] == asset.asset_id and t['blockchain'] == asset.network]
            if len(token) != 1:
                raise ValueError('ambiguous demo token')
            endpoints.append(inverse[asset.network] + '.' + token[0]['symbol'])
        sources.append(f'''mandate automatic_paper_leg_{index} {{
 owner paper-{leg.asset_in.network}-owner.near
 agent paper-trader.near
 spend {endpoints[0]}
 per_payment {leg.amount_in * 2}
 per_24h {leg.amount_in * 4}
 allow recipients {wallets[1-index]}
 allow assets {endpoints[1]}
 refund_to {wallets[index]}
 require slippage <= 50 bps
 require fees <= 100 bps
 expires 1d
}}''')
    return Strategy(route, tokens, tuple(sources), wallets, 1000000,
                    route.external_cost_units, now + 300)


def strategy_from_document(document):
    if not isinstance(document, dict) or set(document) != {
            'route', 'tokens', 'mandates', 'addresses', 'minimum_profit_units',
            'max_external_cost_units', 'expires_at'}:
        raise ValueError('explicit route, tokens, mandates, addresses and profit/cost/expiry limits required')
    route = dict(document['route'])
    route['legs'] = tuple(decode_leg(leg) for leg in route['legs'])
    return Strategy(Route(**route), document['tokens'], tuple(document['mandates']),
                    tuple(document['addresses']), uint(document['minimum_profit_units'], 'minimum profit'),
                    uint(document['max_external_cost_units'], 'cost budget'),
                    uint(document['expires_at'], 'strategy expiry'))
