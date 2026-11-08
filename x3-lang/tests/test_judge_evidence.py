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


def test_refund_signature_is_explicitly_unsupported_by_judge_gate(tmp_path):
    # This gate cannot bind an imported receipt to finalized refund evidence.
    # Accepting --refund-tx and silently ignoring it misleads an operator.
    path = tmp_path / "claimed-refund.json"
    path.write_text(json.dumps({"kind": "x3.near_intents.v1", "verified_outcome": "refunded"}))
    result = subprocess.run(
        [sys.executable, str(MODULE), "--skip-tests", "--receipt", str(path),
         "--refresh-chain", "--refund-tx", "5" * 88],
        cwd=ROOT, capture_output=True, text=True,
    )
    assert result.returncode == 2
    assert "not supported" in result.stderr.lower()


def test_require_live_rejects_unproven_claims_even_with_tests_skipped(tmp_path):
    path = tmp_path / "claimed-live.json"
    path.write_text(json.dumps({
        "kind": "x3.near_intents.v1",
        "verified_outcome": "settled",
        "evidence": {"deposit": {"verified": True}, "settlement": {"verified": True}},
    }))
    result = subprocess.run(
        [sys.executable, str(MODULE), "--skip-tests", "--receipt", str(path),
         "--refresh-chain", "--require-live"],
        cwd=ROOT, capture_output=True, text=True,
    )
    assert result.returncode == 5


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


def test_full_mode_covers_every_quick_test_and_gate():
    quick = {job["name"]: job for job in judge_evidence.plan_tests("quick")}
    full = {job["name"]: job for job in judge_evidence.plan_tests("full")}
    assert quick.keys() < full.keys()
    for name, job in quick.items():
        assert job["argv"] and job["cwd"] == full[name]["cwd"]
        if name == "python_policy_and_chain_evidence_tests":
            full_paths = [ROOT / path for path in full[name]["argv"][4:]]
            for path in job["argv"][4:]:
                test_file = ROOT / path
                assert test_file.is_file()
                assert any(
                    selected == test_file
                    or (selected.is_dir() and selected in test_file.parents)
                    for selected in full_paths
                ), f"full mode omits quick Python test: {path}"
        elif name == "browser_signature_and_guard_tests":
            assert job["argv"][:2] == full[name]["argv"][:2]
            assert set(job["argv"][2:]) <= set(full[name]["argv"][2:])
        else:
            assert job == full[name]
    assert any("wasm32-unknown-unknown" in job["argv"] for job in full.values())


def test_full_browser_mode_discovers_all_test_files_without_shell_expansion(tmp_path, monkeypatch):
    browser_dir = Path("x3-lang/near/demo/test")
    test_dir = tmp_path / browser_dir
    test_dir.mkdir(parents=True)
    names = ["x3guard.test.mjs", "judge.test.mjs", "additional [suite].test.mjs"]
    for name in [*names, "helper.mjs"]:
        (test_dir / name).touch()
    monkeypatch.setattr(judge_evidence, "ROOT", tmp_path)
    monkeypatch.setattr(judge_evidence, "LANG", tmp_path / "x3-lang")
    full = judge_evidence.plan_tests("full")
    browser = next(job for job in full if job["name"] == "browser_signature_and_guard_tests")
    assert browser["argv"][1:] == ["--test", *(str(browser_dir / name) for name in sorted(names))]
    assert browser["cwd"] == str(tmp_path)


@pytest.mark.parametrize("mode", ["quick", "full"])
def test_node_override_propagates_to_browser_gate(monkeypatch, mode):
    monkeypatch.setenv("NODE", "/opt/node-24/bin/node")
    jobs = judge_evidence.plan_tests(mode)
    browser = next(job for job in jobs if job["name"] == "browser_signature_and_guard_tests")
    assert browser["argv"][0] == "/opt/node-24/bin/node"


def test_judge_tests_do_not_inherit_unrecorded_rustflags(monkeypatch):
    import subprocess
    monkeypatch.setenv("RUSTFLAGS", "-C target-cpu=native")
    captured = []

    def test_process(argv, *, env, **_kwargs):
        captured.append(dict(env))
        return subprocess.CompletedProcess(argv, 0, stdout="pass")

    monkeypatch.setattr(judge_evidence, "plan_tests", lambda mode: [{
        "name": "rust_vm_tests", "argv": ["cargo", "test"], "cwd": ROOT, "timeout": 10
    }])
    monkeypatch.setattr(judge_evidence.subprocess, "run", test_process)
    outcomes = judge_evidence.run_tests("full")
    assert outcomes[0]["status"] == "passed"
    assert captured and all("RUSTFLAGS" not in env for env in captured)


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


def test_signed_live_quote_cannot_trust_unsigned_receipt_fields(tmp_path, monkeypatch):
    import near_intents as near
    captured = json.loads((ROOT / 'x3-lang/tests/fixtures/near_intents_quote_dry_live_capture.json').read_text())
    captured['quoteRequest']['dry'] = False
    monkeypatch.setattr(near, 'check_oneclick_quote_signature', lambda _: (True, 'test-only'))
    def no_chain_call(*args, **kwargs):
        raise AssertionError('must not verify chain against untrusted receipt leg/baseline')
    monkeypatch.setattr(near, 'verify_deposit', no_chain_call)
    monkeypatch.setattr(near, 'verify_settlement', no_chain_call)
    receipt = {
        'kind': 'x3.near_intents.v1',
        'quote_response': captured,
        'quote_request': captured['quoteRequest'],
        'signed_payload': near.signed_quote_payload(captured),
        'quote_hash': near.oneclick_quote_hash(captured),
        'quote_response_sha256': near.chain_evidence.sha256_json(captured),
        'leg': {'recipient': 'attacker.near'},
        'tokens': {'USDC': 'attacker-token'},
        'evidence': {'settlement_baseline': '0'},
        'verified_outcome': 'settled',
    }
    path = tmp_path / 'forged-local-fields.json'
    path.write_text(json.dumps(receipt))
    r = judge_evidence.inspect_receipt(path, refresh_chain=True)
    assert r['quote_signature_valid'] and r['receipt_binding_valid'] and r['live_quote']
    assert r['status'] == 'unverified_receipt_chain_binding'
    assert not r['delivery_independently_rechecked']
    assert not r['deposit_independently_rechecked']
    assert not r['cross_chain_causality_proven']


def test_quick_mode_includes_model_and_fee_tests():
    steps = judge_evidence.plan_tests('quick')
    python_argv = next(step['argv'] for step in steps if 'pytest' in step['argv'])
    assert 'x3-lang/tests/test_near_agent.py' in python_argv
    assert 'x3-lang/tests/test_value_fee.py' in python_argv


def test_malformed_nested_receipt_fails_closed(tmp_path):
    deeply_nested = ('[' * 2000) + '0' + (']' * 2000)
    p = tmp_path / 'deep.json'
    p.write_text(deeply_nested)
    report = judge_evidence.inspect_receipt(p, refresh_chain=True)
    assert report['status'] == 'unverified'


def test_oversized_receipt_rejected_without_unbounded_read(tmp_path):
    p = tmp_path / 'large-receipt.json'
    p.write_bytes(b'x' * 5_000_002)
    with pytest.raises(ValueError, match='safe size'):
        judge_evidence.inspect_receipt(p, refresh_chain=True)


def test_judge_subprocess_environment_strips_wallet_secrets(monkeypatch):
    monkeypatch.setenv('NEAR_INTENTS_JWT', 'do-not-copy')
    monkeypatch.setenv('X3_AGENT_API_KEY', 'do-not-copy')
    observed = []
    class Completed:
        returncode = 0
        stdout = 'ok'
    def recording_run(argv, **kwargs):
        observed.append(kwargs['env'])
        return Completed()
    monkeypatch.setattr(judge_evidence, 'plan_tests', lambda _: [{'name':'test', 'argv':['echo','ok'],
        'cwd':ROOT, 'timeout':2}])
    monkeypatch.setattr(judge_evidence.subprocess, 'run', recording_run)
    checks = judge_evidence.run_tests('quick')
    assert checks[0]['status'] == 'passed'
    assert 'NEAR_INTENTS_JWT' not in observed[0]
    assert 'X3_AGENT_API_KEY' not in observed[0]
