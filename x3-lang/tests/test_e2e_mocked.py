import json
import subprocess
import sys
from pathlib import Path


def test_runner_dry_run_is_explicit_and_not_production_settlement():
    root = Path(__file__).resolve().parents[2]
    proc = subprocess.run([sys.executable, str(root / 'x3-lang' / 'runner.py'), str(root / 'x3-lang' / 'examples' / 'arb_solana_eth.x3'), '--dry-run', '--no-schema'], capture_output=True, check=True)
    result = json.loads(proc.stdout.decode())
    assert result['status'] == 'ok'
    assert result['execution']
    assert all(step['mode'] == 'dry-run' for step in result['execution'])
    assert any(out.get('proof_required') for out in result['emitted']['emitted'])


def test_runner_production_without_backend_rolls_back():
    root = Path(__file__).resolve().parents[2]
    env = {**__import__('os').environ, 'X3_LANG_LEGACY': '1'}
    proc = subprocess.run(
        [sys.executable, str(root / 'x3-lang' / 'runner.py'), str(root / 'x3-lang' / 'examples' / 'arb_solana_eth.x3'), '--no-schema'],
        capture_output=True, check=True, env=env,
    )
    result = json.loads(proc.stdout.decode())
    assert result['status'] == 'rolled_back'
    assert result['execution'][0]['ok'] is False
    assert result['execution'][0]['code'] in {'X3_BACKEND_REQUIRED', 'X3_PROOF_REQUIRED'}
