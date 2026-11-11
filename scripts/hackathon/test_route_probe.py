"""Tests for the dry-quote route probe, using a real archived 1Click signed dry quote.

The archived quote (Solana USDC -> NEAR USDC, 25 USDC, 50 bps) was signed by the 1Click
manager key on 2026-11-06. A fake client serves it for the matching request and answers
"No liquidity available" for other amounts, the way 1Click did on 2026-11-10.
"""
from __future__ import annotations

import argparse
import copy
import json
import stat
from datetime import datetime, timezone
from pathlib import Path

import pytest

import route_probe as rp

ROOT = Path(__file__).resolve().parents[2]
ARCHIVE = json.loads((ROOT / "evidence/near-guard-testnet/quote-dry-official.json").read_text())
SIGNED = {k: ARCHIVE[k] for k in ("quote", "quoteRequest", "signature", "timestamp")}
TOKENS = json.loads((ROOT / "x3-lang/tests/fixtures/near_intents_tokens.json").read_text())
BASE_USDC = {"assetId": "nep141:base-0x833589fcd6edb6e08f4c7c32d4f71b54bda02913.omft.near", "decimals": 6,
             "blockchain": "base", "symbol": "USDC", "contractAddress": "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913"}
SOLANA = SIGNED["quoteRequest"]["refundTo"]
NEAR = SIGNED["quoteRequest"]["recipient"]
BASE = "0x" + "ab" * 20
# The archived request's deadline is exactly 10 minutes after this instant.
NOW = datetime(2026, 11, 6, 22, 34, 26, 89000, tzinfo=timezone.utc)


class FakeClient:
    base_url = "https://1click.chaindefuser.com"
    authenticated = False

    def __init__(self, tokens=None, response=None):
        self._tokens = TOKENS + [BASE_USDC] if tokens is None else tokens
        self._response = SIGNED if response is None else response
        self.requests = []

    def tokens(self):
        return copy.deepcopy(self._tokens)

    def quote(self, request):
        self.requests.append(request)
        echo = self._response["quoteRequest"]
        if all(request[k] == echo[k] for k in rp.ni._ECHO_FIELDS):
            return copy.deepcopy(self._response)
        raise rp.ni.NearIntentsError("X3_NEAR_HTTP", "POST /v0/quote returned 400: No liquidity available",
                                     "/v0/quote", {"status": 400})


def args(**over):
    base = dict(near_account=NEAR, solana_address=SOLANA, base_address=None, amounts="1,25,50", routes="all",
                slippage_bps=50, fee_cap_bps=100, delay=0, deadline_minutes=10)
    base.update(over)
    return argparse.Namespace(**base)


def run(client=None, **over):
    return rp.run(args(**over), client or FakeClient(), now_fn=lambda: NOW, sleep=lambda s: None)


def quotes(report, route):
    return {q["amount_usdc"]: q for q in next(r for r in report["routes"] if r["route"] == route)["quotes"]}


def test_real_signed_quote_is_usable_and_missing_liquidity_is_reported():
    report = run()
    sol = quotes(report, "sol-near")
    assert sol["25.000000"]["verdict"] == "USABLE"
    assert sol["25.000000"]["signature_ok"] is True
    assert sol["25.000000"]["refund_fee_usdc"] == "0.239292"
    assert sol["25.000000"]["refund_fee_bps"] == "95.72"
    assert sol["1.000000"]["verdict"] == "NO_QUOTE" and sol["1.000000"]["http_status"] == 400
    assert "No liquidity available" in sol["50.000000"]["provider_message"]
    assert report["usable"] == [{"route": "sol-near", "amount_usdc": "25.000000"}]
    skipped = {r["route"]: r.get("skipped") for r in report["routes"]}
    assert skipped["base-near"] == skipped["near-base"] == "no --base-address given"
    assert report["mode"] == "dry_quotes_only_nothing_signed_nothing_deposited"


def test_every_request_is_dry_and_exact_input():
    client = FakeClient()
    run(client, base_address=BASE)
    assert len(client.requests) == 12
    for request in client.requests:
        assert request["dry"] is True and request["swapType"] == "EXACT_INPUT"
        assert request["depositType"] == "ORIGIN_CHAIN" and request["recipientType"] == "DESTINATION_CHAIN"
    near_base = [r for r in client.requests if r["destinationAsset"] == BASE_USDC["assetId"]]
    assert {r["recipient"] for r in near_base} == {BASE} and {r["refundTo"] for r in near_base} == {NEAR}


def test_tampered_quote_is_refused_on_signature():
    tampered = copy.deepcopy(SIGNED)
    tampered["quote"]["amountOut"] = str(int(tampered["quote"]["amountOut"]) + 1)
    report = run(FakeClient(response=tampered))
    q = quotes(report, "sol-near")["25.000000"]
    assert q["verdict"] == "REFUSED: quote_signature" and q["signature_ok"] is False
    assert report["usable"] == []


def test_refund_fee_over_the_cap_is_refused():
    q = quotes(run(fee_cap_bps=90), "sol-near")["25.000000"]
    assert q["verdict"] == "REFUSED: refund_fee"


def test_slippage_wider_than_asked_is_refused():
    tampered = copy.deepcopy(SIGNED)
    tampered["quoteRequest"]["slippageTolerance"] = 40
    report = run(FakeClient(response=tampered), slippage_bps=40)
    assert quotes(report, "sol-near")["25.000000"]["verdict"].startswith("REFUSED: quote_signature")


def test_token_matched_by_contract_not_symbol():
    impostor = dict(BASE_USDC, contractAddress="0x" + "11" * 20)
    report = run(FakeClient(tokens=TOKENS + [impostor]), base_address=BASE)
    skipped = {r["route"]: r.get("skipped") for r in report["routes"]}
    assert "0 tokens for Base USDC" in skipped["base-near"]
    twice = run(FakeClient(tokens=TOKENS + [BASE_USDC, dict(BASE_USDC)]), base_address=BASE)
    assert "2 tokens for Base USDC" in {r["route"]: r.get("skipped") for r in twice["routes"]}["near-base"]
    wrong_decimals = run(FakeClient(tokens=TOKENS + [dict(BASE_USDC, decimals=18)]), base_address=BASE)
    assert "not 6-decimal USDC" in {r["route"]: r.get("skipped") for r in wrong_decimals["routes"]}["base-near"]


@pytest.mark.parametrize("field,value,message", [
    ("near_account", "x3lang.testnet", "NEAR mainnet"),
    ("near_account", "Bad.near", "NEAR mainnet"),
    ("solana_address", "0xabc", "Solana"),
    ("solana_address", "1111", "Solana"),
    ("base_address", "0x123", "Base"),
    ("base_address", "0x" + "0" * 40, "zero address"),
    ("amounts", "0", "positive"),
    ("amounts", "1.0000001", "positive"),
    ("routes", "sol-eth", "unknown route"),
    ("slippage_bps", 0, "between 1 and 500"),
    ("deadline_minutes", 5, "10 to 1440"),
])
def test_bad_input_is_refused(field, value, message):
    with pytest.raises(rp.ProbeError, match=message):
        run(**{field: value})


@pytest.mark.parametrize("api_key,jwt,expected", [
    ("synthetic-api-key", "", {"X-API-Key": "synthetic-api-key"}),
    ("synthetic-api-key", "synthetic-jwt", {"X-API-Key": "synthetic-api-key"}),
    ("", "synthetic-jwt", {"Authorization": "Bearer synthetic-jwt"}),
    ("", "", {}),
])
def test_client_authentication_matches_headers_and_report(monkeypatch, api_key, jwt, expected):
    seen = {}

    def inner(method, url, headers, body, timeout):
        seen.update(headers)
        return 200, b"[]"

    monkeypatch.setenv("NEAR_INTENTS_API_KEY", api_key)
    monkeypatch.setenv("NEAR_INTENTS_JWT", jwt)
    client = rp.make_client()
    client.transport = inner
    assert client.tokens() == []
    assert {k: v for k, v in seen.items() if k in ("X-API-Key", "Authorization")} == expected
    assert client.authenticated is bool(expected)
    report = rp.run(args(routes="sol-near"), client, now_fn=lambda: NOW, sleep=lambda s: None)
    assert report["authenticated"] is bool(expected)
    assert "synthetic-api-key" not in json.dumps(report)
    assert "synthetic-jwt" not in json.dumps(report)
    assert "synthetic-api-key" not in rp.render(report)
    assert "synthetic-jwt" not in rp.render(report)


def test_report_is_private_and_never_overwritten(tmp_path, monkeypatch, capsys):
    monkeypatch.setattr(rp, "make_client", lambda: FakeClient())
    monkeypatch.setattr(rp.ni, "verify_quote", _verify_at(NOW))
    out = tmp_path / "probe.json"
    argv = ["--near-account", NEAR, "--solana-address", SOLANA, "--routes", "sol-near", "--amounts", "25",
            "--slippage-bps", "50", "--deadline-minutes", "10", "--delay", "0", "--out", str(out)]
    monkeypatch.setattr(rp, "datetime", _FrozenDatetime)
    assert rp.main(argv) == 0
    printed = capsys.readouterr().out
    assert "USABLE at this moment: sol-near @ 25.000000 USDC" in printed
    assert "nothing was signed or deposited" in printed
    assert stat.S_IMODE(out.stat().st_mode) == 0o600
    assert json.loads(out.read_text())["routes"][0]["quotes"][0]["response"]["signature"] == SIGNED["signature"]
    with pytest.raises(FileExistsError):
        rp.main(argv)


def test_exit_code_is_one_when_nothing_is_usable(tmp_path, monkeypatch):
    monkeypatch.setattr(rp, "make_client", lambda: FakeClient())
    argv = ["--near-account", NEAR, "--solana-address", SOLANA, "--routes", "near-sol", "--amounts", "1",
            "--delay", "0", "--out", str(tmp_path / "p.json")]
    assert rp.main(argv) == 1


def test_bad_arguments_exit_two(tmp_path, capsys):
    assert rp.main(["--near-account", "nope", "--solana-address", SOLANA, "--out", str(tmp_path / "x.json")]) == 2
    assert "route probe stopped" in capsys.readouterr().err


class _FrozenDatetime(datetime):
    @classmethod
    def now(cls, tz=None):
        return NOW if tz else NOW.replace(tzinfo=None)


def _verify_at(now):
    real = rp.ni.verify_quote

    def verify(leg, request, response, *, now=None, manager_public_key=None):
        return real(leg, request, response, now=NOW, manager_public_key=manager_public_key)
    return verify
