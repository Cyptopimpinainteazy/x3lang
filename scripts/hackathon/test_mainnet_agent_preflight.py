"""Offline orchestration tests. Synthetic RPC/quote responses are never chain evidence."""
import copy
import importlib.util
import json
import os
from pathlib import Path

import pytest

SPEC = importlib.util.spec_from_file_location("mainnet_preflight", Path(__file__).with_name("mainnet_agent_preflight.py"))
p = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(p)
EXAMPLES = p.ROOT / "x3-lang" / "examples"
HASH = "A" * 44


def args(*extra):
    return p.parser().parse_args(["--report", "/tmp/unused-preflight.json", *extra])


def complete(*extra):
    return args("--program", str(EXAMPLES / "near_intents_agent_payment.x3"),
                "--mandate", str(EXAMPLES / "near_mandate_agent.x3"),
                "--max-input-usdc", "25", *extra)


class FixtureTransport:
    """Test-only remote responses; no network or real signatures."""
    def __init__(self, tokens=None, chain="mainnet", code_hash=HASH):
        self.calls = []
        self.tokens = copy.deepcopy(tokens if tokens is not None else list(p.PINS))
        self.chain = chain
        self.code_hash = code_hash

    def __call__(self, method, url, headers, body, timeout):
        payload = json.loads(body) if body else None
        self.calls.append((method, url, payload))
        if url == p.API + "/v0/tokens":
            result = self.tokens
        elif url == p.API + "/v0/quote":
            assert payload["dry"] is True
            result = {"quoteRequest": payload, "quote": {}, "signature": "synthetic-invalid"}
        elif url == p.RPC and payload["method"] == "status":
            result = {"result": {"chain_id": self.chain}}
        elif url == p.RPC and payload["method"] == "query":
            result = {"result": {"amount": "1", "code_hash": self.code_hash,
                                 "block_hash": HASH, "block_height": 123}}
        else:
            pytest.fail("Unexpected operation: " + url)
        return 200, json.dumps(result).encode()


def test_discovery_reports_every_missing_input_without_quotes():
    t = FixtureTransport()
    report = p.run(args(), transport=t)
    assert report["missing_inputs"] == ["--program", "--mandate", "--max-input-usdc",
                                         "--guard", "--expected-guard-code-hash"]
    assert report["checks"]["assets"]["ok"]
    assert report["eligible_for_wallet_authorization"] is False
    assert t.calls == [("GET", p.API + "/v0/tokens", None)]


@pytest.mark.parametrize("field,value", [("contractAddress", "wrong-mint"), ("decimals", 9),
                                         ("decimals", True), ("blockchain", "near")])
def test_wrong_sol_asset_metadata_blocks_quote(field, value):
    tokens = copy.deepcopy(list(p.PINS))
    tokens[0][field] = value
    report = p.run(complete(), transport=FixtureTransport(tokens))
    assert report["error"]["code"] == "PINNED_ASSET_METADATA_MISMATCH"


def test_ambiguous_or_missing_pin_is_rejected():
    with pytest.raises(p.PreflightError):
        p.pinned_tokens([p.PINS[0], p.PINS[0], p.PINS[1]])
    with pytest.raises(p.PreflightError):
        p.pinned_tokens([p.PINS[0]])


def test_over_budget_does_not_request_quote():
    a = complete()
    a.max_input_usdc = "24.999999"
    t = FixtureTransport()
    report = p.run(a, transport=t)
    assert report["error"]["code"] == "PROGRAM_EXCEEDS_OWNER_INPUT_CAP"
    assert not any(url.endswith("/quote") for _, url, _ in t.calls)


@pytest.mark.parametrize("value", ["0", "-1", "NaN", "Infinity", "1e2", "0.0000001"])
def test_budget_requires_positive_exact_decimal(value):
    with pytest.raises(p.PreflightError):
        p.budget_units(value)


def test_budget_base_units_are_exact():
    assert p.budget_units("25.000001") == 25000001


def test_guard_codehash_mismatch_stops_before_quote():
    t = FixtureTransport(code_hash="B" * 44)
    report = p.run(complete("--guard", "guard.near", "--expected-guard-code-hash", HASH), transport=t)
    assert report["error"]["code"] == "GUARD_CODE_HASH_MISMATCH"
    assert not report["checks"]["guard"]["matches_expected"]
    assert not any(url.endswith("/quote") for _, url, _ in t.calls)


def test_testnet_rpc_observation_stops():
    t = FixtureTransport(chain="testnet")
    report = p.run(complete(), transport=t)
    assert report["error"]["code"] == "NEAR_NETWORK_IS_NOT_MAINNET"
    assert not any(url.endswith("/quote") for _, url, _ in t.calls)


def test_invalid_signature_does_not_verify_preview():
    t = FixtureTransport()
    report = p.run(complete(), transport=t)
    assert "error" in report
    assert not report.get("dry_preview_verified", False)
    assert report["eligible_for_wallet_authorization"] is False


def test_successful_orchestration_is_still_not_wallet_authorization(monkeypatch):
    # Stub only signature/policy verifier return here; actual signature refusal tested above.
    monkeypatch.setattr(p.ni, "verify_quote", lambda *a, **kw: [{"guard": "fixture", "ok": True}])
    t = FixtureTransport()
    report = p.run(complete("--guard", "guard.near", "--expected-guard-code-hash", HASH), transport=t)
    assert report.get("error") is None, report
    assert report["dry_preview_verified"] is True
    assert report["eligible_for_wallet_authorization"] is False
    assert report["status"] == "blocked"
    quotes = [payload for _, url, payload in t.calls if url.endswith("/quote")]
    assert len(quotes) == 1 and quotes[0]["dry"] is True
    assert quotes[0]["originAsset"] == p.SOL_ASSET


@pytest.mark.parametrize("method,url,payload", [
    ("POST", p.API + "/v0/quote", {"dry": False}),
    ("POST", p.API + "/v0/quote", {"dry": 1}),
    ("POST", p.API + "/v0/deposit/submit", {"txHash": "anything"}),
    ("POST", p.RPC, {"method": "broadcast_tx_commit", "params": ["anything"]}),
    ("POST", "https://rpc.testnet.near.org", {"method": "status", "params": []}),
    ("GET", "https://attacker.invalid/v0/tokens", None),
])
def test_transport_refuses_payment_or_unpinned_endpoints(method, url, payload):
    with pytest.raises(p.PreflightError, match="NETWORK_OPERATION_REFUSED"):
        p.readonly_transport(method, url, {}, json.dumps(payload).encode() if payload else None, 1)


def test_environment_cannot_change_network_endpoints(monkeypatch):
    monkeypatch.setenv("NEAR_INTENTS_BASE_URL", "https://attacker.invalid")
    monkeypatch.setenv("NEAR_RPC_URL", "https://rpc.testnet.near.org")
    t = FixtureTransport()
    p.run(complete(), transport=t)
    assert all(url.startswith(p.API + "/") or url == p.RPC for _, url, _ in t.calls)


def test_report_private_and_exclusive(tmp_path):
    target = tmp_path / "report.json"
    p.save_report(target, {"eligible_for_wallet_authorization": False})
    assert os.stat(target).st_mode & 0o777 == 0o600
    with pytest.raises(FileExistsError):
        p.save_report(target, {"changed": True})


def test_report_refuses_repository_or_symlink(tmp_path):
    with pytest.raises(p.PreflightError, match="OUTSIDE_REPOSITORY"):
        p.save_report(p.ROOT / "do-not-create.json", {})
    link = tmp_path / "link.json"
    link.symlink_to(tmp_path / "other.json")
    with pytest.raises(FileExistsError):
        p.save_report(link, {})
