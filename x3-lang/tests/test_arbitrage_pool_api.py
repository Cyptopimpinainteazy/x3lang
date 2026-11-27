"""Discovery never creates executable opportunities or wallet authority."""
import pytest
from arbitrage.server import App


def test_pool_refresh_is_cached_and_cannot_accept_urls_or_fees(tmp_path):
    calls = []
    report = {'loaded_at': 1000, 'pools': [], 'errors': [], 'claim': 'discovery only'}
    def discover(**kwargs):
        calls.append(kwargs)
        return report
    app = App(mode='demo', database=tmp_path / 'paper.sqlite', clock=lambda:1000,
              pool_discovery=discover)
    original = app.state()['routes']
    assert app.state()['pools'] is None
    assert app.action('/api/pools', {}) == report
    assert app.action('/api/pools', {}) == report
    assert len(calls) == 1
    assert app.state()['routes'] == original
    assert app.state()['cycles'] == []
    for data in ({'url':'https://example.com'}, {'fee':0.01}, {'deposit':True}):
        with pytest.raises(ValueError):
            app.action('/api/pools', data)


def test_pool_refresh_failure_is_rate_limited_and_unlocks(tmp_path):
    calls = []
    now = [1000]
    def discover(**kwargs):
        calls.append(kwargs)
        raise ValueError('provider refused')
    app = App(mode='demo', database=tmp_path / 'paper.sqlite', clock=lambda:now[0],
              pool_discovery=discover)
    with pytest.raises(ValueError, match='provider refused'):
        app.action('/api/pools', {})
    with pytest.raises(ValueError, match='60-second'):
        app.action('/api/pools', {})
    assert len(calls) == 1
    now[0] += 60
    with pytest.raises(ValueError, match='provider refused'):
        app.action('/api/pools', {})
    assert len(calls) == 2
