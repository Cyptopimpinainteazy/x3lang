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

WEB = Path(__file__).parent / 'web'


class App:
    def __init__(self, *, mode, database, config=None, clock=time.time):
        if mode not in {'demo', 'live'}:
            raise ValueError('mode must be demo or live')
        self.mode, self.clock = mode, clock
        self.config = validate_config(config) if config is not None else None
        self.ledger = PaperLedger(database)
        self.trader = Trader(database, clock=clock)
        self.csrf = secrets.token_urlsafe(32)
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
                if self.config:
                    try:
                        cross, tokens, cross_errors = scan_cross_chain(self.config, now=now)
                        routes.extend(cross)
                        errors.extend(cross_errors)
                    except Exception:
                        errors.append({'provider': 'oneclick', 'code': 'CATALOG_UNAVAILABLE',
                                       'message': 'Token catalog unavailable or invalid; no substitute quotes used.'})
                else:
                    errors.append({'provider': 'oneclick', 'code': 'ROUTES_NOT_CONFIGURED',
                                   'message': 'Configure exact assets, addresses and external costs for cross-chain discovery.'})
            with self.lock:
                self.routes, self.tokens, self.errors = routes, tokens, errors
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
            return {'schema': 'x3.arbitrage.v1', 'mode': self.mode, 'csrf': self.csrf,
                    'now': int(self.clock()), 'updated_at': self.updated_at,
                    'refreshing': self.refreshing,
                    'routes': rank(self.routes, now=int(self.clock())),
                    'errors': list(self.errors), 'orders': self.ledger.orders(),
                    'cycles': [cycle for cycle in self.trader.cycles() if cycle['mode'] == 'paper'],
                    'asset_symbols': {token['blockchain'] + '/' + token['assetId']: token['symbol']
                                      for token in self.tokens},
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
                tokens = list(self.tokens)
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
