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


def test_typechecker_rejects_non_finite_amount_and_malformed_constraints():
    root = Path(__file__).resolve().parents[2]
    tc = load_module_from(root / 'x3-lang' / 'typechecker.py')
    bad = parsed_example(root)
    bad['from']['amount'] = 'NaN'
    bad['constraints'] = {'max_slippage': 'not-a-number'}

    ok, errs = tc.typecheck(bad)

    assert not ok
    assert {'X3_INVALID_AMOUNT', 'X3_INVALID_CONSTRAINT'} <= {e.code for e in errs}


def test_typechecker_rejects_an_amount_the_pipeline_cannot_represent():
    # `NaN` above is rejected by `Decimal.is_finite`. `1e400` is not: it *is* a finite
    # `Decimal`. It only becomes `inf` on the narrowing every caller of `parse_decimal`
    # performs (`float(...)`), and `float()` does not raise — so this amount passed
    # validation and the runner reported `expected_profit_usd: NaN` with
    # `estimated_slippage_usd: Infinity` in a `json.dumps` document that RFC 8259 does
    # not define. The bound is the pipeline's own representable range, not a ceiling on
    # how large an amount may be: `1e30` is enormous and stays legal.
    root = Path(__file__).resolve().parents[2]
    tc = load_module_from(root / 'x3-lang' / 'typechecker.py')

    overflow = parsed_example(root)
    overflow['from']['amount'] = '1e400'
    ok, errs = tc.typecheck(overflow)
    assert not ok, 'an amount that narrows to inf is not a large amount, it is unrepresentable'
    assert [e.code for e in errs] == ['X3_INVALID_AMOUNT']

    large = parsed_example(root)
    large['from']['amount'] = '1e30'
    ok, errs = tc.typecheck(large)
    assert ok, [e.to_dict() for e in errs]


def test_typechecker_rejects_malformed_policies_without_crashing():
    root = Path(__file__).resolve().parents[2]
    tc = load_module_from(root / 'x3-lang' / 'typechecker.py')
    bad = parsed_example(root)
    bad['policies'] = {'timeout': '30s'}

    ok, errs = tc.typecheck(bad)

    assert not ok
    assert 'X3_INVALID_POLICY' in {e.code for e in errs}
