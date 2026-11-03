import json
import subprocess
import sys
from pathlib import Path


def test_cli_parses_production_intent_example():
    root = Path(__file__).resolve().parents[2]
    example = root / 'x3-lang' / 'examples' / 'arb_solana_eth.x3'
    proc = subprocess.run([sys.executable, str(root / 'x3-lang' / 'cli.py'), str(example)], capture_output=True, check=True)
    out = json.loads(proc.stdout.decode())
    assert out['intent'] == 'arb_solana_eth'
    assert out['from']['chain'] == 'solana'
    assert out['to']['receiver'].startswith('0x')
    assert len(out['route']) == 3
    assert {step['type'] for step in out['route']} == {'swap', 'bridge'}
    assert any(req['kind'] == 'nonce' for req in out['requires'])
    assert out['policies']['timeout']['action']['type'] == 'refund'


def test_cli_rejects_malformed_route(tmp_path):
    root = Path(__file__).resolve().parents[2]
    bad = tmp_path / 'bad.x3'
    bad.write_text('intent bad {\n from Solana.USDC amount 1\n to Ethereum.USDC\n route {\n swap Raydium Solana.USDC Solana.SOL\n }\n}\n')
    proc = subprocess.run([sys.executable, str(root / 'x3-lang' / 'cli.py'), str(bad)], capture_output=True)
    assert proc.returncode != 0
    err = json.loads(proc.stderr.decode())
    assert err['errors'][0]['code'].startswith('X3_PARSE')
