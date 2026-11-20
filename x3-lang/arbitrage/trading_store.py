"""Durable coordinator state and simulated wallet transactions on real SQLite."""
import json
from hashlib import sha256

from .paper import PaperLedger
from near_liquidity.quote import canonical_json


class TradingStore(PaperLedger):
    """Shares the existing WAL/FULL/timeout connection policy and paper database."""
    def __init__(self, path):
        super().__init__(path)
        with self._connect() as db:
            db.executescript('''
                CREATE TABLE IF NOT EXISTS trade_cycles(
                    id TEXT PRIMARY KEY, fingerprint TEXT NOT NULL, spec TEXT NOT NULL,
                    state TEXT NOT NULL);
                CREATE TABLE IF NOT EXISTS trade_mandates(
                    id TEXT PRIMARY KEY, document TEXT NOT NULL, revoked INTEGER NOT NULL DEFAULT 0);
                CREATE TABLE IF NOT EXISTS trade_spends(
                    operation TEXT PRIMARY KEY, mandate TEXT NOT NULL, amount TEXT NOT NULL, at_ms INTEGER NOT NULL);
                CREATE INDEX IF NOT EXISTS trade_spends_window ON trade_spends(mandate, at_ms);
                CREATE TABLE IF NOT EXISTS trade_reservations(
                    operation TEXT PRIMARY KEY, cycle TEXT NOT NULL, mandate TEXT NOT NULL, amount TEXT NOT NULL, at_ms INTEGER NOT NULL);
                CREATE INDEX IF NOT EXISTS trade_reservations_owner ON trade_reservations(mandate);
                CREATE TABLE IF NOT EXISTS paper_inventory(
                    cycle TEXT NOT NULL, asset TEXT NOT NULL, amount TEXT NOT NULL,
                    PRIMARY KEY(cycle,asset));
                CREATE TABLE IF NOT EXISTS paper_operations(
                    id TEXT PRIMARY KEY, fingerprint TEXT NOT NULL, document TEXT NOT NULL);
            ''')

    def exists(self, key, spec):
        with self._connect() as db:
            old = db.execute('SELECT fingerprint FROM trade_cycles WHERE id=?', (key,)).fetchone()
        if old is None:
            return False
        if old[0] != sha256(canonical_json(spec)).hexdigest():
            raise ValueError('cycle idempotency key already binds a different strategy')
        return True

    def create(self, key, spec, state, mandates, authorize=None):
        fingerprint = sha256(canonical_json(spec)).hexdigest()
        with self._connect() as db:
            db.execute('BEGIN IMMEDIATE')
            old = db.execute('SELECT fingerprint FROM trade_cycles WHERE id=?', (key,)).fetchone()
            if old:
                if old[0] != fingerprint:
                    raise ValueError('cycle idempotency key already binds a different strategy')
                return False
            for item in mandates:
                db.execute('INSERT OR IGNORE INTO trade_mandates(id,document) VALUES(?,?)',
                           (item['id'], json.dumps(item['compiled'])))
            if authorize is not None:
                authorize(db)
            db.execute('INSERT INTO trade_cycles VALUES(?,?,?,?)',
                       (key, fingerprint, json.dumps(spec), json.dumps(state)))
            for asset, amount in state['inventory'].items():
                db.execute('INSERT INTO paper_inventory VALUES(?,?,?)', (key, asset, amount))
            return True

    def load(self, key):
        with self._connect() as db:
            row = db.execute('SELECT spec,state FROM trade_cycles WHERE id=?', (key,)).fetchone()
        if row is None:
            raise ValueError('unknown trading cycle')
        return json.loads(row[0]), json.loads(row[1])

    def save(self, key, state, *, terminal=False):
        with self._connect() as db:
            db.execute('BEGIN IMMEDIATE')
            for item in state.get('funded_operations', []):
                previous = db.execute('SELECT mandate,amount FROM trade_spends WHERE operation=?',
                                      (item['operation'],)).fetchone()
                if previous and previous != (item['mandate'], item['amount']):
                    raise ValueError('funding operation already binds a different spend')
                db.execute('INSERT OR IGNORE INTO trade_spends VALUES(?,?,?,?)',
                           (item['operation'], item['mandate'], item['amount'], item['at_ms']))
                db.execute('DELETE FROM trade_reservations WHERE operation=?', (item['operation'],))
            db.execute('UPDATE trade_cycles SET state=? WHERE id=?', (json.dumps(state), key))
            if terminal:
                db.execute('DELETE FROM trade_reservations WHERE cycle=?', (key,))

    def inventory(self, key):
        with self._connect() as db:
            return dict(db.execute('SELECT asset,amount FROM paper_inventory WHERE cycle=?', (key,)))

    def mandate(self, identity, *, db=None, exclude_operation=None):
        if db is None:
            with self._connect() as connection:
                return self.mandate(identity, db=connection, exclude_operation=exclude_operation)
        row = db.execute('SELECT document,revoked FROM trade_mandates WHERE id=?', (identity,)).fetchone()
        if row is None:
            raise ValueError('mandate not registered')
        document = json.loads(row[0])
        document['revoked'] = bool(row[1])
        document['spends'] = [{'amount': amount, 'at_ms': at_ms} for amount, at_ms in db.execute(
            'SELECT amount,at_ms FROM trade_spends WHERE mandate=?', (identity,))]
        document['spends'].extend({'amount': amount, 'at_ms': at_ms} for operation, amount, at_ms in db.execute(
            'SELECT operation,amount,at_ms FROM trade_reservations WHERE mandate=?', (identity,))
            if operation != exclude_operation)
        return document

    def release_reservations(self, key):
        with self._connect() as db:
            db.execute('DELETE FROM trade_reservations WHERE cycle=?', (key,))

    def revoke(self, identity):
        with self._connect() as db:
            if db.execute('UPDATE trade_mandates SET revoked=1 WHERE id=?', (identity,)).rowcount != 1:
                raise ValueError('unknown mandate')

    def cycles(self):
        with self._connect() as db:
            return [json.loads(row[0]) for row in db.execute(
                'SELECT state FROM trade_cycles ORDER BY rowid DESC LIMIT 100')]
