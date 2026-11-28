"""Discovery never grants swap or LP authority; malformed data fails closed."""
from copy import deepcopy
from types import SimpleNamespace

import pytest
from arbitrage.pools import TARGETS, discover_pools


def pair(network, anchor, *, pool=None):
    other = 'So11111111111111111111111111111111111111112' if network == 'solana' else '0x' + '1' * 40
    return {'chainId': network, 'pairAddress': pool or other, 'dexId': 'test-dex',
            'baseToken': {'address': anchor, 'symbol': 'USDC'},
            'quoteToken': {'address': other, 'symbol': 'OTHER'},
            'liquidity': {'usd': 12345.67}, 'volume': {'h24': 456},
            'url': 'https://attacker.invalid/ignored'}


class Near:
    def final_block(self, *, now):
        return SimpleNamespace(timestamp_s=now-1, height=123, hash='pinned-block')
    def view(self, block, contract, method, args):
        assert contract == 'v2.ref-finance.near' and method == 'get_pools'
        assert args == {'from_index': 0, 'limit': 100}
        return SimpleNamespace(value=[{'pool_kind': 'SIMPLE_POOL',
            'token_account_ids': ['wrap.near', 'usdc.near'], 'amounts': ['10', '20'],
            'total_fee': 30, 'shares_total_supply': '100', 'amp': 0}])


def fetch(url):
    network, anchor = url.split('/')[-2:]
    return [pair(network, anchor)]


def report(fetcher=fetch, near=None):
    return discover_pools(fetch=fetcher, now=1000, near_view=near or Near())


def test_all_networks_preserve_exact_identity_and_never_enable_funding():
    result = report()
    assert result['pool_count'] == 5 and result['errors'] == []
    assert {p['network'] for p in result['pools']} == {'near', 'solana', 'ethereum', 'base', 'arbitrum'}
    assert not result['fees_enabled'] and result['transactions_submitted'] == 0
    assert all(p['executable'] is False for p in result['pools'])
    assert all(p['source_url'].startswith(('https://dexscreener.com/', 'https://app.rhea.finance/')) for p in result['pools'])
    near = next(p for p in result['pools'] if p['network'] == 'near')
    assert near['reserves_atomic'] == ['10', '20'] and near['block_height'] == 123
    assert near['liquidity_usd'] is None and near['volume_24h_usd'] is None
    indexed = next(p for p in result['pools'] if p['network'] == 'ethereum')
    assert indexed['liquidity_usd'] == '12345.67' and indexed['chain_timestamp'] is None


@pytest.mark.parametrize('mutation', [
    lambda row: row.update(chainId='wrong'),
    lambda row: row.update(pairAddress='../../evil'),
    lambda row: row['liquidity'].update(usd=0),
    lambda row: row['liquidity'].update(usd=float('nan')),
    lambda row: row['liquidity'].update(usd=True),
    lambda row: row['liquidity'].update(usd='1e99999'),
    lambda row: row.update(baseToken={'address':'0x'+'2'*40,'symbol':'USDC'}),
    lambda row: row.update(dexId='"><script>'),
])
def test_unrelated_or_malformed_indexed_rows_are_refused(mutation):
    def invalid(url):
        network, anchor = url.split('/')[-2:]
        row = pair(network, anchor)
        mutation(row)
        return [row]
    result = report(invalid)
    assert result['pool_count'] == 1
    assert len(result['errors']) == 4


def test_partial_outage_retains_other_networks_without_fake_rows():
    def partial(url):
        if '/solana/' in url:
            raise OSError('a private message must not be exposed')
        return fetch(url)
    result = report(partial)
    assert result['pool_count'] == 4
    assert result['errors'][0]['network'] == 'solana'
    assert 'private' not in str(result)


def test_oversized_response_and_near_disagreement_refuse_sources():
    class Disagree(Near):
        def final_block(self, *, now):
            raise ValueError('disagreement')
    result = report(lambda url: [{}] * 501, near=Disagree())
    assert result['pools'] == [] and len(result['errors']) == 5


def test_duplicates_do_not_inflate_pool_count_and_symbols_do_not_override_identity():
    def repeated(url):
        rows = fetch(url)
        rows[0]['baseToken']['symbol'] = 'misleading\nlabel'
        return rows * 2
    result = report(repeated)
    assert result['pool_count'] == 5
    assert all('\n' not in p['base_token']['symbol'] for p in result['pools'])


def test_funded_near_pool_without_shares_is_not_a_real_lp_position():
    class Empty(Near):
        def view(self, *args):
            result = super().view(*args)
            result.value[0]['shares_total_supply'] = '0'
            return result
    result = report(near=Empty())
    assert result['pool_count'] == 4


def test_reject_bad_timestamp():
    with pytest.raises(ValueError):
        discover_pools(now=True)


def test_conflicting_duplicate_is_removed_for_whole_snapshot():
    def conflicting(url):
        row = fetch(url)[0]
        second = deepcopy(row)
        second['liquidity']['usd'] = 999
        return [row, second, row]
    result = report(conflicting)
    assert result['pool_count'] == 1
    assert len(result['errors']) == 4


def test_tiny_exponents_refuse_without_unbounded_expansion():
    def tiny(url):
        row = fetch(url)[0]
        row['liquidity']['usd'] = '1e-999999999'
        return [row]
    assert report(tiny)['pool_count'] == 1
