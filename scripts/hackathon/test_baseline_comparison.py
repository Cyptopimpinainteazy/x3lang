"""`baseline_comparison.py` is judge-facing: pin that it runs on the real on-chain bytes, that both
baselines really accept both quotes, and that it fails when the archive or the logic drifts."""
import importlib.util
import json
import shutil
from pathlib import Path

import pytest

SCRIPT = Path(__file__).resolve().parent / "baseline_comparison.py"


def _load():
    spec = importlib.util.spec_from_file_location("x3_baseline_comparison", SCRIPT)
    module = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    spec.loader.exec_module(module)
    return module


bc = _load()


@pytest.fixture
def evidence(tmp_path):
    copy = tmp_path / "near-guard-testnet"
    shutil.copytree(bc.EVIDENCE, copy)
    return copy


def test_only_the_x3_term_check_separates_the_two_real_quotes():
    report = bc.evaluate()
    assert report["ok"] is True
    assert report["baselines_accept_both"] is True and report["function_call_scope_identical"] is True
    rows = {r["case"]: r for r in report["cases"]}
    assert rows["allowed"]["x3_guard_terms"]["allows"] is True and rows["allowed"]["onchain_accept_allowed"] is True
    forbidden = rows["forbidden"]
    assert forbidden["quote_recipient"] == "wrap.near"
    assert forbidden["x3_guard_terms"]["allows"] is False and forbidden["onchain_accept_allowed"] is False
    assert forbidden["x3_guard_terms"]["mismatched"] == ["recipient"]
    assert "quote_request_echo" in forbidden["archived_view_failed_guards"]
    assert forbidden["onchain"]["tx_hash"].startswith("3Q7qTZqi")
    assert rows["allowed"]["onchain"]["tx_hash"].startswith("6B6k5kre")


def test_the_tampered_quote_is_refused_by_the_signature_check():
    tampered = bc.evaluate()["tampered"]
    assert tampered["signature_check"]["allows"] is False
    assert "quote_signature" in tampered["x3_guard_terms"]["reasons"]


def test_an_archived_quote_that_was_not_the_one_sent_on_chain_stops_the_run(evidence):
    path = evidence / "quote-live-official.json"
    quote = json.loads(path.read_text())
    quote["quote"]["amountOut"] = str(int(quote["quote"]["amountOut"]) + 1)
    path.write_text(json.dumps(quote))
    with pytest.raises(bc.ArchiveMismatch, match="live_accept"):
        bc.evaluate(evidence)


def test_a_policy_that_already_matched_the_recipient_fails_the_claim(evidence, monkeypatch):
    """If the recipient were not the difference, the comparison must not claim it was."""
    real = bc.load_archive

    def same_recipient(path):
        archive = real(path)
        archive["policies"]["evidence-prohibited-01"] = {**archive["policies"]["evidence-prohibited-01"],
                                                         "recipient": "wrap.near"}
        return archive

    monkeypatch.setattr(bc, "load_archive", same_recipient)
    report = bc.evaluate(evidence)
    forbidden = next(r for r in report["cases"] if r["case"] == "forbidden")
    assert forbidden["x3_guard_terms"]["mismatched"] == []
    assert report["ok"] is False


def test_different_function_call_scopes_fail_the_baseline_claim(evidence):
    path = evidence / "transactions.json"
    data = json.loads(path.read_text())
    for tx in data["transactions"]:
        if tx["label"] == "prohibited_accept":
            tx["method_names"] = ["accept_quote_v2"]
    path.write_text(json.dumps(data))
    report = bc.evaluate(evidence)
    assert report["function_call_scope_identical"] is False
    assert report["ok"] is False


def test_an_onchain_outcome_that_disagrees_with_the_terms_fails(evidence):
    path = evidence / "transactions.json"
    data = json.loads(path.read_text())
    for tx in data["transactions"]:
        if tx["label"] == "prohibited_accept":
            tx["status"] = json.dumps({"SuccessValue": ""})
    path.write_text(json.dumps(data))
    report = bc.evaluate(evidence)
    assert next(r for r in report["cases"] if r["case"] == "forbidden")["pass"] is False
    assert report["ok"] is False


def test_cli_exit_code_and_wording(capsys):
    assert bc.main([]) == 0
    out = capsys.readouterr().out
    assert "Both baselines allow both real quotes" in out
    assert "independent reasons: the recipient differs from the policy, and it is a dry quote" in out
    assert "Result: PASS" in out
