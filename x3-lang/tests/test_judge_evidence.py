"""Judge evidence gate must fail closed and never treat a receipt as chain proof."""
import importlib.util
import json
import subprocess
import sys
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]
MODULE = ROOT / "scripts" / "judge_evidence.py"
spec = importlib.util.spec_from_file_location("judge_evidence", MODULE)
judge_evidence = importlib.util.module_from_spec(spec)
spec.loader.exec_module(judge_evidence)


def test_no_receipt_is_no_onchain_evidence():
    status = judge_evidence.inspect_receipt(None, refresh_chain=False)
    assert status["status"] == "not_supplied"
    assert status["cross_chain_causality_proven"] is False


def test_forged_reported_settlement_never_counts_as_proof(tmp_path):
    bad = {
        "kind": "x3.near_intents.v1",
        "receipt_version": 1,
        "verified_outcome": "settled",
        "verification": {"status": "verified", "outcome": "settled"},
        "evidence": {
            "deposit": {"verified": True, "checks": [{"ok": True}]},
            "settlement": {"verified": True, "checks": [{"ok": True}]},
        },
        "quote_request": {"dry": False},
        "quote_response": {"signature": "ed25519:FORGED", "quote": {"minAmountOut": "1"}},
    }
    path = tmp_path / "fake.json"
    path.write_text(json.dumps(bad))
    report = judge_evidence.inspect_receipt(path, refresh_chain=False)
    assert report["status"] == "unverified"
    assert report["quote_signature_valid"] is False
    assert report["deposit_independently_rechecked"] is False
    assert report["delivery_independently_rechecked"] is False
    assert report["cross_chain_causality_proven"] is False
    assert report["claimed_outcome"] == "settled"


def test_tampered_signed_hash_is_refused(tmp_path):
    from near_intents import oneclick_quote_hash
    fixture = ROOT / "x3-lang" / "tests" / "fixtures" / "near_intents_quote_dry_live_capture.json"
    captured = json.loads(fixture.read_text())
    assert "quote" in captured and "signature" in captured
    forged = {
        "kind": "x3.near_intents.v1",
        "quote_response": captured,
        "quote_hash": oneclick_quote_hash(captured),
        "quote_response_sha256": "a" * 64,
        "quote_request": captured["quoteRequest"],
        "verified_outcome": "settled",
    }
    path = tmp_path / "tampered.json"
    path.write_text(json.dumps(forged))
    report = judge_evidence.inspect_receipt(path, refresh_chain=False)
    assert report["status"] == "unverified"
    assert report["receipt_binding_valid"] is False
    assert report["cross_chain_causality_proven"] is False


def test_refresh_requires_receipt():
    result = subprocess.run(
        [sys.executable, str(MODULE), "--refresh-chain", "--skip-tests"],
        cwd=ROOT, capture_output=True, text=True,
    )
    assert result.returncode == 2
    assert "receipt" in result.stderr.lower()


def test_evidence_json_does_not_copy_keys_or_private_quote_fields(tmp_path):
    secret = "NEAR_INTENTS_JWT_SUPER_SECRET_TEST_ONLY"
    receipt = tmp_path / "receipt.json"
    receipt.write_text(json.dumps({
        "kind": "x3.near_intents.v1",
        "verified_outcome": "settled",
        "quote_response": {"quote": {"depositAddress": secret}},
        "my_secrets": {"api_key": secret},
    }))
    out = tmp_path / "report"
    result = subprocess.run(
        [sys.executable, str(MODULE), "--skip-tests", "--receipt", str(receipt),
         "--report-dir", str(out)],
        cwd=ROOT, capture_output=True, text=True,
    )
    assert result.returncode == 0, result.stderr
    artifact = (out / "evidence.json").read_text()
    guide = (out / "evidence.md").read_text()
    assert secret not in artifact and secret not in guide
    assert "unverified" in artifact
    assert "cross_chain_causality_proven" in artifact
    assert '"status": "verified"' not in artifact


def test_receipt_local_flags_cannot_grant_verified_outcome(tmp_path):
    data = {"kind": "x3.near_intents.v1", "verified_outcome": "refunded",
            "evidence": {"deposit": {"verified": True}, "refund": {"verified": True}}}
    path = tmp_path / "receipt.json"
    path.write_text(json.dumps(data))
    r = judge_evidence.inspect_receipt(path, refresh_chain=False)
    assert r["status"] == "unverified"
    assert r["cross_chain_causality_proven"] is False


def test_quick_and_full_modes_are_explicit_and_disjoint():
    quick = judge_evidence.plan_tests("quick")
    full = judge_evidence.plan_tests("full")
    assert all(x["argv"] and x["cwd"] for x in quick)
    assert any("pytest" in " ".join(x["argv"]) for x in quick)
    assert any("cargo" in " ".join(x["argv"]) for x in quick)
    assert len(full) > len(quick)
    assert any("wasm32-unknown-unknown" in " ".join(x["argv"]) for x in full)


def test_missing_receipt_is_not_inferred_from_optional_status():
    p = judge_evidence.inspect_receipt(None, refresh_chain=False)
    assert "settled" not in p["status"]


def test_wrong_receipt_request_shape_does_not_crash(tmp_path):
    payload = {"kind": "x3.near_intents.v1", "quote_request": ["not-a-dict"],
               "quote_response": {"quote": {}, "signature": "fake"},
               "verified_outcome": "settled"}
    p = tmp_path / "bad-request.json"
    p.write_text(json.dumps(payload))
    result = judge_evidence.inspect_receipt(p, refresh_chain=False)
    assert result["status"] == "unverified"
    assert result["cross_chain_causality_proven"] is False


def test_genuine_1click_signature_is_not_a_live_payment(tmp_path):
    import near_intents as near
    captured = json.loads((ROOT / 'x3-lang/tests/fixtures/near_intents_quote_dry_live_capture.json').read_text())
    assert near.check_oneclick_quote_signature(captured)[0]
    receipt = {
        'kind': 'x3.near_intents.v1',
        'quote_response': captured,
        'quote_request': captured['quoteRequest'],
        'signed_payload': near.signed_quote_payload(captured),
        'quote_response_sha256': near.chain_evidence.sha256_json(captured),
        'quote_hash': near.oneclick_quote_hash(captured),
        'verified_outcome': 'settled',  # hostile/self-reported, not an actual payment
    }
    path = tmp_path / 'signed-dry.json'
    path.write_text(json.dumps(receipt))
    observed = judge_evidence.inspect_receipt(path, refresh_chain=False)
    assert observed['quote_signature_valid'] is True
    assert observed['receipt_binding_valid'] is True
    assert observed['live_quote'] is False
    assert observed['deposit_independently_rechecked'] is False
    assert observed['status'] == 'unverified'
    assert observed['cross_chain_causality_proven'] is False


def test_attacker_cannot_relabel_a_signed_dry_quote_as_live(tmp_path):
    """The local receipt must not promote a dry signature into a live deposit instruction."""
    import near_intents as near
    response = json.loads((ROOT / 'x3-lang/tests/fixtures/near_intents_quote_dry_live_capture.json').read_text())
    assert response['quoteRequest']['dry'] is True
    forged_request = dict(response['quoteRequest'], dry=False)
    forged_receipt = {
        'kind': 'x3.near_intents.v1',
        'quote_request': forged_request,
        'quote_response': response,
        'signed_payload': near.signed_quote_payload(response),
        'quote_hash': near.oneclick_quote_hash(response),
        'quote_response_sha256': near.chain_evidence.sha256_json(response),
        'verified_outcome': 'settled',
    }
    path = tmp_path / 'dry-to-live.json'
    path.write_text(json.dumps(forged_receipt))
    classified = judge_evidence.inspect_receipt(path, refresh_chain=False)
    assert classified['quote_signature_valid'] is True
    assert classified['receipt_binding_valid'] is False
    assert classified['live_quote'] is False
    assert classified['cross_chain_causality_proven'] is False
