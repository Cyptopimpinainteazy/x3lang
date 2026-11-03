"""NEAR Intents adapter: lowering, guard enforcement, replay protection and status tracking.

The 1Click API is reached through `OneClickClient`'s injectable transport; these tests supply a
recording transport so every request the adapter would send is asserted on, and nothing here
touches the network.
"""
import importlib.util
import json
import subprocess
import sys
from datetime import datetime, timedelta, timezone
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]
EXAMPLE = ROOT / "examples" / "near_intents_sol_to_near.x3"
TOKENS = ROOT / "tests" / "fixtures" / "near_intents_tokens.json"
NOW = datetime(2026, 11, 3, 3, 0, 0, tzinfo=timezone.utc)
SOL_USDC = "nep141:sol-5ce3bf3a31af18be40ba30f721101b4341690186.omft.near"
NEAR_USDC = "nep141:17208628f84f5d6ad33f0da3bbbeb27ffcb398eac501a31bd6ad2011e36133a1"
SOL_ADDR = "4Nd1mzi8Y1QYxJt9wZWBYZpG7S4pYkZs6YzD3Vt9aBcD"


def _load(name, path):
    sys.path.insert(0, str(path.parent))
    try:
        spec = importlib.util.spec_from_file_location(name, str(path))
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
    finally:
        sys.path.pop(0)
    return module


ni = _load("near_intents_under_test", ROOT / "near_intents.py")


class RecordingTransport:
    """Test double for the 1Click HTTP API: answers from a handler, records every call."""

    def __init__(self, handler):
        self.handler = handler
        self.calls = []

    def __call__(self, method, url, headers, body, timeout):
        payload = json.loads(body) if body else None
        self.calls.append({"method": method, "url": url, "headers": headers, "body": payload})
        status, data = self.handler(method, url, payload)
        return status, json.dumps(data).encode() if data is not None else b""


def good_quote(request, *, amount_out=24_950_000, min_out=None, usd_out="24.94", deadline=None, time_estimate=20):
    min_out = min_out if min_out is not None else amount_out * (10_000 - request["slippageTolerance"]) // 10_000
    quote = {
        "amountIn": request["amount"],
        "amountInFormatted": "25.0",
        "amountInUsd": "24.995",
        "minAmountIn": request["amount"],
        "amountOut": str(amount_out),
        "amountOutFormatted": str(amount_out / 1e6),
        "amountOutUsd": usd_out,
        "minAmountOut": str(min_out),
        "timeEstimate": time_estimate,
        "refundFee": "244425",
    }
    if not request["dry"]:
        quote.update({
            "depositAddress": "9xQeWvG816bUx9EPjHmaT23yvVM2ZWbrrpZb9PusVFin",
            "deadline": deadline or request["deadline"],
            "timeWhenInactive": request["deadline"],
        })
    return {
        "correlationId": "c0ffee",
        "timestamp": "2026-11-03T03:00:00.000Z",
        "signature": "ed25519:3Kz",
        "quoteRequest": dict(request),
        "quote": quote,
    }


def api(quote_fn=good_quote, statuses=None):
    statuses = list(statuses or [])

    def handler(method, url, payload):
        if url.endswith("/v0/tokens"):
            return 200, json.loads(TOKENS.read_text())
        if url.endswith("/v0/quote"):
            return 200, quote_fn(payload)
        if "/v0/status" in url:
            return 200, statuses.pop(0)
        if url.endswith("/v0/deposit/submit"):
            return 200, {"ok": True, "echo": payload}
        return 404, {"message": "not found"}

    return RecordingTransport(handler)


def near_rpc(missing=(), error=None):
    def handler(method, url, payload):
        if error:
            return 200, {"jsonrpc": "2.0", "id": payload["id"], "error": error}
        account = payload["params"]["account_id"]
        if account in missing:
            return 200, {"jsonrpc": "2.0", "id": payload["id"],
                         "error": {"name": "HANDLER_ERROR", "cause": {"name": "UNKNOWN_ACCOUNT", "info": {}}, "code": -32000}}
        return 200, {"jsonrpc": "2.0", "id": payload["id"], "result": {"amount": "1", "locked": "0", "code_hash": "11111111111111111111111111111111"}}
    transport = RecordingTransport(handler)
    return ni.NearRpc(url="https://rpc.example.invalid", transport=transport), transport


def program(tmp_path, *, route=None, requires=None, timeout="timeout 10m refund Solana.USDC", on_fail="on_fail rollback"):
    route = route or "bridge NearIntents Solana.USDC -> Near.USDC min_receive 24800000;"
    requires = requires if requires is not None else [
        "require slippage <= 0.5%",
        "require fees <= 100 bps",
        "require nonce unused n_001",
    ]
    body = "\n".join([
        "intent t {",
        f"    from Solana.USDC amount 25000000 receiver {SOL_ADDR}",
        "    to Near.USDC receiver x3-demo.near",
        "    route {",
        f"        {route}",
        "    }",
        *[f"    {r}" for r in requires],
        f"    {timeout}" if timeout else "",
        f"    {on_fail}" if on_fail else "",
        "}",
    ])
    path = tmp_path / "t.x3"
    path.write_text(body)
    return str(path)


# --- lowering ------------------------------------------------------------------------------

def test_example_lowers_to_the_1click_request_it_describes():
    planned = ni.plan(str(EXAMPLE), client=ni.OneClickClient(jwt="", transport=api()), tokens_path=str(TOKENS), now=NOW)
    req = planned["quote_request"]
    assert req == {
        "dry": True,
        "swapType": "EXACT_INPUT",
        "slippageTolerance": 50,
        "originAsset": SOL_USDC,
        "depositType": "ORIGIN_CHAIN",
        "destinationAsset": NEAR_USDC,
        "amount": "25000000",
        "refundTo": SOL_ADDR,
        "refundType": "ORIGIN_CHAIN",
        "recipient": "x3-demo.near",
        "recipientType": "DESTINATION_CHAIN",
        "deadline": "2026-11-03T03:10:00.000Z",
    }
    assert planned["leg"]["min_receive"] == 24_800_000
    assert planned["leg"]["fee_cap_bps"] == 100
    assert planned["leg"]["nonce"] == "near_intents_sol_to_near_001"


@pytest.mark.parametrize("bound,expected", [
    ("require slippage <= 0.5%", 50),
    ("require slippage < 0.5%", 49),
    ("require slippage <= 0.125%", 12),  # rounds toward the stricter side
    ("require slippage <= 0.003", 30),
])
def test_slippage_bound_becomes_whole_bps_never_looser(tmp_path, bound, expected):
    _, leg = ni.compile_program(program(tmp_path, requires=[bound]), now=NOW)
    assert leg["slippage_bps"] == expected


def test_tightest_of_several_slippage_guards_wins(tmp_path):
    _, leg = ni.compile_program(program(tmp_path, requires=["require slippage <= 1%", "require slippage <= 0.3%"]), now=NOW)
    assert leg["slippage_bps"] == 30


def test_explicit_refund_receiver_is_used(tmp_path):
    other = "So11111111111111111111111111111111111111112"
    _, leg = ni.compile_program(program(tmp_path, requires=["require slippage <= 0.5%"],
                                        timeout=f"timeout 90s refund Solana.USDC to {other}"), now=NOW)
    assert leg["refund_to"] == other
    assert leg["deadline"] == "2026-11-03T03:01:30.000Z"


@pytest.mark.parametrize("kwargs,code", [
    ({"route": "swap Raydium Solana.USDC -> Solana.SOL amount 25000000 min_output 1;\n        bridge NearIntents Solana.SOL -> Near.USDC;"}, "X3_NEAR_ROUTE_SHAPE"),
    ({"route": "bridge X3 Solana.USDC -> Near.USDC;"}, "X3_NEAR_ROUTE_SHAPE"),
    ({"route": "bridge NearIntents Solana.SOL -> Near.USDC;"}, "X3_NEAR_ROUTE_ENDPOINTS"),
    ({"requires": []}, "X3_NEAR_SLIPPAGE_REQUIRED"),
    ({"requires": ["require slippage >= 0.5%"]}, "X3_NEAR_GUARD_OPERATOR"),
    ({"requires": ["require slippage <= 30"]}, "X3_NEAR_BOUND_UNIT"),
    ({"requires": ["require slippage <= 0.5%", "require finality Solana >= 32"]}, "X3_NEAR_GUARD_UNENFORCEABLE"),
    ({"requires": ["require slippage <= 0.5%", "require proof_complete source_lock_proof"]}, "X3_NEAR_GUARD_UNENFORCEABLE"),
    ({"timeout": None}, "X3_NEAR_TIMEOUT_REQUIRED"),
    ({"timeout": "timeout 2blocks refund Solana.USDC"}, "X3_NEAR_TIMEOUT_UNIT"),
    ({"timeout": "timeout 10m refund Solana.SOL"}, "X3_NEAR_REFUND_ASSET"),
    ({"on_fail": "on_fail halt"}, "X3_NEAR_ON_FAIL_UNSUPPORTED"),
    ({"on_fail": "on_fail refund Solana.USDC to So11111111111111111111111111111111111111112"}, "X3_NEAR_REFUND_TO"),
])
def test_programs_the_adapter_cannot_honour_are_refused_by_code(tmp_path, kwargs, code):
    with pytest.raises(ni.NearIntentsError) as err:
        ni.compile_program(program(tmp_path, **kwargs), now=NOW)
    assert err.value.code == code


def test_fractional_amount_is_refused_not_rounded():
    intent = {
        "intent": "t",
        "from": {"chain": "solana", "asset": "USDC", "amount": "25.5", "receiver": SOL_ADDR},
        "to": {"chain": "near", "asset": "USDC", "receiver": "x3-demo.near"},
        "route": [{"type": "bridge", "via": "nearintents", "from_ref": {"chain": "solana", "asset": "USDC"},
                   "to_ref": {"chain": "near", "asset": "USDC"}}],
        "requires": [{"kind": "slippage", "op": "<=", "value": "0.5%"}],
        "policies": {"timeout": {"duration": "10m"}},
    }
    with pytest.raises(ni.NearIntentsError) as err:
        ni.lower(intent, now=NOW)
    assert err.value.code == "X3_NEAR_AMOUNT"


def test_invalid_near_receiver_fails_typecheck(tmp_path):
    path = program(tmp_path)
    Path(path).write_text(Path(path).read_text().replace("x3-demo.near", "Not_A.Near..Account"))
    with pytest.raises(ni.NearIntentsError) as err:
        ni.compile_program(path, now=NOW)
    assert err.value.code == "X3_NEAR_TYPECHECK"


def test_asset_resolution_refuses_missing_and_ambiguous_tokens():
    tokens = json.loads(TOKENS.read_text())
    with pytest.raises(ni.NearIntentsError) as err:
        ni.resolve_asset(tokens, "near", "USDT")
    assert err.value.code == "X3_NEAR_ASSET_UNSUPPORTED"
    dup = tokens + [{**tokens[0], "assetId": "nep141:sol-impostor.omft.near"}]
    with pytest.raises(ni.NearIntentsError) as err:
        ni.resolve_asset(dup, "solana", "USDC")
    assert err.value.code == "X3_NEAR_ASSET_AMBIGUOUS"
    assert ni.resolve_asset(tokens, "near", "NEAR")["assetId"] == "nep141:wrap.near"


# --- quote verification --------------------------------------------------------------------

def _leg_and_request(live=False):
    planned = ni.plan(str(EXAMPLE), client=ni.OneClickClient(jwt="", transport=api()), tokens_path=str(TOKENS), live=live, now=NOW)
    return planned["leg"], planned["quote_request"]


def test_a_quote_that_meets_every_guard_passes():
    leg, req = _leg_and_request(live=True)
    results = ni.verify_quote(leg, req, good_quote(req), now=NOW)
    assert {r["guard"] for r in results} == {
        "quote_request_echo", "amount_in", "min_receive", "slippage", "fees", "refund_fee",
        "deposit_address", "quote_signature", "deadline",
    }
    assert all(r["ok"] for r in results)


@pytest.mark.parametrize("mutate,guard", [
    (lambda q: q["quote"].update(minAmountOut="24700000", amountOut="24750000"), "min_receive"),
    (lambda q: q["quote"].update(minAmountOut="24850000", amountOut="25500000"), "slippage"),
    (lambda q: q["quote"].update(amountOutUsd="24.60"), "fees"),
    (lambda q: q["quote"].update(amountIn="20000000"), "amount_in"),
    (lambda q: q["quoteRequest"].update(recipient="attacker.near"), "quote_request_echo"),
    (lambda q: q["quote"].pop("depositAddress"), "deposit_address"),
    (lambda q: q.update(signature=""), "quote_signature"),
    (lambda q: q["quote"].update(timeEstimate=900), "deadline"),
    (lambda q: q["quote"].update(amountInUsd="0"), "fees"),
    (lambda q: q["quote"].update(refundFee="250001"), "refund_fee"),  # 100.0004 bps of 25 USDC
    (lambda q: q["quote"].pop("refundFee"), "refund_fee"),
])
def test_a_quote_breaking_a_guard_is_rejected_before_deposit(mutate, guard):
    leg, req = _leg_and_request(live=True)
    response = good_quote(req)
    mutate(response)
    with pytest.raises(ni.NearIntentsError) as err:
        ni.verify_quote(leg, req, response, now=NOW)
    assert err.value.code == "X3_NEAR_GUARD_FAILED"
    failed = {r["guard"] for r in err.value.detail if not r["ok"]}
    assert guard in failed


def test_echoed_deadline_with_different_precision_still_matches():
    leg, req = _leg_and_request()
    response = good_quote(req)
    response["quoteRequest"]["deadline"] = "2026-11-03T03:10:00Z"
    ni.verify_quote(leg, req, response, now=NOW)


# --- client --------------------------------------------------------------------------------

def test_client_sends_bearer_jwt_and_surfaces_http_errors():
    transport = RecordingTransport(lambda m, u, p: (401, {"message": "invalid token"}))
    client = ni.OneClickClient(base_url="https://example.invalid/", jwt="secret", transport=transport)
    with pytest.raises(ni.NearIntentsError) as err:
        client.quote({"dry": True})
    assert err.value.code == "X3_NEAR_HTTP"
    assert "invalid token" in err.value.message
    call = transport.calls[0]
    assert call["url"] == "https://example.invalid/v0/quote"
    assert call["headers"]["Authorization"] == "Bearer secret"


def test_client_without_jwt_sends_no_authorization_header():
    transport = api()
    ni.OneClickClient(jwt="", transport=transport).tokens()
    assert "Authorization" not in transport.calls[0]["headers"]


# --- end to end ----------------------------------------------------------------------------

def test_dry_quote_checks_guards_and_consumes_nothing(tmp_path):
    store = ni.NonceStore(str(tmp_path / "nonces.json"))
    transport = api()
    result = ni.quote(str(EXAMPLE), client=ni.OneClickClient(jwt="", transport=transport), live=False,
                      nonce_store=store, receipt_path=str(tmp_path / "r.json"), rpc=near_rpc()[0], now=NOW)
    assert result["status"] == "quoted"
    assert transport.calls[-1]["body"]["dry"] is True
    assert "note" in result  # unauthenticated fee schedule is called out
    assert not (tmp_path / "nonces.json").exists()
    assert not (tmp_path / "r.json").exists()


def test_live_quote_writes_receipt_consumes_nonce_and_refuses_replay(tmp_path):
    store = ni.NonceStore(str(tmp_path / "nonces.json"))
    receipt_path = tmp_path / "r.json"
    client = ni.OneClickClient(jwt="k", transport=api())
    result = ni.quote(str(EXAMPLE), client=client, live=True, nonce_store=store,
                      receipt_path=str(receipt_path), rpc=near_rpc()[0], now=NOW)
    assert result["status"] == "awaiting_deposit"
    assert result["deposit"]["address"] == "9xQeWvG816bUx9EPjHmaT23yvVM2ZWbrrpZb9PusVFin"
    assert result["deposit"]["amount"] == "25000000"

    receipt = json.loads(receipt_path.read_text())
    assert receipt["kind"] == "x3.near_intents.v1"
    assert len(receipt["program_sha256"]) == 64
    assert receipt["validated_intent_v1"]["schema_version"] == 1
    assert all(g["ok"] for g in receipt["guards"])
    assert "near_intents_sol_to_near_001" in json.loads((tmp_path / "nonces.json").read_text())["used"]

    transport = api()
    with pytest.raises(ni.NearIntentsError) as err:
        ni.quote(str(EXAMPLE), client=ni.OneClickClient(jwt="k", transport=transport), live=True,
                 nonce_store=store, receipt_path=str(receipt_path), rpc=near_rpc()[0], now=NOW)
    assert err.value.code == "X3_NEAR_NONCE_USED"
    assert not any(c["url"].endswith("/v0/quote") for c in transport.calls), "a used nonce must not request a quote"


def test_rejected_live_quote_does_not_consume_the_nonce(tmp_path):
    store = ni.NonceStore(str(tmp_path / "nonces.json"))
    bad = lambda req: good_quote(req, amount_out=24_700_000)  # below min_receive once slippage is applied
    with pytest.raises(ni.NearIntentsError) as err:
        ni.quote(str(EXAMPLE), client=ni.OneClickClient(jwt="k", transport=api(bad)), live=True,
                 nonce_store=store, receipt_path=str(tmp_path / "r.json"), rpc=near_rpc()[0], now=NOW)
    assert err.value.code == "X3_NEAR_GUARD_FAILED"
    assert not (tmp_path / "nonces.json").exists()
    assert not (tmp_path / "r.json").exists()


def _status(status, **details):
    return {"correlationId": "c", "quoteResponse": {}, "status": status, "updatedAt": "2026-11-03T03:01:00Z",
            "swapDetails": {"intentHashes": [], "nearTxHashes": ["n1"], "originChainTxHashes": [{"hash": "o1", "explorerUrl": "x"}],
                            "destinationChainTxHashes": [], **details}}


def test_status_waits_to_a_terminal_outcome_and_records_it(tmp_path):
    store = ni.NonceStore(str(tmp_path / "nonces.json"))
    receipt_path = tmp_path / "r.json"
    ni.quote(str(EXAMPLE), client=ni.OneClickClient(jwt="k", transport=api()), live=True,
             nonce_store=store, receipt_path=str(receipt_path), rpc=near_rpc()[0], now=NOW)

    transport = api(statuses=[_status("PENDING_DEPOSIT"), _status("PROCESSING"), _status("PROCESSING"),
                              _status("SUCCESS", amountOut="24950000")])
    sleeps = []
    observed = ni.track(ni.OneClickClient(jwt="k", transport=transport), "9xQe", wait=True, interval=1,
                        receipt_path=str(receipt_path), sleep=sleeps.append, clock=lambda: 0.0)
    assert observed["outcome"] == "settled"
    assert observed["amount_out"] == "24950000"
    assert len(sleeps) == 3
    receipt = json.loads(receipt_path.read_text())
    assert [s["status"] for s in receipt["status_history"]] == ["PENDING_DEPOSIT", "PROCESSING", "SUCCESS"]
    assert receipt["outcome"] == "settled"


def test_refund_and_unknown_statuses():
    client = ni.OneClickClient(jwt="", transport=api(statuses=[_status("REFUNDED", refundReason="deadline"), _status("WAT")]))
    observed = ni.track(client, "a")
    assert observed["outcome"] == "refunded" and observed["refund_reason"] == "deadline"
    with pytest.raises(ni.NearIntentsError) as err:
        ni.track(client, "a")
    assert err.value.code == "X3_NEAR_STATUS_UNKNOWN"


def test_cli_status_exit_codes_follow_the_outcome(capsys):
    for status, code in (("SUCCESS", 0), ("REFUNDED", 2), ("FAILED", 3), ("PROCESSING", 4)):
        client = ni.OneClickClient(jwt="", transport=api(statuses=[_status(status)]))
        assert ni.main(["status", "--deposit-address", "a"], client=client) == code
    capsys.readouterr()


def test_cli_plan_runs_offline_with_saved_tokens():
    proc = subprocess.run([sys.executable, str(ROOT / "near_intents.py"), "plan", str(EXAMPLE), "--tokens", str(TOKENS)],
                          capture_output=True, text=True)
    assert proc.returncode == 0, proc.stderr
    out = json.loads(proc.stdout)
    assert out["quote_request"]["originAsset"] == SOL_USDC
    assert out["quote_request"]["dry"] is True


def test_cli_reports_refusals_as_structured_errors(tmp_path):
    path = program(tmp_path, requires=[])
    proc = subprocess.run([sys.executable, str(ROOT / "near_intents.py"), "plan", path, "--tokens", str(TOKENS)],
                          capture_output=True, text=True)
    assert proc.returncode == 1
    assert json.loads(proc.stderr)["errors"][0]["code"] == "X3_NEAR_SLIPPAGE_REQUIRED"


# --- runner integration --------------------------------------------------------------------

def test_runner_names_the_near_intents_executor_instead_of_settling():
    dry = subprocess.run([sys.executable, str(ROOT / "runner.py"), str(EXAMPLE), "--dry-run", "--no-schema"],
                         capture_output=True, check=True)
    result = json.loads(dry.stdout)
    assert result["status"] == "ok"
    leg = result["emitted"]["emitted"][0]
    assert leg["settles_via"] == "near_intents_1click" and leg["proof_required"] is False

    prod = subprocess.run([sys.executable, str(ROOT / "runner.py"), str(EXAMPLE), "--no-schema"],
                          capture_output=True, check=True)
    result = json.loads(prod.stdout)
    assert result["status"] == "rolled_back"
    assert result["execution"][0]["code"] == "X3_NEAR_INTENTS_EXECUTOR"


# --- NEAR account preflight ----------------------------------------------------------------

def test_unknown_near_recipient_is_refused_before_any_quote_is_requested(tmp_path):
    rpc, rpc_calls = near_rpc(missing={"x3-demo.near"})
    transport = api()
    with pytest.raises(ni.NearIntentsError) as err:
        ni.quote(str(EXAMPLE), client=ni.OneClickClient(jwt="", transport=transport), live=False,
                 nonce_store=ni.NonceStore(str(tmp_path / "n.json")), rpc=rpc, now=NOW)
    assert err.value.code == "X3_NEAR_ACCOUNT_UNKNOWN"
    assert "x3-demo.near" in err.value.message
    assert rpc_calls.calls[0]["body"]["params"] == {"request_type": "view_account", "finality": "final", "account_id": "x3-demo.near"}
    assert not any(c["url"].endswith("/v0/quote") for c in transport.calls)


def test_existing_recipient_is_recorded_as_a_passed_guard(tmp_path):
    result = ni.quote(str(EXAMPLE), client=ni.OneClickClient(jwt="", transport=api()), live=False,
                      nonce_store=ni.NonceStore(str(tmp_path / "n.json")), rpc=near_rpc()[0], now=NOW)
    assert {"guard": "recipient_account_exists", "ok": True, "account": "x3-demo.near"} in result["guards"]


def test_implicit_accounts_are_not_looked_up():
    rpc, calls = near_rpc(missing={"a" * 64})
    assert rpc.account_exists("a" * 64)
    assert rpc.account_exists("0x" + "b" * 40)
    assert calls.calls == []


def test_an_rpc_that_cannot_answer_fails_closed():
    rpc, _ = near_rpc(error={"name": "REQUEST_VALIDATION_ERROR", "cause": {"name": "PARSE_ERROR"}, "code": -32700})
    with pytest.raises(ni.NearIntentsError) as err:
        rpc.account_exists("x3-demo.near")
    assert err.value.code == "X3_NEAR_RPC"


def test_http_errors_carry_the_1click_correlation_id():
    body = {"message": "Internal server error", "correlationId": "0819adac-8f7e-4c9e-ac1a-4d817da91b4f", "path": "/v0/quote"}
    client = ni.OneClickClient(jwt="", transport=RecordingTransport(lambda m, u, p: (400, body)))
    with pytest.raises(ni.NearIntentsError) as err:
        client.quote({"dry": True})
    assert err.value.detail == {"status": 400, "correlationId": "0819adac-8f7e-4c9e-ac1a-4d817da91b4f"}


# --- a real 1Click response ----------------------------------------------------------------

def test_a_captured_mainnet_dry_quote_passes_the_guards_the_example_declares():
    """Captured from 1click.chaindefuser.com on 2026-11-03 (dry quote, recipient wrap.near).

    The server echoes fields this adapter did not send (`depositMode`, `confidentiality`,
    `appFees`, …). The echo check compares the fields that were sent, so those extras pass,
    and the cost they add is what `require fees` measures.
    """
    response = json.loads((ROOT / "tests" / "fixtures" / "near_intents_quote_dry_live_capture.json").read_text())
    sent_fields = ("dry", "swapType", "slippageTolerance", "originAsset", "depositType", "destinationAsset",
                   "amount", "refundTo", "refundType", "recipient", "recipientType", "deadline")
    request = {k: response["quoteRequest"][k] for k in sent_fields}
    leg = {"amount": 25_000_000, "min_receive": 24_800_000, "slippage_bps": 50, "fee_cap_bps": 100}
    results = {r["guard"]: r for r in ni.verify_quote(leg, request, response, now=NOW)}
    assert all(r["ok"] for r in results.values())
    assert results["min_receive"]["guaranteed"] == "24872939"
    assert results["slippage"]["quoted_spread_bps"] == "50.00"
    assert results["fees"]["effective_cost_bps"] == "0.83"
    assert results["refund_fee"]["refund_fee_bps"] == "97.77"

    # The same quote fails a floor or a fee cap it does not meet.
    with pytest.raises(ni.NearIntentsError):
        ni.verify_quote({**leg, "min_receive": 24_900_000}, request, response, now=NOW)
    with pytest.raises(ni.NearIntentsError):
        ni.verify_quote({**leg, "fee_cap_bps": 0}, request, response, now=NOW)


# --- x3-guard contract arguments -----------------------------------------------------------

GUARD_FIXTURE = ROOT / "tests" / "fixtures" / "x3_guard_args.json"


def _settled_receipt(tmp_path):
    receipt_path = tmp_path / "r.json"
    ni.quote(program(tmp_path), client=ni.OneClickClient(jwt="k", transport=api()), live=True,
             nonce_store=ni.NonceStore(str(tmp_path / "n.json")), receipt_path=str(receipt_path),
             rpc=near_rpc()[0], now=NOW)
    ni.track(ni.OneClickClient(jwt="k", transport=api(statuses=[_status("SUCCESS", amountOut="24950000")])),
             "9xQe", receipt_path=str(receipt_path))
    return json.loads(receipt_path.read_text())


def _all_guard_args(receipt):
    return {step: ni.guard_args(receipt, step) for step in ni.GUARD_STEPS}


def test_guard_args_carry_the_policy_and_quote_the_adapter_verified(tmp_path):
    receipt = _settled_receipt(tmp_path)
    args = _all_guard_args(receipt)
    policy = args["register"]["policy"]
    assert args["register"]["nonce"] == "n_001"
    assert policy["program_sha256"] == receipt["program_sha256"]
    assert policy["origin_asset"] == SOL_USDC and policy["destination_asset"] == NEAR_USDC
    assert policy["amount"] == "25000000" and policy["min_receive"] == "24800000"
    assert (policy["slippage_bps"], policy["fee_cap_bps"]) == (50, 100)
    assert policy["deadline_ms"] == int((NOW + timedelta(minutes=10)).timestamp() * 1000)

    quote = args["quote"]["quote"]
    assert quote["quote_response_sha256"] == receipt["quote_response_sha256"]
    assert quote["deposit_address"] == "9xQeWvG816bUx9EPjHmaT23yvVM2ZWbrrpZb9PusVFin"
    assert quote["quote_deadline_ms"] == policy["deadline_ms"]
    assert quote["refund_fee"] == "244425"

    assert args["outcome"] == {"nonce": "n_001", "status": "Settled", "amount_out": "24950000",
                               "refunded_amount": None, "tx_hashes": ["o1", "n1"]}


def test_guard_args_match_the_fixture_the_contract_tests_read(tmp_path):
    """`near/x3-guard/src/tests.rs` deserializes this file and runs the on-chain guards on it.

    If this test fails because the adapter's output changed on purpose, regenerate the
    fixture with `X3_REGEN_GUARD_FIXTURE=1` and run the contract tests.
    """
    import os
    args = _all_guard_args(_settled_receipt(tmp_path))
    if os.environ.get("X3_REGEN_GUARD_FIXTURE") == "1":
        GUARD_FIXTURE.write_text(json.dumps(args, indent=2, sort_keys=True) + "\n")
    assert json.loads(GUARD_FIXTURE.read_text()) == args


def test_outcome_args_need_a_terminal_status(tmp_path):
    receipt_path = tmp_path / "r.json"
    ni.quote(program(tmp_path), client=ni.OneClickClient(jwt="k", transport=api()), live=True,
             nonce_store=ni.NonceStore(str(tmp_path / "n.json")), receipt_path=str(receipt_path),
             rpc=near_rpc()[0], now=NOW)
    receipt = json.loads(receipt_path.read_text())
    with pytest.raises(ni.NearIntentsError) as err:
        ni.guard_args(receipt, "outcome")
    assert err.value.code == "X3_NEAR_NOT_TERMINAL"


def test_a_deposit_address_outliving_the_program_timeout_is_rejected():
    leg, req = _leg_and_request(live=True)
    later = ni.format_timestamp(ni.parse_timestamp(req["deadline"]) + timedelta(seconds=1))
    with pytest.raises(ni.NearIntentsError) as err:
        ni.verify_quote(leg, req, good_quote(req, deadline=later), now=NOW)
    failed = [r for r in err.value.detail if not r["ok"]]
    assert [r["guard"] for r in failed] == ["deadline"]
    assert failed[0]["within_program_timeout"] is False


def test_cli_guard_args_prints_the_near_cli_command(tmp_path, capsys):
    receipt = _settled_receipt(tmp_path)
    (tmp_path / "settled.json").write_text(json.dumps(receipt))
    code = ni.main(["guard-args", "--receipt", str(tmp_path / "settled.json"), "--step", "register",
                    "--contract", "x3guard.near", "--signer", "x3guard.near", "--key-file", "/k.json"])
    assert code == 0
    out = json.loads(capsys.readouterr().out)
    words = __import__("shlex").split(out["command"])
    assert words[:6] == ["near", "contract", "call-function", "as-transaction", "x3guard.near", "register_intent"]
    assert json.loads(words[words.index("json-args") + 1]) == out["args"]
    assert words[-4:] == ["mainnet", "sign-with-access-key-file", "/k.json", "send"]
