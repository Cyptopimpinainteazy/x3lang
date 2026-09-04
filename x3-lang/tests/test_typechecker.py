import importlib.util
import json
import subprocess
import sys
from pathlib import Path


def load_module_from(path):
    spec = importlib.util.spec_from_file_location(path.stem, str(path))
    mod = importlib.util.module_from_spec(spec)
    sys.path.insert(0, str(path.parent))
    try:
        spec.loader.exec_module(mod)
    finally:
        sys.path.pop(0)
    return mod


def parsed_example(root):
    proc = subprocess.run([sys.executable, str(root / 'x3-lang' / 'cli.py'), str(root / 'x3-lang' / 'examples' / 'arb_solana_eth.x3')], capture_output=True, check=True)
    return json.loads(proc.stdout.decode())


def test_typechecker_accepts_valid_production_intent():
    root = Path(__file__).resolve().parents[2]
    tc = load_module_from(root / 'x3-lang' / 'typechecker.py')
    ok, errs = tc.typecheck(parsed_example(root))
    assert ok, [e.to_dict() for e in errs]


def test_typechecker_rejects_invalid_chain_asset_receiver():
    root = Path(__file__).resolve().parents[2]
    tc = load_module_from(root / 'x3-lang' / 'typechecker.py')
    bad = parsed_example(root)
    bad['from']['chain'] = 'mars'
    bad['to']['receiver'] = 'not-an-evm-address'
    bad['route'][0]['from_ref']['asset'] = 'FAKE'
    ok, errs = tc.typecheck(bad)
    codes = {e.code for e in errs}
    assert not ok
    assert {'X3_INVALID_CHAIN', 'X3_INVALID_RECEIVER', 'X3_INVALID_ASSET'} & codes


def test_planner_outputs_cross_chain_steps():
    root = Path(__file__).resolve().parents[2]
    planner = load_module_from(root / 'x3-lang' / 'planner.py')
    plan = planner.plan(parsed_example(root))
    assert len(plan['steps']) == 3
    assert any(step['type'] == 'bridge' for step in plan['steps'])
