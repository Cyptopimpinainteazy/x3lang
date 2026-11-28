"""Narrow bot that only closes or unwinds open sequential cycles."""
from typing import List, Dict, Any

from .trader import Trader


class CycleCloser:
    """Watches open cycles and forces re-quote or refund path under existing Guard rules."""

    def __init__(self, trader: Trader):
        self.trader = trader

    def open_cycles(self) -> List[Dict[str, Any]]:
        return [c for c in self.trader.cycles() if c.get('phase') not in {'COMPLETED', 'REJECTED', 'REFUNDED', 'DISARMED'}]

    def close_or_unwind(self, key: str, backend, now: int) -> Dict[str, Any]:
        """Attempt one tick. Never opens new cycles."""
        state = self.trader.get(key)
        if state['phase'] in {'COMPLETED', 'REJECTED', 'REFUNDED', 'DISARMED'}:
            return state
        # Force a re-quote if holding intermediate and approaching expiry or inventory pressure
        if state.get('phase') == 'HOLDING' and state.get('leg_index', 0) > 0:
            # Existing tick path already re-quotes on QUOTE phase; nudge by resetting if safe
            state['phase'] = 'QUOTE'
            self.trader.store.save(key, state)
        return self.trader.tick(key, backend)

    def scan_and_close(self, backend, now: int) -> List[Dict[str, Any]]:
        results = []
        for cycle in self.open_cycles():
            try:
                results.append(self.close_or_unwind(cycle['id'], backend, now))
            except Exception as exc:
                results.append({'id': cycle['id'], 'error': str(exc)[:200]})
        return results
