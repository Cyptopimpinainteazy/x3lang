import importlib.util
import json
import subprocess
import sys
from pathlib import Path


def test_simulator_produces_estimates():
    root = Path(__file__).resolve().parents[2]
    proc = subprocess.run([sys.executable, str(root / 'x3-lang' / 'cli.py'), str(root / 'x3-lang' / 'examples' / 'arb_solana_eth.x3')], capture_output=True, check=True)
    intent = json.loads(proc.stdout.decode())

    spec = importlib.util.spec_from_file_location('planner', str(root / 'x3-lang' / 'planner.py'))
    planner = importlib.util.module_from_spec(spec)
    sys.path.insert(0, str(root / 'x3-lang'))
    try:
        spec.loader.exec_module(planner)
    finally:
        sys.path.pop(0)
    plan = planner.plan(intent)

    spec = importlib.util.spec_from_file_location('simulator', str(root / 'x3-lang' / 'simulator.py'))
    simulator = importlib.util.module_from_spec(spec)
    sys.path.insert(0, str(root / 'x3-lang'))
    try:
        spec.loader.exec_module(simulator)
    finally:
        sys.path.pop(0)
    out = simulator.simulate(plan)

    assert 'estimates' in out
    e = out['estimates']
    assert isinstance(e.get('estimated_gas_usd'), float)
    assert isinstance(e.get('estimated_bridge_fee_usd'), float)
    assert 'expected_profit_usd' in e
