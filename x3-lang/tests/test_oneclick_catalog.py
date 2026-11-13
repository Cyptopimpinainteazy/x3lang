from datetime import datetime, timedelta, timezone
import pytest
import oneclick_catalog as oc


def token(chain, decimals=6, price='1'):
    return dict(assetId=chain + ':asset', blockchain=chain, decimals=decimals, price=price)


def request(origin='base', dest='near', amount='1000000', **over):
    return dict(dry=True, swapType='EXACT_INPUT', originAsset=origin + ':asset',
                destinationAsset=dest + ':asset', amount=amount, depositType='ORIGIN_CHAIN',
                recipientType='DESTINATION_CHAIN', refundType='ORIGIN_CHAIN',
                recipient='dotx3.near', refundTo='public-address', slippageTolerance=100,
                deadline=(datetime.now(timezone.utc)+timedelta(minutes=10)).isoformat(), **over)


@pytest.mark.parametrize('origin,dest,amount,minimum,status', [
    ('bsc','near','999999999','1000','below_documented_minimum'),
    ('near','bsc','1000000000','1000','meets_documented_minimum'),
    ('tron','near','99999999','100','below_documented_minimum'),
    ('tron','near','100000000','100','meets_documented_minimum'),
    ('near','tron','1000000','0','no_documented_chain_minimum'),
    ('base','near','1000000','0','no_documented_chain_minimum'),
    ('sol','near','1000000','0','no_documented_chain_minimum'),
    ('tron','bsc','100000000','1000','below_documented_minimum'),
])
def test_limits(origin,dest,amount,minimum,status):
    result = oc.limit_advisory(request(origin,dest,amount), oc.catalog([token(origin),token(dest)]))
    assert result['minimum_usd'] == minimum
    assert result['status'] == status


def test_exact_output_uses_destination_decimals_and_price():
    r = request('bsc','near','500000000000000000000000000')
    r['swapType'] = 'EXACT_OUTPUT'
    result = oc.limit_advisory(r, oc.catalog([token('bsc'),token('near',24,'2')]))
    assert result['estimated_usd'] == '1000'
    assert result['status'] == 'meets_documented_minimum'


@pytest.mark.parametrize('field', ['depositType','recipientType'])
def test_intents_exception(field):
    r = request('bsc'); r[field]='INTENTS'
    assert oc.limit_advisory(r,oc.catalog([token('bsc'),token('near')]))['minimum_usd']=='0'


@pytest.mark.parametrize('price',[None,'NaN','Infinity','-1','0','1e999999999'])
def test_unknown_price_does_not_pass(price):
    assert oc.limit_advisory(request('bsc'),oc.catalog([token('bsc',price=price),token('near')]))['status']=='price_unknown'


def test_dynamic_catalog_and_duplicate_rejection():
    assert 'future-chain:asset' in oc.catalog([token('future-chain')])
    with pytest.raises(ValueError,match='duplicate'):
        oc.catalog([token('base'),token('base')])


class Client:
    def __init__(self): self.calls=[]
    def tokens(self): return [token('base'), token('near')]
    def quote(self,r): self.calls.append(r); return {'quote':{'depositAddress':'not-authorized'}}


def test_dry_transport_never_authorizes():
    client=Client(); result=oc.dry_quote(client,request())
    assert client.calls[0]['dry'] is True
    assert result['eligible_for_wallet_authorization'] is False
    assert result['status']=='provider_quote_unverified'


@pytest.mark.parametrize('field,value',[('dry',False),('appFees',[]),('amount','1.2'),('originAsset','missing'),('slippageTolerance',True)])
def test_invalid_requests_never_quote(field,value):
    r=request(); r[field]=value; client=Client()
    with pytest.raises(ValueError): oc.dry_quote(client,r)
    assert client.calls==[]


def test_provider_bridge_error_preserved():
    class Refused(Client):
        def quote(self,r):
            raise oc.ni.NearIntentsError('X3_NEAR_HTTP','Amount is too low for bridge, try at least 42',detail={'status':400})
    result=oc.dry_quote(Refused(),request())
    assert result['status']=='provider_refused'
    assert 'try at least 42' in result['error']['message']


@pytest.mark.parametrize('field',['originAsset','destinationAsset','swapType','depositType','recipientType','refundType'])
def test_json_arrays_are_structured_validation_errors(field):
    r=request(); r[field]=[]; client=Client()
    with pytest.raises(ValueError): oc.dry_quote(client,r)
    assert client.calls==[]
