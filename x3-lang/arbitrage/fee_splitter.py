"""Splits verified positive net only after Guard-confirmed settlement."""
from typing import Dict, Any


class FeeSplitter:
    """Accounting-only. No new trading. Performance fee on verified positive net."""

    DEFAULT_SPLIT = {'owner': 80, 'operator': 20}  # percent

    def split(self, cycle_state: Dict[str, Any], split: Dict[str, int] = None) -> Dict[str, Any]:
        if cycle_state.get('phase') != 'COMPLETED':
            raise ValueError('only settled completed cycles can be split')
        profit = int(cycle_state.get('paper_profit_units') or 0)
        if profit <= 0:
            return {'profit_units': '0', 'owner_units': '0', 'operator_units': '0', 'split_applied': False}

        ratios = split or self.DEFAULT_SPLIT
        total = sum(ratios.values())
        if total <= 0:
            raise ValueError('invalid split ratios')

        owner_units = (profit * ratios.get('owner', 80)) // total
        operator_units = profit - owner_units
        return {
            'profit_units': str(profit),
            'owner_units': str(owner_units),
            'operator_units': str(operator_units),
            'split_applied': True,
            'asset': cycle_state.get('profit_asset'),
        }
