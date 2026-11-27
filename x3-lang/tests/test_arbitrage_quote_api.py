"""Quote previews cannot grant route, paper or signing authority."""
import pytest
from arbitrage.server import App


def test_preview_is_rate_limited_and_separate_from_execution(tmp_path):
    calls=[]
    now=[1000]
    def quote(parameters, **kwargs):
        calls.append((parameters,kwargs))
        return {'loaded_at':kwargs['now'], 'quotes':[], 'errors':[]}
    app=App(mode='demo',database=tmp_path/'paper.sqlite',clock=lambda:now[0],quote_discovery=quote)
    params={'market_id':'base-usdc-weth','amount':'100','address':'0x'+'1'*40}
    before=app.state()['routes']
    result=app.action('/api/provider-quotes',params)
    assert result['loaded_at']==1000
    assert app.state()['provider_quotes']==result
    assert app.state()['routes']==before
    assert app.state()['cycles']==[]
    with pytest.raises(ValueError,match='15-second'):
        app.action('/api/provider-quotes',params)
    now[0]+=15
    app.action('/api/provider-quotes',params)
    assert len(calls)==2


def test_failed_quote_attempt_still_consumes_request_budget(tmp_path):
    def quote(parameters, **kwargs):
        raise ValueError('quote refused')
    app=App(mode='demo',database=tmp_path/'paper.sqlite',clock=lambda:1000,quote_discovery=quote)
    with pytest.raises(ValueError,match='quote refused'):
        app.action('/api/provider-quotes',{})
    with pytest.raises(ValueError,match='15-second'):
        app.action('/api/provider-quotes',{})
    assert app.state()['provider_quotes'] is None
