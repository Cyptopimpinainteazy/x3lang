import importlib.util
import json
import subprocess
import sys
from pathlib import Path


def load_module(path):
    spec = importlib.util.spec_from_file_location(path.stem, str(path))
    module = importlib.util.module_from_spec(spec)
    sys.path.insert(0, str(path.parent))
    try:
        spec.loader.exec_module(module)
    finally:
        sys.path.pop(0)
    return module


def test_emitter_generates_chain_payloads():
    root = Path(__file__).resolve().parents[2]
    proc = subprocess.run([sys.executable, str(root / 'x3-lang' / 'cli.py'), str(root / 'x3-lang' / 'examples' / 'arb_solana_eth.x3')], capture_output=True, check=True)
    intent = json.loads(proc.stdout.decode())

    planner = load_module(root / 'x3-lang' / 'planner.py')
    plan = planner.plan(intent)
    simulator = load_module(root / 'x3-lang' / 'simulator.py')
    plan = simulator.simulate(plan)

    emitter = load_module(root / 'x3-lang' / 'emitter' / '__init__.py')
    output = emitter.emit(plan)

    assert 'emitted' in output
    assert isinstance(output['emitted'], list)
    assert any(item['chain'] == 'solana' for item in output['emitted'])
    assert any(item['chain'] == 'ethereum' for item in output['emitted'])
    assert any(item['chain'] == 'x3' for item in output['emitted'])


def test_svm_emitter_encodes_ray_dium_swap_with_base58_data():
    root = Path(__file__).resolve().parents[2]
    proc = subprocess.run([sys.executable, str(root / 'x3-lang' / 'cli.py'), str(root / 'x3-lang' / 'examples' / 'arb_solana_eth.x3')], capture_output=True, check=True)
    intent = json.loads(proc.stdout.decode())

    planner = load_module(root / 'x3-lang' / 'planner.py')
    plan = planner.plan(intent)
    simulator = load_module(root / 'x3-lang' / 'simulator.py')
    plan = simulator.simulate(plan)

    emitter = load_module(root / 'x3-lang' / 'emitter' / '__init__.py')
    output = emitter.emit(plan)
    svm = next(item for item in output['emitted'] if item['chain'] == 'solana')

    assert svm['program'] == '4k3Dyjzvzp8e2y8bKE8xUrg8rXQZkmh8Y2xS1QZsbJt4'
    assert isinstance(svm['data'], str) and len(svm['data']) > 0
    assert svm['raw_data_hex'].startswith('0x01')
    assert isinstance(svm['accounts'], list)
    assert any(acc['pubkey'] == 'EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v' for acc in svm['accounts'])
    assert any(acc['pubkey'] == '11111111111111111111111111111111' for acc in svm['accounts'])


def test_evm_emitter_uses_uniswap_selector_and_target_contract():
    root = Path(__file__).resolve().parents[2]
    proc = subprocess.run([sys.executable, str(root / 'x3-lang' / 'cli.py'), str(root / 'x3-lang' / 'examples' / 'arb_solana_eth.x3')], capture_output=True, check=True)
    intent = json.loads(proc.stdout.decode())

    planner = load_module(root / 'x3-lang' / 'planner.py')
    plan = planner.plan(intent)
    simulator = load_module(root / 'x3-lang' / 'simulator.py')
    plan = simulator.simulate(plan)

    emitter = load_module(root / 'x3-lang' / 'emitter' / '__init__.py')
    output = emitter.emit(plan)
    evm = next(item for item in output['emitted'] if item['chain'] == 'ethereum')

    assert evm['calldata'].startswith('0x38ed1739')
    assert evm['target_contract'] == '0xUniswapRouter000000000000000000000000'
    byte_length = len(bytes.fromhex(evm['calldata'][2:]))
    assert byte_length == 260


def test_x3_emitter_builds_settlement_payload_and_proof_bundle():
    root = Path(__file__).resolve().parents[2]
    proc = subprocess.run([sys.executable, str(root / 'x3-lang' / 'cli.py'), str(root / 'x3-lang' / 'examples' / 'arb_solana_eth.x3')], capture_output=True, check=True)
    intent = json.loads(proc.stdout.decode())

    planner = load_module(root / 'x3-lang' / 'planner.py')
    plan = planner.plan(intent)
    simulator = load_module(root / 'x3-lang' / 'simulator.py')
    plan = simulator.simulate(plan)

    emitter = load_module(root / 'x3-lang' / 'emitter' / '__init__.py')
    output = emitter.emit(plan)
    x3 = next(item for item in output['emitted'] if item['chain'] == 'x3')

    assert x3['type'] == 'bridge'
    assert x3['asset'] == 'SOL'
    assert x3['payload']['action'] == 'settle'
    assert x3['payload']['source_chain'] == 'solana'
    assert x3['payload']['target_chain'] == 'ethereum'
    assert x3['transaction_bytes'].startswith('0x')
    assert x3['proof_bundle']['bundle_version'] == '0.1'
    assert isinstance(x3['proof_bundle']['signatures'], list)
    # The emitter must never fabricate validator signatures/signing keys.
    assert x3['proof_bundle']['signatures'] == []
    assert x3['proof_bundle'].get('error_code') == 'X3_PROOF_REQUIRED'
