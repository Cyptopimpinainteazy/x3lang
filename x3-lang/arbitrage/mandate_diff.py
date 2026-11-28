"""Paper-to-live mandate predicate checker."""
from typing import Dict, Any, List


class MandateDiff:
    """Flags live quotes that would have been refused by sequential predicates."""

    def compare(self, paper_checks: List[Dict], live_checks: List[Dict]) -> Dict[str, Any]:
        paper_failed = {c['guard'] for c in paper_checks if not c.get('ok')}
        live_failed = {c['guard'] for c in live_checks if not c.get('ok')}
        only_paper = sorted(paper_failed - live_failed)
        only_live = sorted(live_failed - paper_failed)
        return {
            'schema': 'x3.mandate.diff.v1',
            'paper_failed': sorted(paper_failed),
            'live_failed': sorted(live_failed),
            'only_in_paper': only_paper,
            'only_in_live': only_live,
            'aligned': not only_paper and not only_live,
        }

    def would_refuse(self, checks: List[Dict]) -> bool:
        return any(not c.get('ok') for c in checks)
