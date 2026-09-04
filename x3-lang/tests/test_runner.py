import json
import subprocess
import sys
from pathlib import Path


def test_runner_dry_run_end_to_end():
    root = Path(__file__).resolve().parents[2]
    runner = root / 'x3-lang' / 'runner.py'
    example = root / 'x3-lang' / 'examples' / 'arb_solana_eth.x3'

    # Simulation mode is explicit via --dry-run.
    proc = subprocess.run([sys.executable, str(runner), "--dry-run", str(example)], capture_output=True, check=True)
    result = json.loads(proc.stdout.decode())

    assert result.get('status') == 'ok'
    assert 'steps' in result
    assert 'emitted' in result
    assert isinstance(result['emitted'], dict)
    assert 'constraint_results' in result
    assert any(r['constraint'] == 'atomic' for r in result['constraint_results'])
    assert result['intent'] == 'arb_solana_eth'


def test_runner_defaults_to_fail_closed_without_backend():
    root = Path(__file__).resolve().parents[2]
    runner = root / 'x3-lang' / 'runner.py'
    example = root / 'x3-lang' / 'examples' / 'arb_solana_eth.x3'

    # Default invocation must not silently fall back to dry-run.
    proc = subprocess.run([sys.executable, str(runner), str(example)], capture_output=True, check=True)
    result = json.loads(proc.stdout.decode())

    assert result.get('status') == 'rolled_back'
    assert any(
        e.get('code') == 'X3_BACKEND_REQUIRED'
        for e in result.get('execution', [])
    )
