"""Real compiler + mandates + SQLite; all monetary execution is explicitly simulated."""
from dataclasses import replace
import pytest
from arbitrage.adapters import demo_routes
from arbitrage.trader import Trader, Strategy, PaperExchange

SOL = '4Nd1mzi8Y1QYxJt9wZWBYZpG7S4pYkZs6YzD3Vt9aBcD'


def mandate(owner, spend, destination, recipient, refund):
    return f'''mandate roundtrip {{
 owner {owner}
 agent trader.near
 spend {spend}
 per_payment 200000000
 per_24h 400000000
 allow recipients {recipient}
 allow assets {destination}
 refund_to {refund}
 require slippage <= 50 bps
 require fees <= 100 bps
 expires 1d
}}'''


def strategy(now=1000):
    routes, tokens = demo_routes(now=now)
    return Strategy(route=routes[0], tokens=tokens,
                    mandates=(mandate('sol-owner.near', 'Solana.USDC', 'Near.USDC', 'alice.near', SOL),
                              mandate('near-owner.near', 'Near.USDC', 'Solana.USDC', SOL, 'alice.near')),
                    addresses=(SOL, 'alice.near'), minimum_profit_units=1000000,
                    max_external_cost_units=500000, expires_at=now + 300)


def test_full_automatic_roundtrip_compiles_both_mandates_and_conserves_paper_inventory(tmp_path):
    trader = Trader(tmp_path / 'trading.sqlite', clock=lambda: 1000)
    trader.create('cycle-1', strategy())
    result = trader.run('cycle-1', PaperExchange(trader.store), max_steps=20)
    assert result['phase'] == 'COMPLETED'
    assert result['paper_profit_units'] == '2000000'
    assert result['real_transactions_submitted'] == 0
    assert result['mode'] == 'paper'
    assert len(result['completed_legs']) == 2
    assert result['inventory']['near/demo:near-usdc'] == '0'
    assert result['inventory']['sol/demo:sol-usdc'] == '102500000'
    assert result['initial_equity_units'] == '100500000'


@pytest.mark.parametrize('scenario,phase,completed', [
    ('return_loss', 'HOLDING', 1), ('outbound_refund', 'REFUNDED', 0),
    ('return_refund', 'HOLDING', 1), ('timeout', 'RESOLUTION_REQUIRED', 0),
    ('rpc_failure', 'RESOLUTION_REQUIRED', 0), ('submit_timeout', 'COMPLETED', 2),
])
def test_automatic_failure_paths_never_claim_atomic_rollback(tmp_path, scenario, phase, completed):
    trader = Trader(tmp_path / 'trading.sqlite', clock=lambda: 1000)
    trader.create('cycle', strategy(), scenario=scenario)
    result = trader.run('cycle', PaperExchange(trader.store), max_steps=20)
    assert result['phase'] == phase
    assert len(result['completed_legs']) == completed
    assert result['real_transactions_submitted'] == 0
    with trader.store._connect() as db:
        count = db.execute('SELECT count(*) FROM paper_operations').fetchone()[0]
    assert count <= (2 if completed == 2 or scenario == 'return_refund' else 1)


def test_crash_after_submit_commit_reconciles_without_double_payment(tmp_path):
    class CrashingExchange(PaperExchange):
        def submit(self, spec, state, *, now):
            super().submit(spec, state, now=now)
            raise SystemExit('simulated process death after wallet committed')
    path = tmp_path / 'trading.sqlite'
    trader = Trader(path, clock=lambda: 1000)
    trader.create('cycle', strategy())
    trader.tick('cycle', PaperExchange(trader.store))
    with pytest.raises(SystemExit):
        trader.tick('cycle', CrashingExchange(trader.store))
    restarted = Trader(path, clock=lambda: 1001)
    result = restarted.run('cycle', PaperExchange(restarted.store), max_steps=20)
    assert result['phase'] == 'COMPLETED'
    assert result['paper_profit_units'] == '2000000'
    with restarted.store._connect() as db:
        assert db.execute('SELECT count(*) FROM paper_operations').fetchone()[0] == 2


def test_revocation_between_quote_and_submit_stops_all_funding(tmp_path):
    trader = Trader(tmp_path / 'trading.sqlite', clock=lambda: 1000)
    initial = trader.create('cycle', strategy())
    trader.tick('cycle', PaperExchange(trader.store))
    trader.store.revoke(initial['mandate_ids'][0])
    result = trader.run('cycle', PaperExchange(trader.store))
    assert result['phase'] == 'REJECTED'
    assert 'mandate_active' in result['reason']
    assert result['inventory']['sol/demo:sol-usdc'] == '100500000'


def test_return_uses_actual_overfill_not_initial_indicative_amount(tmp_path):
    trader = Trader(tmp_path / 'trading.sqlite', clock=lambda: 1000)
    trader.create('cycle', strategy(), scenario='outbound_overfill')
    result = trader.run('cycle', PaperExchange(trader.store))
    assert result['phase'] == 'COMPLETED'
    assert result['completed_legs'][0]['amount_out'] == '102000000'
    assert result['completed_legs'][1]['quote']['amount_in'] == '102000000'
    assert result['inventory']['near/demo:near-usdc'] == '0'


def test_create_replay_after_expiry_returns_original_cycle(tmp_path):
    now = [1000]
    trader = Trader(tmp_path / 'trading.sqlite', clock=lambda: now[0])
    trader.create('cycle', strategy())
    first = trader.run('cycle', PaperExchange(trader.store))
    now[0] = 2000
    assert trader.create('cycle', strategy()) == first
    with pytest.raises(ValueError, match='idempotency'):
        trader.create('cycle', replace(strategy(), minimum_profit_units=2))


def test_initial_profit_floor_refused_before_outbound_inventory_is_spent(tmp_path):
    trader = Trader(tmp_path / 'trading.sqlite', clock=lambda: 1000)
    with pytest.raises(ValueError, match='profit'):
        trader.create('cycle', replace(strategy(), minimum_profit_units=3000000))


def test_shared_rolling_budget_does_not_reset_for_each_cycle(tmp_path):
    trader = Trader(tmp_path / 'trading.sqlite', clock=lambda: 1000)
    restrictive = tuple(text.replace('per_24h 400000000', 'per_24h 200000000') for text in strategy().mandates)
    policy = replace(strategy(), mandates=restrictive)
    trader.create('first', policy)
    assert trader.run('first', PaperExchange(trader.store))['phase'] == 'COMPLETED'
    with pytest.raises(ValueError, match='mandate_daily'):
        trader.create('second', policy)
    with trader.store._connect() as db:
        assert db.execute('SELECT count(*) FROM paper_operations').fetchone()[0] == 2


def test_live_adapter_is_disarmed_before_any_network_or_funding(tmp_path):
    from arbitrage.live import LiveExchange
    class NoCalls:
        def __getattr__(self, name):
            raise AssertionError('no real network, keys, signing or funds allowed')
    source = strategy()
    real_route = replace(source.route, legs=tuple(replace(leg, evidence='provider_dry_quote') for leg in source.route.legs))
    trader = Trader(tmp_path / 'trading.sqlite', clock=lambda: 1000)
    trader.create('live', replace(source, route=real_route), mode='live')
    result = trader.run('live', LiveExchange(client=NoCalls()))
    assert result['phase'] == 'DISARMED'
    assert result['completed_legs'] == []


def test_backend_mode_cannot_be_swapped_to_fund_a_paper_cycle(tmp_path):
    from arbitrage.live import LiveExchange
    trader = Trader(tmp_path / 'trading.sqlite', clock=lambda: 1000)
    trader.create('cycle', strategy())
    with pytest.raises(ValueError, match='modes differ'):
        trader.run('cycle', LiveExchange())


def test_resuming_outage_reconciles_without_new_funding_operation(tmp_path):
    class RecoveredExchange(PaperExchange):
        def observe(self, spec, state, *, now):
            return super().observe(dict(spec, scenario='success'), state, now=now)
    trader = Trader(tmp_path / 'trading.sqlite', clock=lambda: 1000)
    trader.create('cycle', strategy(), scenario='rpc_failure')
    first = trader.run('cycle', PaperExchange(trader.store))
    assert first['phase'] == 'RESOLUTION_REQUIRED'
    trader.resume_observation('cycle')
    result = trader.run('cycle', RecoveredExchange(trader.store))
    assert result['phase'] == 'COMPLETED'
    with trader.store._connect() as db:
        assert db.execute('SELECT count(*) FROM paper_operations').fetchone()[0] == 2


def test_inventory_observation_failure_never_duplicates_completed_legs(tmp_path):
    class FlakyInventory(PaperExchange):
        failures = 2
        def inventory(self, spec, state):
            if self.failures:
                self.failures -= 1
                raise OSError('temporary inventory outage')
            return super().inventory(spec, state)
    trader = Trader(tmp_path / 'trading.sqlite', clock=lambda: 1000)
    trader.create('cycle', strategy())
    result = trader.run('cycle', FlakyInventory(trader.store), max_steps=20)
    assert result['phase'] == 'COMPLETED'
    assert len(result['completed_legs']) == 2


def test_confirmed_live_backend_usage_is_durable_without_real_wallet_calls(tmp_path):
    # Only the coordinator's live-backend contract is simulated here; no LiveExchange,
    # wallet gateway, chain RPC, deposit quote or signer is invoked.
    from arbitrage.trader import decode_leg
    class SimulatedLiveBoundary(PaperExchange):
        mode = 'live'
        def submit(self, spec, state, *, now):
            return {'status': 'submitted', 'operation': state['prepared']['operation'], 'funded_at_ms': now*1000}
        def observe(self, spec, state, *, now):
            leg = decode_leg(state['prepared']['leg'])
            return {'status': 'settled', 'operation': state['prepared']['operation'],
                    'asset_out': leg.asset_out.to_json(), 'amount_out': str(leg.minimum_out),
                    'funded_at_ms': now*1000}
        def inventory(self, spec, state):
            return {'sol/demo:sol-usdc': '102500000', 'near/demo:near-usdc': '0'}
    trader = Trader(tmp_path / 'trading.sqlite', clock=lambda: 1000)
    original = strategy()
    route = replace(original.route, legs=tuple(replace(leg, evidence='provider_dry_quote') for leg in original.route.legs))
    policies = tuple(source.replace('per_24h 400000000', 'per_24h 200000000') for source in original.mandates)
    configured = replace(original, route=route, mandates=policies)
    trader.create('first', configured, mode='live')
    assert trader.run('first', SimulatedLiveBoundary(trader.store))['phase'] == 'COMPLETED'
    with trader.store._connect() as db:
        assert db.execute('SELECT count(*) FROM trade_spends').fetchone()[0] == 2
        assert db.execute('SELECT count(*) FROM trade_reservations').fetchone()[0] == 0
    with pytest.raises(ValueError, match='mandate_daily'):
        trader.create('second', configured, mode='live')


def test_terminal_cleanup_and_state_transition_are_atomic(tmp_path):
    import sqlite3
    trader = Trader(tmp_path / 'trading.sqlite', clock=lambda: 1000)
    trader.create('cycle', strategy(), scenario='return_loss')
    backend = PaperExchange(trader.store)
    for _ in range(3): trader.tick('cycle', backend)
    assert trader.get('cycle')['leg_index'] == 1
    with trader.store._connect() as db:
        db.execute("CREATE TRIGGER fail_cleanup BEFORE DELETE ON trade_reservations BEGIN SELECT RAISE(ABORT,'simulated disk error'); END")
    with pytest.raises(sqlite3.IntegrityError):
        trader.tick('cycle', backend)
    assert trader.get('cycle')['phase'] == 'QUOTE'
    with trader.store._connect() as db: db.execute('DROP TRIGGER fail_cleanup')
    assert trader.run('cycle', backend)['phase'] == 'HOLDING'
    with trader.store._connect() as db:
        assert db.execute('SELECT count(*) FROM trade_reservations').fetchone()[0] == 0


def test_real_cli_can_continue_an_immutable_cycle_after_restart(tmp_path):
    import json, os, subprocess, sys
    path = tmp_path / 'cli.sqlite'
    environment = dict(os.environ, PYTHONPATH='x3-lang')
    start = subprocess.run([sys.executable, '-m', 'arbitrage', 'trade', '--demo', '--key', 'cli',
                            '--database', str(path), '--steps', '2'], env=environment,
                           text=True, capture_output=True)
    assert start.returncode == 3
    assert json.loads(start.stdout)['phase'] == 'SUBMITTED'
    finish = subprocess.run([sys.executable, '-m', 'arbitrage', 'trade-continue', '--key', 'cli',
                             '--database', str(path)], env=environment, text=True, capture_output=True)
    assert finish.returncode == 0, finish.stdout + finish.stderr
    result = json.loads(finish.stdout)
    assert result['phase'] == 'COMPLETED'
    assert result['paper_profit_units'] == '2000000'
