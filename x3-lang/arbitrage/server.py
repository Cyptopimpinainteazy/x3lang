"""Loopback-only local application; state changes require same-origin CSRF capability."""
import json
import secrets
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import urlsplit

from .adapters import demo_routes, scan_cex, scan_cross_chain, validate_config
from .engine import rank
from .execution import export_leg
from .paper import PaperLedger
from .trader import Trader, PaperExchange, example_strategy
from .catalogue import token_report, build_route
from .provider_quotes import market_catalog
from .workbench import template, compile_draft, strategy_from_draft
from .trader import strategy_from_document
from .mandate_proof import MandateProof
from .cycle_closer import CycleCloser
from .liquidity_listing import LiquidityListing
from .fee_splitter import FeeSplitter
import near_intents as ni

WEB = Path(__file__).parent / 'web'


class App:
    def __init__(self, *, mode, database, config=None, clock=time.time, catalog_client=None, pool_discovery=None, quote_discovery=None):
        if mode not in {'demo', 'live'}:
            raise ValueError('mode must be demo or live')
        self.mode, self.clock = mode, clock
        self.config = validate_config(config) if config is not None else None
        self.ledger = PaperLedger(database)
        self.trader = Trader(database, clock=clock)
        self.proof = MandateProof(self.trader.store)
        self.closer = CycleCloser(self.trader)
        self.listing = LiquidityListing(self.proof)
        self.splitter = FeeSplitter()
        self.csrf = secrets.token_urlsafe(32)
        self.catalog_client = catalog_client or ni.OneClickClient(timeout=8, api_key='')
        if pool_discovery is None:
            from .pools import discover_pools
            pool_discovery = discover_pools
        if quote_discovery is None:
            from .provider_quotes import quote_markets
            quote_discovery = quote_markets
        self.quote_discovery = quote_discovery
        self.provider_quotes = None
        self.quote_lock = threading.Lock()
        self.last_quote_refresh = None
        self.pool_discovery = pool_discovery
        self.pools = None
        self.pool_lock = threading.Lock()
        self.last_pool_refresh = None
        self.catalogue = None
        self.provider_tokens = []
        self.catalogue_tokens = []
        self.route_asset_pins = {}
        self.catalogue_lock = threading.Lock()
        self.lock = threading.RLock()
        self.refresh_lock = threading.Lock()
        self.routes, self.tokens, self.errors = [], [], []
        self.updated_at, self.last_refresh = None, None
        self.refreshing = False
        if mode == 'demo':
            self.refresh()

    def refresh(self):
        if not self.refresh_lock.acquire(blocking=False):
            return
        try:
            now = int(self.clock())
            with self.lock:
                self.refreshing = True
            if self.mode == 'demo':
                routes, tokens = demo_routes(now=now)
                errors = []
            else:
                routes, errors = scan_cex(now=now)
                tokens = []
            provider_tokens = list(self.provider_tokens)
            if self.config:
                try:
                    cross, provider_tokens, cross_errors = scan_cross_chain(self.config, client=self.catalog_client, now=now, expected_assets=dict(self.route_asset_pins))
                    routes.extend(cross)
                    errors.extend(cross_errors)
                except Exception:
                    errors.append({'provider': 'oneclick', 'code': 'CATALOG_UNAVAILABLE',
                                   'message': 'Token catalog unavailable or invalid; no substitute quotes used.'})
            elif self.mode == 'live':
                errors.append({'provider': 'oneclick', 'code': 'ROUTES_NOT_CONFIGURED',
                               'message': 'Load provider tokens and build a route with exact assets and addresses.'})
            with self.lock:
                self.routes = routes
                self.tokens = tokens if self.mode == 'demo' else provider_tokens
                self.provider_tokens, self.errors = provider_tokens, errors
                self.updated_at = int(self.clock())
        finally:
            with self.lock:
                self.refreshing = False
            self.refresh_lock.release()

    def start_refresh(self):
        with self.lock:
            now = self.clock()
            if self.refreshing or (self.last_refresh is not None and now - self.last_refresh < 15):
                raise ValueError('refresh already running or 15-second request budget exhausted')
            self.last_refresh = now
            self.refreshing = True
        threading.Thread(target=self.refresh, daemon=True).start()

    def state(self):
        with self.lock:
            return {'schema': 'x3.arbitrage.v1', 'mode': ('mixed' if self.mode == 'demo' and any(not route.demo for route in self.routes) else self.mode), 'csrf': self.csrf,
                    'provider_quotes': self.provider_quotes, 'quote_markets': market_catalog(),
                    'pools': self.pools, 'catalogue': self.catalogue, 'configured_route_count': len(self.config['routes']) if self.config else 0,
                    'now': int(self.clock()), 'updated_at': self.updated_at,
                    'refreshing': self.refreshing,
                    'routes': rank(self.routes, now=int(self.clock())),
                    'errors': list(self.errors), 'orders': self.ledger.orders(),
                    'cycles': [cycle for cycle in self.trader.cycles() if cycle['mode'] == 'paper'],
                    'asset_symbols': {token['blockchain'] + '/' + token['assetId']: (token.get('symbol') if isinstance(token.get('symbol'), str) and len(token['symbol']) <= 128 else token['assetId'])
                                      for token in self.tokens + self.provider_tokens + self.catalogue_tokens},
                    'live_execution': 'Separate CLI preparation; no signing or deposits from dashboard.'}

    def route(self, route_id):
        with self.lock:
            for route in self.routes:
                if route.id == route_id:
                    return route
        raise ValueError('route not found; refresh and select a current route')

    def action(self, path, data):
        if not isinstance(data, dict):
            raise ValueError('JSON object required')
        if path == '/api/mandate/proof':
            if set(data) != {'mandate_id'}:
                raise ValueError('mandate_id required')
            return self.proof.snapshot(data['mandate_id'], int(self.clock()) * 1000)
        if path == '/api/liquidity/listing':
            ids = data.get('mandate_ids', [])
            if not isinstance(ids, list) or len(ids) > 20:
                raise ValueError('mandate_ids must be a short list')
            return self.listing.listing(ids, int(self.clock()) * 1000)
        if path == '/api/cycle/close':
            if set(data) != {'key'}:
                raise ValueError('cycle key required')
            # Paper backend only from the dashboard
            return self.closer.close_or_unwind(data['key'], PaperExchange(self.trader.store), int(self.clock()))
        if path == '/api/provider-quotes':
            if not self.quote_lock.acquire(blocking=False):
                raise ValueError('provider quote request already running')
            try:
                with self.lock:
                    now = int(self.clock())
                    if self.last_quote_refresh is not None and now - self.last_quote_refresh < 15:
                        raise ValueError('provider quote 15-second request budget exhausted')
                    self.last_quote_refresh = now
                report = self.quote_discovery(data, now=now)
                with self.lock:
                    self.provider_quotes = report
                return report
            finally:
                self.quote_lock.release()
        if path == '/api/pools':
            if data:
                raise ValueError('pool discovery accepts no URLs, fees or wallet parameters')
            if not self.pool_lock.acquire(blocking=False):
                raise ValueError('pool discovery already running')
            try:
                with self.lock:
                    now = int(self.clock())
                    if self.last_pool_refresh is not None and now - self.last_pool_refresh < 60:
                        if self.pools is not None:
                            return self.pools
                        raise ValueError('pool discovery 60-second request budget exhausted')
                    self.last_pool_refresh = now
                report = self.pool_discovery(now=now)
                with self.lock:
                    self.pools = report
                return report
            finally:
                self.pool_lock.release()
        if path == '/api/strategy/template':
            if set(data) != {'route_id'}:
                raise ValueError('selected route required')
            with self.lock:
                route = self.route(data['route_id'])
                tokens = list(self.tokens if route.demo else self.provider_tokens)
                addresses = None
                if not route.demo:
                    for configured in (self.config or {}).get('routes', []):
                        if (configured['origin_asset'], configured['destination_asset'], str(configured['amount'])) == (
                                route.legs[0].asset_in.asset_id, route.legs[0].asset_out.asset_id, str(route.legs[0].amount_in)):
                            addresses = (configured['origin_address'], configured['destination_address'])
                            break
                    if addresses is None:
                        raise ValueError('route has no configured quote addresses')
                return {'draft': template(route, tokens, now=int(self.clock()), addresses=addresses),
                        'paper_only': True, 'eligible_for_wallet_authorization': False}
        if path == '/api/strategy/compile':
            if set(data) != {'draft'} or not isinstance(data['draft'], dict):
                raise ValueError('one editable .x3 draft required')
            with self.lock:
                route = self.route(data['draft'].get('route_id'))
                tokens = list(self.tokens if route.demo else self.provider_tokens)
            return compile_draft(data['draft'], route, tokens, now=int(self.clock()))
        if path == '/api/strategy/run':
            if set(data) != {'draft', 'key', 'scenario'} or not isinstance(data['draft'], dict):
                raise ValueError('draft, idempotency key and paper scenario required')
            try:
                existing = self.trader.get(data['key'])
            except ValueError:
                existing = None
            if existing is not None:
                stored, _ = self.trader.store.load(data['key'])
                fields = ('route', 'tokens', 'mandates', 'addresses', 'minimum_profit_units', 'max_external_cost_units', 'expires_at')
                snapshot = strategy_from_document({field: stored[field] for field in fields})
                route, tokens = snapshot.route, snapshot.tokens
            else:
                with self.lock:
                    route = self.route(data['draft'].get('route_id'))
                    tokens = list(self.tokens if route.demo else self.provider_tokens)
            strategy = strategy_from_draft(data['draft'], route, tokens, now=int(self.clock()))
            try:
                self.trader.create(data['key'], strategy, mode='paper', scenario=data['scenario'])
            except ni.NearIntentsError as exc:
                raise ValueError(f'{exc.code}: {exc.message}') from exc
            return self.trader.run(data['key'], PaperExchange(self.trader.store))
        if path == '/api/catalogue':
            if not self.catalogue_lock.acquire(blocking=False):
                raise ValueError('catalogue request already running')
            try:
                with self.lock:
                    if self.catalogue and int(self.clock()) - self.catalogue['loaded_at'] < 30:
                        return self.catalogue
                tokens = self.catalog_client.tokens()
                report = token_report(tokens, now=int(self.clock()))
                with self.lock:
                    self.catalogue, self.catalogue_tokens = report, tokens
                return report
            finally:
                self.catalogue_lock.release()
        if path == '/api/routes':
            with self.lock:
                if not self.catalogue:
                    raise ValueError('load the provider token catalogue first')
                if self.refreshing or (self.last_refresh is not None and self.clock()-self.last_refresh < 15):
                    raise ValueError('wait for the current scan and its 15-second request budget')
                row = build_route(data, self.catalogue_tokens)
                additions = {token['assetId']: {'network': token['blockchain'], 'decimals': token['decimals']}
                             for token in self.catalogue_tokens if token['assetId'] in (row['origin_asset'], row['destination_asset'])}
                if any(asset in self.route_asset_pins and self.route_asset_pins[asset] != pin for asset, pin in additions.items()):
                    raise ValueError('asset metadata changed; restart with a reviewed fresh route configuration')
                configured = list(self.config['routes']) if self.config else []
                if len(configured) >= 4:
                    raise ValueError('at most four configured round trips; restart with a fresh configuration to replace them')
                self.config = {'routes': configured + [row]}
                self.route_asset_pins.update(additions)
                self.start_refresh()
                return {'configured_routes': len(self.config['routes']), 'refreshing': True,
                        'transactions_submitted': 0, 'quote_mode': 'dry'}
        if path == '/api/refresh':
            self.start_refresh()
            return {'refreshing': True}
        if path == '/api/paper':
            previous = self.ledger.replay(key=data.get('key'), route_id=data.get('route_id'),
                                          scenario=data.get('scenario', 'projection'))
            if previous is not None:
                return previous
            return self.ledger.project(self.route(data.get('route_id')), key=data.get('key'),
                                       now=int(self.clock()), scenario=data.get('scenario', 'projection'))
        if path == '/api/trade':
            if set(data) != {'route_id', 'key', 'scenario'}:
                raise ValueError('paper cycle requires route_id, key and scenario only')
            try:
                existing = self.trader.get(data['key'])
            except ValueError:
                existing = None
            if existing is not None:
                spec, _ = self.trader.store.load(data['key'])
                if existing['route_id'] != data['route_id'] or spec['scenario'] != data['scenario']:
                    raise ValueError('cycle idempotency key binds another request')
                return self.trader.run(data['key'], PaperExchange(self.trader.store))
            route = self.route(data['route_id'])
            strategy = example_strategy(route, list(self.tokens), now=int(self.clock()))
            self.trader.create(data['key'], strategy, scenario=data['scenario'])
            return self.trader.run(data['key'], PaperExchange(self.trader.store))
        if path == '/api/export':
            route = self.route(data.get('route_id'))
            with self.lock:
                tokens = list(self.tokens if route.demo else self.provider_tokens)
            source = export_leg(route, data.get('leg', 0), tokens=tokens,
                                refund_to=data.get('refund_to'), recipient=data.get('recipient'),
                                now=int(self.clock()), mandate_owner=data.get('mandate_owner') or None)
            return {'source': source, 'filename': f'arb-{route.id[:16]}-{data.get("leg", 0)}.x3',
                    'eligible_for_wallet_authorization': False}
        raise ValueError('unknown action')


def create_server(app, *, port=8787):
    class Handler(BaseHTTPRequestHandler):
        server_version = 'X3Arbitrage/0.1'

        def setup(self):
            super().setup()
            self.connection.settimeout(10)

        def log_message(self, *_args):
            pass  # Request bodies, addresses and capabilities never enter access logs.

        def allowed(self):
            port = self.server.server_port
            hosts = {f'127.0.0.1:{port}', f'localhost:{port}'}
            host = self.headers.get('Host')
            origins = {f'http://{entry}' for entry in hosts}
            origin = self.headers.get('Origin')
            return host in hosts and (origin is None or origin in origins)

        def send(self, status, payload, content_type='application/json'):
            body = json.dumps(payload).encode() if content_type == 'application/json' else payload
            self.send_response(status)
            self.send_header('Content-Type', content_type)
            self.send_header('Content-Length', str(len(body)))
            self.send_header('Cache-Control', 'no-store')
            self.send_header('X-Content-Type-Options', 'nosniff')
            self.send_header('Content-Security-Policy', "default-src 'self'; script-src 'self'; style-src 'self'; "
                             "connect-src 'self'; object-src 'none'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'")
            self.end_headers()
            self.wfile.write(body)

        def do_GET(self):
            if not self.allowed():
                return self.send(403, {'error': 'HOST_OR_ORIGIN_DENIED'})
            path = urlsplit(self.path).path
            if path == '/api/state':
                return self.send(200, app.state())
            files = {'/': ('index.html', 'text/html; charset=utf-8'),
                     '/app.mjs': ('app.mjs', 'text/javascript'),
                     '/format.mjs': ('format.mjs', 'text/javascript'),
                     '/wallet.mjs': ('wallet.mjs', 'text/javascript'),
                     '/wallet-ui.mjs': ('wallet-ui.mjs', 'text/javascript'),
                     '/style.css': ('style.css', 'text/css'),
                     '/icon.svg': ('icon.svg', 'image/svg+xml')}
            if path not in files:
                return self.send(404, {'error': 'NOT_FOUND'})
            filename, mime = files[path]
            self.send(200, (WEB / filename).read_bytes(), mime)

        def do_POST(self):
            if not self.allowed() or not secrets.compare_digest(self.headers.get('X-X3-CSRF', ''), app.csrf):
                return self.send(403, {'error': 'HOST_ORIGIN_OR_CSRF_DENIED'})
            if self.headers.get('Content-Type') != 'application/json' or self.headers.get('Transfer-Encoding'):
                return self.send(415, {'error': 'JSON_CONTENT_TYPE_REQUIRED'})
            try:
                size = int(self.headers.get('Content-Length', '0'))
                if not 1 <= size <= 8192:
                    return self.send(413, {'error': 'BODY_SIZE_LIMIT'})
                data = json.loads(self.rfile.read(size))
                result = app.action(urlsplit(self.path).path, data)
                self.send(200, result)
            except (ValueError, TypeError, KeyError, UnicodeError) as exc:
                self.send(400, {'error': 'REQUEST_REFUSED', 'message': str(exc)[:300]})
            except Exception:
                self.send(503, {'error': 'LOCAL_OPERATION_FAILED',
                                'message': 'Operation failed; no trade was submitted.'})
    return ThreadingHTTPServer(('127.0.0.1', port), Handler)
