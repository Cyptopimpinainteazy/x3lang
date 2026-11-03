from typing import Any, Dict, List


def execute_dry_run(emitted_steps: List[Dict[str, Any]]) -> List[Dict[str, Any]]:
    """Explicit dry-run adapter. Never used by production execution."""
    results = []
    for step in emitted_steps:
        if step.get('error'):
            results.append({'ok': False, 'mode': 'dry-run', 'step': step, 'reason': step.get('error'), 'code': step.get('error_code')})
        else:
            results.append({'ok': True, 'mode': 'dry-run', 'step': step, 'simulated_tx_id': f"dry-run-{step.get('type')}-{step.get('chain')}"})
    return results


def execute(emitted_steps: List[Dict[str, Any]]) -> List[Dict[str, Any]]:
    raise RuntimeError('mock_rpc.execute removed from production path; use execute_dry_run via runner --dry-run')
