"""Editable .x3 mandate workflow uses the actual compiler and paper coordinator."""
import pytest
from arbitrage.server import App


def app(tmp_path):
    return App(mode='demo', database=tmp_path / 'workbench.sqlite', clock=lambda: 1000)


def test_route_opens_editable_x3_draft_and_compiles_both_legs_without_running(tmp_path):
    application = app(tmp_path)
    draft = application.action('/api/strategy/template', {'route_id': application.routes[0].id})['draft']
    assert len(draft['mandates']) == 2
    assert 'mandate automatic_paper_leg_0' in draft['mandates'][0]
    compiled = application.action('/api/strategy/compile', {'draft': draft})
    assert compiled['valid'] is True
    assert len(compiled['programs']) == 2
    assert compiled['eligible_for_wallet_authorization'] is False
    assert application.trader.cycles() == []


def test_editing_mandate_caps_really_blocks_the_route(tmp_path):
    application = app(tmp_path)
    draft = application.action('/api/strategy/template', {'route_id': application.routes[0].id})['draft']
    draft['mandates'][0] = draft['mandates'][0].replace('per_payment 200000000', 'per_payment 1')
    with pytest.raises(ValueError, match='mandate'):
        application.action('/api/strategy/compile', {'draft': draft})
    assert application.trader.cycles() == []


def test_validated_x3_draft_runs_only_in_paper_and_replays_exact_inputs(tmp_path):
    application = app(tmp_path)
    draft = application.action('/api/strategy/template', {'route_id': application.routes[0].id})['draft']
    request = {'draft': draft, 'key': 'edited-x3', 'scenario': 'success'}
    result = application.action('/api/strategy/run', request)
    assert result['phase'] == 'COMPLETED'
    assert result['mode'] == 'paper'
    assert result['real_transactions_submitted'] == 0
    assert application.action('/api/strategy/run', request) == result
    draft['minimum_profit'] = '0.5'
    with pytest.raises(ValueError, match='idempotency'):
        application.action('/api/strategy/run', request)


def test_running_edited_source_cannot_bypass_compilation_or_select_live_mode(tmp_path):
    application = app(tmp_path)
    draft = application.action('/api/strategy/template', {'route_id': application.routes[0].id})['draft']
    draft['mandates'][0] = draft['mandates'][0].replace('per_payment 200000000', 'per_payment 1')
    with pytest.raises(ValueError, match='mandate_per_payment'):
        application.action('/api/strategy/run', {'draft':draft,'key':'bypass','scenario':'success'})
    with pytest.raises(ValueError):
        application.action('/api/strategy/run', {'draft':draft,'key':'bypass','scenario':'success','mode':'live'})
    assert application.trader.cycles() == []


def test_edited_x3_execution_replay_survives_expired_route_and_server_restart(tmp_path):
    application = app(tmp_path)
    draft = application.action('/api/strategy/template', {'route_id': application.routes[0].id})['draft']
    request = {'draft':draft,'key':'persistent-x3','scenario':'success'}
    result = application.action('/api/strategy/run', request)
    restarted = App(mode='demo', database=tmp_path / 'workbench.sqlite', clock=lambda:2000)
    assert restarted.action('/api/strategy/run', request) == result
