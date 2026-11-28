"""Exact identities/decimals and fail-closed coverage, not token safety endorsements."""
import pytest
from arbitrage.catalogue import token_report, build_route

TOKENS = [
    {'assetId':'bsc-usdc', 'blockchain':'bsc', 'symbol':'USDC', 'decimals':18},
    {'assetId':'near-usdc', 'blockchain':'near', 'symbol':'USDC', 'decimals':6},
    {'assetId':'sol-wif', 'blockchain':'sol', 'symbol':'$WIF', 'decimals':6},
    {'assetId':'old', 'blockchain':'bsc', 'symbol':'USDC (DEPRECATED)', 'decimals':18},
]


def parameters():
    return {'origin_asset':'bsc-usdc', 'destination_asset':'near-usdc','amount':'1.25',
            'origin_address':'0x1111111111111111111111111111111111111111',
            'destination_address':'alice.near', 'external_cost':'0.001',
            'cost_source':'operator gas estimate in origin USDC'}


def test_provider_listings_keep_exact_ids_and_never_grant_execution():
    result = token_report(TOKENS, now=1000)
    assert result['asset_count'] == 4
    assert result['chains'] == {'bsc':2,'near':1,'sol':1}
    assert all(asset['executable'] is False for asset in result['assets'])
    by_id = {asset['asset_id']:asset for asset in result['assets']}
    assert not by_id['sol-wif']['x3_export_candidate']
    assert by_id['old']['deprecated']
    assert by_id['bsc-usdc']['x3_export_candidate']


def test_route_builder_uses_18_decimal_bsc_usdc_without_assuming_6():
    result = build_route(parameters(), TOKENS)
    assert result['amount'] == '1250000000000000000'
    assert result['external_cost_units'] == 1000000000000000
    assert result['origin_asset'] == 'bsc-usdc'
    assert result['destination_asset'] == 'near-usdc'


def test_unknown_costs_stay_unknown_and_deprecated_assets_are_blocked():
    assert build_route(dict(parameters(), external_cost=''), TOKENS)['external_cost_units'] is None
    with pytest.raises(ValueError, match='deprecated'):
        build_route(dict(parameters(), origin_asset='old'), TOKENS)
    with pytest.raises(ValueError, match='exact assets'):
        build_route(dict(parameters(), origin_asset='USDC'), TOKENS)


def test_catalog_browser_keeps_real_catalog_separate_from_demo_compilation(tmp_path):
    from arbitrage.server import App
    class CatalogOnly:
        def tokens(self): return TOKENS
    app = App(mode='demo', database=tmp_path / 'catalog.sqlite', clock=lambda: 1000,
              catalog_client=CatalogOnly())
    original = list(app.tokens)
    result = app.action('/api/catalogue', {})
    assert result['asset_count'] == 4
    assert app.state()['catalogue']['asset_count'] == 4
    assert app.tokens == original
    assert len(app.routes) == 3


def test_ambiguous_symbols_remain_exact_discovery_entries():
    ambiguous = TOKENS + [{'assetId':'near-other-usdc','blockchain':'near','symbol':'USDC','decimals':6}]
    report = token_report(ambiguous, now=1000)
    assert not any(asset['x3_export_candidate'] for asset in report['assets'] if asset['network']=='near')


def test_route_api_only_requests_dry_quotes_and_preserves_demo_on_refusal(tmp_path):
    import time
    import near_intents as ni
    from arbitrage.server import App
    class RefusingProvider:
        base_url = ni.DEFAULT_BASE_URL
        def __init__(self): self.requests = []
        def tokens(self): return TOKENS
        def quote(self, request):
            self.requests.append(request)
            raise ni.NearIntentsError('X3_NEAR_PROVIDER_MINIMUM', 'constructed refusal')
    client = RefusingProvider()
    app = App(mode='demo', database=tmp_path / 'routes.sqlite', clock=lambda: 1000, catalog_client=client)
    app.action('/api/catalogue', {})
    result = app.action('/api/routes', parameters())
    assert result['quote_mode'] == 'dry'
    stop = time.monotonic() + 3
    while app.state()['refreshing'] and time.monotonic() < stop: time.sleep(0.01)
    assert client.requests and all(request['dry'] is True for request in client.requests)
    assert client.requests[0]['amount'] == '1250000000000000000'
    assert len(app.state()['routes']) == 3
    assert all(row['demo'] for row in app.state()['routes'])
    assert app.state()['errors'][0]['code'] == 'X3_NEAR_PROVIDER_MINIMUM'
    assert result['transactions_submitted'] == 0
    with pytest.raises(ValueError):
        app.action('/api/routes', dict(parameters(), dry=False))


def test_missing_provider_symbol_does_not_break_state(tmp_path):
    from arbitrage.server import App
    class MissingSymbol:
        def tokens(self): return [{'assetId':'unnamed','blockchain':'near','decimals':6}]
    app = App(mode='demo', database=tmp_path / 'unnamed.sqlite', clock=lambda: 1000, catalog_client=MissingSymbol())
    app.action('/api/catalogue', {})
    assert app.state()['asset_symbols']['near/unnamed'] == 'unnamed'
    assert app.state()['catalogue']['assets'][0]['x3_export_candidate'] is False


def test_scan_metadata_snapshot_is_separate_from_selection_catalogue(tmp_path):
    from arbitrage.server import App
    class NewCatalog:
        def tokens(self): return TOKENS
    app = App(mode='demo', database=tmp_path / 'snapshot.sqlite', clock=lambda: 1000, catalog_client=NewCatalog())
    app.action('/api/catalogue', {})
    app.provider_tokens = [{'assetId':'old-scan-id','blockchain':'near','symbol':'OLD','decimals':6}]
    # A completed older scan cannot replace the selection snapshot behind visible options.
    assert build_route(parameters(), app.catalogue_tokens)['origin_asset'] == 'bsc-usdc'
    assert app.state()['catalogue']['assets'][0]['asset_id'] != 'old-scan-id'


def test_changed_decimals_are_refused_before_a_provider_quote():
    from arbitrage.adapters import scan_cross_chain
    class ChangedDecimals:
        def tokens(self): return [dict(token, decimals=6) if token['assetId']=='bsc-usdc' else token for token in TOKENS]
        def quote(self, request): raise AssertionError('changed units must not reach provider')
    rows, _, errors = scan_cross_chain({'routes':[build_route(parameters(), TOKENS)]},
        client=ChangedDecimals(), now=1000, expected_assets={'bsc-usdc': {'network':'bsc','decimals':18}})
    assert rows == []
    assert errors[0]['code'] == 'TOKEN_IDENTITY_CHANGED'
