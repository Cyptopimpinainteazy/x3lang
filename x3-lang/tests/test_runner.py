import json
import subprocess
import sys
import tempfile
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
    contract = result['validated_intent_v1']
    assert contract['schema_version'] == 1
    # The example states its amounts in each asset's base units, because `amount` and
    # `min_output` carry no asset and the compiler refuses a fractional literal rather
    # than converting against decimals it cannot see. 10 USDC at six decimals is
    # 10_000_000 — and the assertion follows the example rather than the other way round.
    # The text is the formatter's, which drops the `_` separators, so it reads `10000000`.
    assert contract['from']['amount'] == '10000000'
    assert contract['path'][0]['type'] == 'swap'


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


def test_runner_reports_a_parse_failure_as_a_structured_error():
    """A file this surface cannot read is a diagnosis, not a traceback.

    Five of the repository's examples declare no `intent`, and this surface reads one intent
    per file. It used to raise `IndexError: list index out of range` from inside
    `parse_file`, and the runner let even a clean `X3ParseError` escape as a Python stack —
    so the one surface a user actually runs reported neither a code nor a line. It reports
    both now, in the same JSON shape as a typechecker failure, and exits 1.
    """
    root = Path(__file__).resolve().parents[2]
    runner = root / "x3-lang" / "runner.py"
    # Its own fixture rather than one of the repository's examples: the subject is *a file
    # with no intent*, and writing it here keeps this test from being coupled to a file that
    # someone may later fix — and, since `tests/test_surface_drift.py` derives its list of
    # examples from this suite's source, from pulling an unreadable example into that gate.
    no_intent = Path(tempfile.mkdtemp()) / "no_intent.x3"
    no_intent.write_text(
        "parallel two_way_arb {\n    settlement atomic;\n}\n"
    )

    proc = subprocess.run(
        [sys.executable, str(runner), "--dry-run", str(no_intent)], capture_output=True
    )
    assert proc.returncode == 1, "a refusal must not be reported as success"
    assert b"Traceback" not in proc.stderr, (
        f"a refusal must not be a traceback: {proc.stderr.decode()}"
    )
    result = json.loads(proc.stdout.decode())
    assert result["status"] == "error"
    error = result["errors"][0]
    assert error["code"] == "X3_PARSE_NO_INTENT"
    assert isinstance(error["line"], int) and error["line"] > 0, (
        "a diagnosis without a line is half a diagnosis"
    )


def test_runner_refuses_an_amount_it_cannot_represent_instead_of_reporting_nan():
    # End to end, because the defect was end to end: the parser accepted the literal, the
    # typechecker accepted the `Decimal`, `planner.py` narrowed it with `float()` and got
    # `inf`, and the run finished with `status: rolled_back` and an estimates block
    # carrying `NaN` and `Infinity`. Nothing raised, so nothing said so — the only visible
    # sign was that the output document stopped being JSON.
    root = Path(__file__).resolve().parents[2]
    runner = root / "x3-lang" / "runner.py"
    source = "\n".join(
        [
            "intent overflow {",
            "    from ethereum.USDC amount 1e400 receiver 0x1111111111111111111111111111111111111111",
            "",
            "    route {",
            "        swap Uniswap ethereum.USDC -> ethereum.WETH amount 1000 min_output 900",
            "    }",
            "",
            "    to ethereum.WETH receiver 0x1111111111111111111111111111111111111111",
            "    require finality.ethereum >= 12",
            "}",
            "",
        ]
    )
    path = Path(tempfile.mkdtemp()) / "overflow.x3"
    path.write_text(source)

    proc = subprocess.run(
        [sys.executable, str(runner), "--no-schema", str(path)], capture_output=True
    )
    document = proc.stdout.decode()
    assert proc.returncode == 1, "an unrepresentable amount must not be reported as success"
    assert b"Traceback" not in proc.stderr, (
        f"a refusal must be a diagnosis, not a traceback: {proc.stderr.decode()}"
    )
    result = json.loads(document)
    assert result["status"] == "error", document
    assert [e["code"] for e in result["errors"]] == ["X3_INVALID_AMOUNT"], document
    # The two tokens the float narrowing produced, neither of which is JSON.
    assert "Infinity" not in document and "NaN" not in document, document
