"""Verifiable snapshot of current sequential mandate state for external agents."""
import json
from hashlib import sha256
from typing import Any, Dict

from .trading_store import TradingStore


class MandateProof:
    """Produces a signed-looking proof document (hash-bound) of remaining room."""

    def __init__(self, store: TradingStore):
        self.store = store

    def snapshot(self, mandate_id: str, now_ms: int) -> Dict[str, Any]:
        mandate = self.store.mandate(mandate_id)
        # Remaining inventory room: max_incomplete_cycle_notional minus open reservations
        open_notional = sum(int(s['amount']) for s in mandate.get('spends', []) if s.get('at_ms', 0) > now_ms - 86400000)
        max_room = int(mandate.get('compiled', {}).get('terms', {}).get('max_incomplete_cycle_notional', 0) or 0)
        remaining = max(0, max_room - open_notional)

        payload = {
            'schema': 'x3.mandate.proof.v1',
            'mandate_id': mandate_id,
            'owner': mandate.get('compiled', {}).get('owner'),
            'agent': mandate.get('compiled', {}).get('agent'),
            'asset': mandate.get('compiled', {}).get('terms', {}).get('asset'),
            'remaining_incomplete_notional': str(remaining),
            'max_incomplete_cycle_notional': str(max_room),
            'allowed_recipients': mandate.get('compiled', {}).get('terms', {}).get('recipients', []),
            'allowed_assets': mandate.get('compiled', {}).get('terms', {}).get('assets', []),
            'expires_at': mandate.get('compiled', {}).get('terms', {}).get('expires_at'),
            'revoked': bool(mandate.get('revoked', False)),
            'observed_at_ms': now_ms,
        }
        payload['proof_hash'] = sha256(json.dumps(payload, sort_keys=True, separators=(',', ':')).encode()).hexdigest()
        return payload

    def verify(self, proof: Dict[str, Any]) -> bool:
        expected = proof.copy()
        expected.pop('proof_hash', None)
        recomputed = sha256(json.dumps(expected, sort_keys=True, separators=(',', ':')).encode()).hexdigest()
        return recomputed == proof.get('proof_hash') and not proof.get('revoked', False)
