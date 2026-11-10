"""`x3_wont_compile.py` is judge-facing, so pin both directions: every unsafe edit is refused with
its own code, and the report fails when an edit compiles, when the code changes, or when the
example program drifts so an edit no longer applies."""
import hashlib
import importlib.util
from pathlib import Path

import pytest

SCRIPT = Path(__file__).resolve().parent / "x3_wont_compile.py"


def _load():
    spec = importlib.util.spec_from_file_location("x3_wont_compile", SCRIPT)
    module = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    spec.loader.exec_module(module)
    return module


wc = _load()


def test_every_unsafe_edit_is_refused_with_its_own_code(tmp_path):
    report = wc.evaluate(tmp_path)
    assert report["ok"] is True and report["baseline_compiles"] is True
    assert report["refused"] == report["passed"] == report["total"] == len(wc.EDITS) == 9
    codes = {r["case"]: (r["code"], r["inner"]) for r in report["cases"]}
    assert codes["no_slippage_bound"][0] == "X3_NEAR_SLIPPAGE_REQUIRED"
    assert codes["second_leg"][0] == "X3_NEAR_ROUTE_SHAPE"
    assert codes["unenforceable_guard"][0] == "X3_NEAR_GUARD_UNENFORCEABLE"
    assert codes["lookalike_recipient"] == ("X3_NEAR_TYPECHECK", ["X3_INVALID_RECEIVER"])
    assert codes["zero_amount"] == ("X3_NEAR_TYPECHECK", ["X3_INVALID_AMOUNT"])


def test_each_edit_changes_exactly_one_thing(tmp_path):
    source = wc.PROGRAM.read_text(encoding="utf-8")
    wc.evaluate(tmp_path)
    for edit in wc.EDITS:
        edited = (tmp_path / f"{edit['case']}.x3").read_text(encoding="utf-8")
        assert edited != source
        assert edited == source.replace(edit["old"], edit["new"], 1)


def test_the_example_program_is_never_modified(tmp_path):
    before = hashlib.sha256(wc.PROGRAM.read_bytes()).hexdigest()
    wc.evaluate(tmp_path)
    assert hashlib.sha256(wc.PROGRAM.read_bytes()).hexdigest() == before


def test_an_edit_that_compiles_fails_the_report(tmp_path, monkeypatch):
    harmless = {"case": "harmless", "tried": "rename the nonce", "old": "agent_payment_001",
                "new": "agent_payment_002", "code": "X3_NEAR_NONCE"}
    monkeypatch.setattr(wc, "EDITS", [*wc.EDITS, harmless])
    report = wc.evaluate(tmp_path)
    row = next(r for r in report["cases"] if r["case"] == "harmless")
    assert row["compiled"] is True and row["pass"] is False
    assert report["ok"] is False


def test_a_different_refusal_code_fails_the_report(tmp_path, monkeypatch):
    edits = [dict(e) for e in wc.EDITS]
    edits[0]["code"] = "X3_NEAR_TIMEOUT_REQUIRED"
    monkeypatch.setattr(wc, "EDITS", edits)
    report = wc.evaluate(tmp_path)
    assert report["cases"][0]["compiled"] is False and report["cases"][0]["pass"] is False
    assert report["ok"] is False


def test_a_missing_inner_type_error_fails_the_report(tmp_path, monkeypatch):
    edits = [dict(e) for e in wc.EDITS]
    zero = next(e for e in edits if e["case"] == "zero_amount")
    zero["inner"] = "X3_INVALID_RECEIVER"
    monkeypatch.setattr(wc, "EDITS", edits)
    assert wc.evaluate(tmp_path)["ok"] is False


def test_a_drifted_example_stops_the_run(tmp_path):
    drifted = tmp_path / "drifted.x3"
    drifted.write_text(wc.PROGRAM.read_text(encoding="utf-8").replace("require slippage <= 0.5%", "require slippage <= 1%"))
    work = tmp_path / "work"
    work.mkdir()
    with pytest.raises(SystemExit, match="no_slippage_bound"):
        wc.evaluate(work, program=drifted)


def test_cli_exit_code_follows_the_verdict(capsys, monkeypatch):
    assert wc.main([]) == 0
    assert "9/9 edits refused at compile time; 9/9 with the expected code." in capsys.readouterr().out
    edits = [dict(e) for e in wc.EDITS]
    edits[0]["code"] = "X3_NEAR_NONCE"
    monkeypatch.setattr(wc, "EDITS", edits)
    assert wc.main([]) == 1
