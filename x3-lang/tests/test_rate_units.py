"""Rate bounds carry their unit end to end (issue #241).

`require slippage <= 50 bps` used to reach the pipeline as a bare `50`: the parser kept only
the first word of the bound. On the NEAR path that was refused with a message recommending
the very spelling that had just been dropped; on the generic runner it was read as a fraction,
5000%. These tests pin the parser, the shared `split_rate`, the typechecker and the runner.
"""
import sys
from decimal import Decimal
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))

import cli  # noqa: E402
import runner  # noqa: E402
import typechecker  # noqa: E402
from numeric import NumericParseError, split_rate  # noqa: E402


@pytest.mark.parametrize("text,expected", [
    ("0.5%", (Decimal("0.5"), "%")),
    ("0.5 %", (Decimal("0.5"), "%")),
    ("50 bps", (Decimal("50"), "bps")),
    ("50bps", (Decimal("50"), "bps")),
    (" 50 BPS ", (Decimal("50"), "bps")),
    ("0.005", (Decimal("0.005"), None)),
    ("50", (Decimal("50"), None)),
])
def test_split_rate_returns_the_number_and_the_unit_written(text, expected):
    assert split_rate(text) == expected


@pytest.mark.parametrize("text", ["", "bps", "%", "50 bps extra", "5 0 bps", "50 percent", "abc", "1e400%", None, True])
def test_split_rate_refuses_anything_it_would_have_to_drop_or_guess(text):
    with pytest.raises(NumericParseError):
        split_rate(text)


def _parse(tmp_path, bound):
    src = (ROOT / "examples" / "near_intents_agent_payment.x3").read_text()
    assert "require slippage <= 0.5%" in src
    path = tmp_path / "p.x3"
    path.write_text(src.replace("require slippage <= 0.5%", f"require slippage <= {bound}"))
    return cli.parse_file(str(path))


@pytest.mark.parametrize("bound", ["50 bps", "50bps", "0.5%", "0.5 %"])
def test_the_parser_keeps_the_whole_bound(tmp_path, bound):
    intent = _parse(tmp_path, bound)
    (slippage,) = [r for r in intent["requires"] if r["kind"] == "slippage"]
    assert slippage["value"] == bound
    assert intent["constraints"]["max_slippage"] == bound


@pytest.mark.parametrize("bound,ok", [("50 bps", True), ("50bps", True), ("0.5%", True), ("50 bps extra", False)])
def test_the_typechecker_accepts_bps_and_refuses_trailing_words(tmp_path, bound, ok):
    good, errors = typechecker.typecheck(_parse(tmp_path, bound))
    assert good is ok, typechecker.errors_to_json(errors)
    if not ok:
        assert "X3_INVALID_CONSTRAINT" in {e.code for e in errors}


def _slippage_result(max_slippage, slippage_usd):
    plan = {"from": {"amount": "1000"}, "constraints": {"max_slippage": max_slippage},
            "estimates": {"estimated_slippage_usd": slippage_usd}}
    (result,) = [r for r in runner.evaluate_constraints(plan) if r["constraint"] == "max_slippage"]
    return result


@pytest.mark.parametrize("bound", ["50 bps", "50bps", "0.5%"])
def test_the_runner_reads_bps_as_basis_points(bound):
    assert _slippage_result(bound, 4.0)["max_slippage_pct"] == pytest.approx(0.005)
    assert _slippage_result(bound, 4.0)["ok"] is True      # 0.4% of 1000 is within 0.5%
    assert _slippage_result(bound, 6.0)["ok"] is False     # 0.6% is not


def test_the_runner_fails_closed_on_an_unreadable_bound():
    result = _slippage_result("50 bps extra", 1.0)
    assert result == {"constraint": "max_slippage", "ok": False, "error": "unparseable max_slippage"}
