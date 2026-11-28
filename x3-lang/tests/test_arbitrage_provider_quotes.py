"""Unsigned size-specific quotes never grant execution or paper authority."""
from copy import deepcopy
from urllib.parse import parse_qs, urlsplit

import pytest
from arbitrage.provider_quotes import market_catalog, quote_markets

ADDRESS = '0x' + '1' * 40


def params():
    return {'market_id':'base-usdc-weth', 'amount':'100', 'address':ADDRESS}


def response(url):
    query = parse_qs(urlsplit(url).query)
    market = next(m for m in market_catalog() if m['id'] == 'base-usdc-weth')
    tokens = {t['address'].lower(): t for t in (market['origin_token'], market['intermediate_token'])}
    first = query['fromToken'][0].lower() == market['origin_token']['address'].lower()
    output, minimum = ('50000000000000000', '49750000000000000') if first else ('99000000', '98505000')
    return {'id':'provider-quote', 'tool':'test-dex',
            'action':{'fromChainId':8453, 'toChainId':8453,
                      'fromToken':deepcopy(tokens[query['fromToken'][0].lower()]),
                      'toToken':deepcopy(tokens[query['toToken'][0].lower()]),
                      'fromAmount':query['fromAmount'][0], 'fromAddress':ADDRESS,
                      'toAddress':ADDRESS, 'slippage':0.005},
            'estimate':{'fromAmount':query['fromAmount'][0], 'toAmount':output,
                        'toAmountMin':minimum, 'feeCosts':[], 'gasCosts':[]},
            'transactionRequest':{'data':'SECRET_CALLDATA_DO_NOT_RETURN'}}


def test_size_specific_roundtrip_uses_first_minimum_and_never_returns_calldata():
    requests = []
    def fetch(url):
        requests.append(parse_qs(urlsplit(url).query))
        return response(url)
    report = quote_markets(params(), fetch=fetch, now=1000)
    assert len(requests) == 2
    assert requests[0]['fromAmount'] == ['100000000']
    assert requests[1]['fromAmount'] == ['49750000000000000']
    assert all(q['slippage']==['0.005'] for q in requests)
    assert all(q['fee']==['0'] and 'integrator' not in q for q in requests)
    quote = report['quotes'][0]
    assert quote['roundtrip_floor']=='98505000'
    assert quote['gross_delta']=='-1495000'
    assert quote['net_profit'] is None
    assert not quote['executable'] and not quote['paper_eligible']
    assert 'SECRET_CALLDATA' not in str(report)
    assert not report['fees_enabled'] and report['transactions_submitted']==0


@pytest.mark.parametrize('change',[
    {'market_id':'https://attacker.invalid'}, {'amount':'1e20'}, {'amount':'0'},
    {'amount':'0.0000001'}, {'address':'bad'}, {'address':'0x'+'0'*40},
    {'fee':'0.01'}, {'amount':True},
])
def test_invalid_input_cannot_control_urls_fee_or_wallet_operations(change):
    data = dict(params(), **change)
    with pytest.raises(ValueError):
        quote_markets(data, fetch=lambda _:pytest.fail('invalid input reached provider'), now=1000)


@pytest.mark.parametrize('mutate',[
    lambda r:r['action'].update(fromChainId=1),
    lambda r:r['action'].update(fromAmount='1'),
    lambda r:r['action'].update(toAddress='0x'+'2'*40),
    lambda r:r['action'].update(slippage=0.01),
    lambda r:r['action']['toToken'].update(decimals=6),
    lambda r:r['action']['toToken'].update(address='0x'+'3'*40),
    lambda r:r['estimate'].update(fromAmount='1'),
    lambda r:r['estimate'].update(toAmountMin='999999999999999999'),
    lambda r:r['estimate'].update(toAmount='1e100'),
    lambda r:r['estimate'].update(toAmountMin=True),
])
def test_changed_provider_binding_refuses_entire_roundtrip(mutate):
    def fetch(url):
        r=response(url);mutate(r);return r
    report=quote_markets(params(), fetch=fetch, now=1000)
    assert report['quotes']==[]
    assert report['errors'][0]['code']=='QUOTE_UNAVAILABLE'


def test_return_failure_does_not_present_partial_cycle():
    calls=[]
    def fetch(url):
        calls.append(url)
        if len(calls)==2: raise OSError('private provider details')
        return response(url)
    report=quote_markets(params(),fetch=fetch,now=1000)
    assert report['quotes']==[] and report['errors']
    assert 'private provider' not in str(report)


def test_cross_chain_roundtrip_binds_each_chain_and_consumes_first_minimum():
    requests = []
    market = next(m for m in market_catalog() if m['id'] == 'ethereum-base-usdc')
    def fetch(url):
        query = parse_qs(urlsplit(url).query)
        requests.append(query)
        reverse = len(requests)==2
        src = market['intermediate_token'] if reverse else market['origin_token']
        dst = market['origin_token'] if reverse else market['intermediate_token']
        return {'tool':'across', 'action':{'fromChainId':src['chainId'], 'toChainId':dst['chainId'],
            'fromToken':src, 'toToken':dst, 'fromAmount':query['fromAmount'][0],
            'fromAddress':ADDRESS, 'toAddress':ADDRESS, 'slippage':0.005},
            'estimate':{'fromAmount':query['fromAmount'][0], 'toAmount':'99000000',
            'toAmountMin':'98505000','feeCosts':[],'gasCosts':[]}}
    result = quote_markets(dict(params(), market_id=market['id']), fetch=fetch, now=1000)
    assert len(result['quotes']) == 1
    assert requests[0]['fromChain']==['1'] and requests[0]['toChain']==['8453']
    assert requests[1]['fromChain']==['8453'] and requests[1]['toChain']==['1']
    assert requests[1]['fromAmount']==['98505000']


def test_integer_floor_rounding_allows_one_atomic_unit_without_extra_slippage():
    def fetch(url):
        r=response(url)
        r['estimate']['toAmount']='62023160653570740'
        r['estimate']['toAmountMin']='61713044850302886'
        return r
    assert len(quote_markets(params(),fetch=fetch,now=1000)['quotes'])==1


def test_extra_slippage_beyond_integer_floor_is_refused():
    def fetch(url):
        r=response(url)
        r['estimate']['toAmountMin']='1'
        return r
    assert quote_markets(params(),fetch=fetch,now=1000)['quotes']==[]
