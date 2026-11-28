"""Public constrained-liquidity status for other agents."""
from typing import Dict, Any, List

from .mandate_proof import MandateProof


class LiquidityListing:
    """Advertises remaining room, allowed assets, and proof hash."""

    def __init__(self, proof: MandateProof):
        self.proof = proof

    def listing(self, mandate_ids: List[str], now_ms: int) -> Dict[str, Any]:
        entries = []
        for mid in mandate_ids:
            try:
                snap = self.proof.snapshot(mid, now_ms)
                entries.append({
                    'mandate_id': mid,
                    'remaining_incomplete_notional': snap['remaining_incomplete_notional'],
                    'allowed_assets': snap['allowed_assets'],
                    'allowed_recipients': snap['allowed_recipients'],
                    'proof_hash': snap['proof_hash'],
                    'revoked': snap['revoked'],
                })
            except Exception:
                continue
        return {
            'schema': 'x3.liquidity.listing.v1',
            'entries': entries,
            'observed_at_ms': now_ms,
            'note': 'Remaining room is local sequential inventory under registered mandate. Not a liquidity reservation.',
        }
