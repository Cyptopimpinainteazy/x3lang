"""Public scanner seams: route economics, persistent paper orders, DSL export."""
import pytest
from near_liquidity.quote import AssetRef
from arbitrage.engine import Leg, Route, assess

USDC = AssetRef('sol', 'usdc', 6)
NEAR = AssetRef('near', 'usdc.near', 6)


def route():
    return Route('worked-example', 'cross-chain', (
        Leg('oneclick', USDC, NEAR, 100_000_000, 102_000_000, 101_000_000, 100, 160),
        Leg('oneclick', NEAR, USDC, 101_000_000, 103_000_000, 102_500_000, 101, 161),
    ), 500_000, 'operator estimate', False)


def test_closed_cycle_profit_uses_floors_and_external_costs():
    result = assess(route(), now=110)
    assert result['net_profit_units'] == '2000000'
    assert result['net_profit_bps'] == 200
    assert result['eligible_for_wallet_authorization'] is False
    assert result['paper_eligible'] is True

from dataclasses import replace
from arbitrage.paper import PaperLedger


@pytest.mark.parametrize('change,code', [
    ({'external_cost_units': None}, 'UNKNOWN_EXTERNAL_COSTS'),
    ({'external_cost_units': 3_000_000}, 'BELOW_PROFIT_THRESHOLD'),
])
def test_unfunded_profit_is_not_paper_eligible(change, code):
    result = assess(replace(route(), **change), now=110)
    assert not result['paper_eligible']
    assert code in result['reasons']


def test_stale_and_disconnected_quotes_fail_closed():
    r = route()
    mismatched = replace(r.legs[1], asset_in=AssetRef('eth', 'usdc', 6))
    result = assess(replace(r, legs=(r.legs[0], mismatched)), now=150)
    assert {'STALE_QUOTE', 'ASSET_OR_CHAIN_MISMATCH'} <= set(result['reasons'])


def test_paper_is_idempotent_after_restart_and_never_claims_a_fill(tmp_path):
    path = tmp_path / 'paper.sqlite'
    first = PaperLedger(path).project(route(), key='order-1', now=110)
    assert first['status'] == 'projected'
    assert first['simulation_only'] is True
    assert PaperLedger(path).project(route(), key='order-1', now=200) == first
    assert len(PaperLedger(path).orders()) == 1
    with pytest.raises(ValueError, match='idempotency'):
        PaperLedger(path).project(replace(route(), external_cost_units=100), key='order-1', now=110)


def test_expired_paper_order_is_rejected(tmp_path):
    order = PaperLedger(tmp_path / 'paper.sqlite').project(route(), key='late', now=200)
    assert order['status'] == 'rejected'
    assert order['assessment']['paper_eligible'] is False

from arbitrage.execution import export_leg, prepare_live_quote

SOL_ADDR = '4Nd1mzi8Y1QYxJt9wZWBYZpG7S4pYkZs6YzD3Vt9aBcD'


def test_export_compiles_with_exact_asset_binding(tmp_path):
    import near_intents as ni
    tokens = [
        {'assetId': 'usdc', 'blockchain': 'sol', 'symbol': 'USDC', 'decimals': 6},
        {'assetId': 'usdc.near', 'blockchain': 'near', 'symbol': 'USDC', 'decimals': 6},
    ]
    source = export_leg(route(), 0, tokens=tokens, refund_to=SOL_ADDR,
                        recipient='alice.near', now=110)
    path = tmp_path / 'route.x3'
    path.write_text(source)
    _, leg = ni.compile_program(str(path))
    assert leg['amount'] == 100000000
    assert leg['min_receive'] == 101000000
    assert 'not atomic' in source
    with pytest.raises(ValueError, match='asset'):
        export_leg(route(), 0, tokens=[dict(tokens[0], assetId='other'), tokens[1]],
                   refund_to=SOL_ADDR, recipient='alice.near', now=110)


def test_live_quote_requires_explicit_opt_in_before_any_network(tmp_path):
    with pytest.raises(ValueError, match='explicit'):
        prepare_live_quote('missing.x3', mandate='missing-mandate.x3',
                           guard_contract='guard.near', receipt=tmp_path / 'receipt.json')

from arbitrage.adapters import fill_book, demo_routes, scan_cross_chain


def test_orderbook_walks_depth_and_rejects_insufficient_liquidity():
    # $10 buys 2 units at $2 and 2 at $3, with integer rounding.
    assert fill_book([['2', '2'], ['3', '9']], 10_000_000, buy=True, decimals=0) == 4
    assert fill_book([['3', '2'], ['2', '9']], 4, buy=False, decimals=0) == 10_000_000
    with pytest.raises(ValueError, match='depth'):
        fill_book([['2', '2']], 10_000_000, buy=True, decimals=0)


def test_demo_is_explicit_and_contains_profitable_and_blocked_routes():
    rows = [assess(r, now=1000) for r in demo_routes(now=1000)[0]]
    assert all(row['demo'] for row in rows)
    assert any(row['paper_eligible'] for row in rows)
    assert any('UNKNOWN_EXTERNAL_COSTS' in row['reasons'] for row in rows)


def test_invalid_cross_chain_configuration_never_calls_provider():
    class NoNetwork:
        def tokens(self):
            raise AssertionError('invalid config must be rejected before network')
    with pytest.raises(ValueError, match='routes'):
        scan_cross_chain({'routes': [None] * 5}, client=NoNetwork(), now=1000)

from arbitrage.server import App, create_server
import threading
import json
import urllib.request
import urllib.error


def test_http_dashboard_paper_and_origin_boundary(tmp_path):
    app = App(mode='demo', database=tmp_path / 'orders.sqlite', clock=lambda: 1000)
    server = create_server(app, port=0)
    worker = threading.Thread(target=server.serve_forever, daemon=True)
    worker.start()
    base = f'http://127.0.0.1:{server.server_port}'
    try:
        state = json.load(urllib.request.urlopen(base + '/api/state'))
        assert state['mode'] == 'demo'
        assert len(state['routes']) == 3
        body = json.dumps({'route_id': state['routes'][0]['id'], 'key': 'http-order'}).encode()
        req = urllib.request.Request(base + '/api/paper', data=body,
                                     headers={'Content-Type': 'application/json', 'X-X3-CSRF': state['csrf']})
        result = json.load(urllib.request.urlopen(req))
        assert result['simulation_only'] is True
        req.add_header('Origin', 'https://evil.example')
        with pytest.raises(urllib.error.HTTPError) as error:
            urllib.request.urlopen(req)
        assert error.value.code == 403
        html = urllib.request.urlopen(base + '/').read().decode()
        assert 'X3 Arbitrage' in html
        malicious = urllib.request.Request(base + '/api/state', headers={'Host': 'evil.example'})
        with pytest.raises(urllib.error.HTTPError) as error:
            urllib.request.urlopen(malicious)
        assert error.value.code == 403
    finally:
        server.shutdown()
        server.server_close()
        worker.join()


def test_http_paper_retry_survives_route_refresh_and_restart(tmp_path):
    path = tmp_path / 'journal.sqlite'
    app = App(mode='demo', database=path, clock=lambda: 1000)
    request = {'route_id': app.routes[0].id, 'key': 'persistent-http'}
    first = app.action('/api/paper', request)
    later = App(mode='demo', database=path, clock=lambda: 2000)
    assert later.routes[0].id != request['route_id']
    assert later.action('/api/paper', request) == first


def test_live_preparation_rejects_demo_and_changed_token_identity(tmp_path):
    tokens = [
        {'assetId': 'usdc', 'blockchain': 'sol', 'symbol': 'USDC', 'decimals': 6},
        {'assetId': 'usdc.near', 'blockchain': 'near', 'symbol': 'USDC', 'decimals': 6},
    ]
    r = route()
    program = tmp_path / 'leg.x3'
    def export(r):
        program.write_text(export_leg(r, 0, tokens=tokens, refund_to=SOL_ADDR,
                                      recipient='alice.near', now=110, mandate_owner='owner.near'))
    export(r)
    arguments = dict(mandate='unused.x3', guard_contract='guard.near',
                     receipt=tmp_path / 'receipt', explicit_live=True, now=110)
    with pytest.raises(ValueError, match='demo'):
        prepare_live_quote(program, **arguments)
    live = replace(r, legs=tuple(replace(leg, evidence='provider_dry_quote') for leg in r.legs))
    export(live)
    class ChangedCatalog:
        def tokens(self):
            return [dict(tokens[0], assetId='replacement-contract'), tokens[1]]
        def quote(self, request):
            raise AssertionError('must refuse before requesting deposit quote')
    with pytest.raises(ValueError, match='asset identity'):
        prepare_live_quote(program, client=ChangedCatalog(), **arguments)
    with pytest.raises(ValueError, match='expired'):
        prepare_live_quote(program, **dict(arguments, now=160))


@pytest.mark.parametrize('scenario,status', [('timeout', 'simulated_timeout'),
                                             ('refund', 'simulated_refund'),
                                             ('rpc_failure', 'simulated_rpc_failure')])
def test_paper_failure_cases_are_simulated_and_never_realized(tmp_path, scenario, status):
    result = PaperLedger(tmp_path / 'ledger.sqlite').project(route(), key=scenario, now=110, scenario=scenario)
    assert result['status'] == status
    assert result['realized_profit_units'] is None
    assert result['transactions_submitted'] == 0


def test_concurrent_paper_requests_create_one_order(tmp_path):
    from concurrent.futures import ThreadPoolExecutor
    ledger = PaperLedger(tmp_path / 'ledger.sqlite')
    with ThreadPoolExecutor(max_workers=8) as workers:
        results = list(workers.map(lambda _: ledger.project(route(), key='shared-key', now=110), range(16)))
    assert all(result == results[0] for result in results)
    assert len(ledger.orders()) == 1


@pytest.mark.parametrize('mutator,reason', [
    (lambda leg: replace(leg, amount_in=102_000_000), 'INSUFFICIENT_INTERMEDIATE_OUTPUT'),
    (lambda leg: replace(leg, amount_in=100_000_000), 'UNVALUED_INTERMEDIATE_REMAINDER'),
    (lambda leg: replace(leg, asset_out=NEAR), 'NOT_CLOSED_CYCLE'),
])
def test_invalid_cycle_funding_is_never_eligible(mutator, reason):
    r = route()
    result = assess(replace(r, legs=(r.legs[0], mutator(r.legs[1]))), now=110)
    assert reason in result['reasons']
    assert not result['paper_eligible']


def test_source_edit_is_detected_before_provider_access(tmp_path):
    tokens = [{'assetId': 'usdc', 'blockchain': 'sol', 'symbol': 'USDC', 'decimals': 6},
              {'assetId': 'usdc.near', 'blockchain': 'near', 'symbol': 'USDC', 'decimals': 6}]
    r = replace(route(), legs=tuple(replace(leg, evidence='provider_dry_quote') for leg in route().legs))
    source = export_leg(r, 0, tokens=tokens, refund_to=SOL_ADDR, recipient='alice.near',
                        now=110, mandate_owner='owner.near')
    path = tmp_path / 'edited.x3'
    path.write_text(source.replace('alice.near', 'attacker.near'))
    with pytest.raises(ValueError, match='program changed'):
        prepare_live_quote(path, mandate='mandate.x3', guard_contract='guard.near',
                           receipt=tmp_path / 'receipt', explicit_live=True, now=110)


def test_live_preparation_uses_validated_snapshot_when_original_file_changes(tmp_path, monkeypatch):
    import near_intents as ni
    tokens = [{'assetId': 'usdc', 'blockchain': 'sol', 'symbol': 'USDC', 'decimals': 6},
              {'assetId': 'usdc.near', 'blockchain': 'near', 'symbol': 'USDC', 'decimals': 6}]
    r = replace(route(), legs=tuple(replace(leg, evidence='provider_dry_quote') for leg in route().legs))
    source = export_leg(r, 0, tokens=tokens, refund_to=SOL_ADDR, recipient='alice.near',
                        now=110, mandate_owner='owner.near')
    path = tmp_path / 'racing.x3'
    path.write_text(source)
    class MutatingCatalog:
        def tokens(self):
            path.write_text(source.replace('alice.near', 'attacker.near'))
            return tokens
    # This is the external live-quote boundary: never request a deposit address in tests.
    # Compilation and planning below it remain real.
    def quote_boundary(program, **kwargs):
        return ni.compile_program(program)[1]
    monkeypatch.setattr(ni, 'quote', quote_boundary)
    result = prepare_live_quote(path, mandate='mandate.x3', guard_contract='guard.near',
                                receipt=tmp_path / 'receipt', explicit_live=True,
                                now=110, client=MutatingCatalog())
    assert result['recipient'] == 'alice.near'


def test_http_automatic_cycle_is_paper_only_and_survives_refresh(tmp_path):
    database = tmp_path / 'trades.sqlite'
    app = App(mode='demo', database=database, clock=lambda: 1000)
    body = {'key': 'automatic-api', 'route_id': app.routes[0].id, 'scenario': 'success'}
    result = app.action('/api/trade', body)
    assert result['phase'] == 'COMPLETED'
    assert result['mode'] == 'paper'
    assert result['real_transactions_submitted'] == 0
    assert result['paper_profit_units'] == '2000000'
    restarted = App(mode='demo', database=database, clock=lambda: 2000)
    assert restarted.action('/api/trade', body) == result
    with pytest.raises(ValueError, match='only'):
        restarted.action('/api/trade', dict(body, mode='live'))


def test_paper_dashboard_does_not_mislabel_live_cycle_history(tmp_path):
    from arbitrage.trader import example_strategy
    from dataclasses import replace
    app = App(mode='demo', database=tmp_path / 'both.sqlite', clock=lambda: 1000)
    configured = example_strategy(app.routes[0], app.tokens, now=1000)
    live_route = replace(configured.route, legs=tuple(replace(leg, evidence='provider_dry_quote') for leg in configured.route.legs))
    app.trader.create('live-history', replace(configured, route=live_route), mode='live')
    assert app.state()['cycles'] == []
