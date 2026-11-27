"""Crash-safe, idempotent projections; no orders are submitted and no fills are inferred."""
import json
import re
import sqlite3
from pathlib import Path
from contextlib import contextmanager

from .engine import assess


class PaperLedger:
    def __init__(self, path):
        self.path = Path(path)
        self.path.parent.mkdir(parents=True, exist_ok=True)
        with self._connect() as db:
            db.execute('PRAGMA journal_mode=WAL')
            db.execute('CREATE TABLE IF NOT EXISTS paper_orders '
                       '(id INTEGER PRIMARY KEY, request_key TEXT UNIQUE NOT NULL, '
                       'route_id TEXT NOT NULL, scenario TEXT NOT NULL, result TEXT NOT NULL)')

    @contextmanager
    def _connect(self):
        db = sqlite3.connect(self.path, timeout=5)
        try:
            db.execute('PRAGMA synchronous=FULL')
            with db:
                yield db
        finally:
            db.close()

    def project(self, route, *, key, now, scenario='projection'):
        if not isinstance(key, str) or not re.fullmatch(r'[A-Za-z0-9_-]{1,100}', key):
            raise ValueError('invalid idempotency key')
        outcomes = {'projection': 'projected', 'timeout': 'simulated_timeout',
                    'refund': 'simulated_refund', 'rpc_failure': 'simulated_rpc_failure'}
        if scenario not in outcomes:
            raise ValueError('unknown paper scenario')
        with self._connect() as db:
            db.execute('BEGIN IMMEDIATE')
            previous = db.execute('SELECT route_id, scenario, result FROM paper_orders '
                                  'WHERE request_key=?', (key,)).fetchone()
            if previous:
                if previous[:2] != (route.id, scenario):
                    raise ValueError('idempotency key already binds a different request')
                return json.loads(previous[2])
            assessment = assess(route, now=now)
            result = {'key': key, 'route_id': route.id, 'created_at': now,
                      'simulation_only': True,
                      'status': outcomes[scenario] if assessment['paper_eligible'] else 'rejected',
                      'scenario': scenario, 'assessment': assessment,
                      'realized_profit_units': None, 'transactions_submitted': 0}
            db.execute('INSERT INTO paper_orders(request_key, route_id, scenario, result) '
                       'VALUES(?,?,?,?)', (key, route.id, scenario, json.dumps(result)))
            return result

    def replay(self, *, key, route_id, scenario='projection'):
        if not isinstance(key, str) or not re.fullmatch(r'[A-Za-z0-9_-]{1,100}', key):
            raise ValueError('invalid idempotency key')
        with self._connect() as db:
            previous = db.execute('SELECT route_id, scenario, result FROM paper_orders '
                                  'WHERE request_key=?', (key,)).fetchone()
        if previous is None:
            return None
        if previous[:2] != (route_id, scenario):
            raise ValueError('idempotency key already binds a different request')
        return json.loads(previous[2])

    def orders(self, limit=100):
        if type(limit) is not int or not 1 <= limit <= 500:
            raise ValueError('limit must be 1..500')
        with self._connect() as db:
            return [json.loads(row[0]) for row in db.execute(
                'SELECT result FROM paper_orders ORDER BY id DESC LIMIT ?', (limit,))]
