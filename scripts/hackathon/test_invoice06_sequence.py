"""The invoice_06 sequence is judge-facing: the written payment, its compiler
rejections, the Guard checks, the mainnet transfers, and the recorded outcome
succeed together, and a drifted edit or a passing unsafe edit fails the report."""
import hashlib
import importlib.util
from pathlib import Path

SCRIPT = Path(__file__).resolve().parent / "invoice06_sequence.py"


def _load():
    spec = importlib.util.spec_from_file_location("invoice06_sequence", SCRIPT)
    module = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    spec.loader.exec_module(module)
    return module


seq = _load()


def test_five_stages_pass_for_the_archived_payment(tmp_path):
    report = seq.evaluate(tmp_path)
    assert report["ok"] is True
    stages = report["stages"]
    assert stages["written"]["payment_sha256"] == (
        "9ee29a5603cfa1e7d9fdbd866cf330ce35e6b5f0907c6844dd2ef1a213768ba0"
    )
    assert stages["written"]["mandate_sha256"] == (
        "eea021d5cc1118801f4e1d3b9c0efe89fc59a7f654031f9ff1951aa1f2f3d515"
    )
    compiler = stages["compiler_rejection"]
    assert compiler["baseline_compiles"] is True
    assert compiler["refused"] == compiler["total"] == 6
    codes = {row["case"]: row["code"] for row in compiler["cases"]}
    assert codes["no_slippage_bound"] == "X3_NEAR_SLIPPAGE_REQUIRED"
    assert codes["lookalike_recipient"] == "X3_NEAR_TYPECHECK"
    assert codes["zero_amount"] == "X3_NEAR_TYPECHECK"
    guard = stages["guard"]
    assert guard["term_diffs"] == [] and guard["policy_diffs"] == []
    assert guard["local_refused"] == ["mandate_recipient"]
    assert guard["on_chain_count"] == 21 and guard["on_chain_all_ok"] is True
    assert stages["payment"]["amount_in"] == "25000000"
    assert stages["payment"]["amount_out"] == "24995200"
    outcome = stages["outcome"]
    assert outcome["status"] == "Settled" and outcome["floor_met"] is True
    assert outcome["historical_observation_outcome"] is None
    assert outcome["transaction"] == "9sTn9pJ1ZbCVazBm9Wz9oEDh6aWw4FUVMoAJEKCCzuqE"


def test_sources_are_not_modified(tmp_path):
    before = {path: hashlib.sha256(path.read_bytes()).hexdigest() for path in (seq.PAYMENT, seq.MANDATE)}
    seq.evaluate(tmp_path)
    after = {path: hashlib.sha256(path.read_bytes()).hexdigest() for path in before}
    assert after == before


def test_an_edit_that_compiles_fails_the_sequence(tmp_path, monkeypatch):
    harmless = {
        "case": "harmless",
        "tried": "rename nothing that the compiler rejects",
        "old": "mainnet_demo_nov11_invoice_06",
        "new": "mainnet_demo_nov11_invoice_07",
        "code": "X3_NEAR_NONCE",
    }
    monkeypatch.setattr(seq, "EDITS", [*seq.EDITS, harmless])
    report = seq.evaluate(tmp_path)
    row = next(item for item in report["stages"]["compiler_rejection"]["cases"] if item["case"] == "harmless")
    assert row["compiled"] is True and row["pass"] is False
    assert report["ok"] is False
