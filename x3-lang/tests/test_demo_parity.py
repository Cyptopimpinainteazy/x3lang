"""The browser demo (`near/demo/x3guard.mjs`) and the Python adapter must agree.

The demo re-implements the adapter's lowering, signature check and guards in JavaScript so a
judge can run them in a browser. This suite runs both on the same programs and quotes and
requires the same answers: the same request, the same refusal code, and the same verdict for
every guard. It needs Node 20+ (Ed25519 in WebCrypto); without Node it is skipped, and says so.
"""
import json
import shutil
import subprocess
from datetime import datetime, timezone

import pytest

from test_near_intents import (  # noqa: E402  (shared helpers and fixtures)
    CAPTURES, EXAMPLE, NOW, ONE_CLICK_KEY, ROOT, TEST_KEY, TOKENS, _capture, _leg_and_request,
    good_quote, ni, program, sign,
)

NODE = shutil.which("node")
CLI = ROOT / "near" / "demo" / "cli.mjs"
pytestmark = pytest.mark.skipif(NODE is None, reason="node is not installed; the demo parity suite needs Node 20+")
NOW_MS = int(NOW.timestamp() * 1000)


def js(*args):
    proc = subprocess.run([NODE, str(CLI), *map(str, args)], capture_output=True, text=True, timeout=60)
    assert proc.returncode == 0, proc.stderr
    return json.loads(proc.stdout)


def python_lower(path):
    try:
        planned = ni.plan(str(path), client=ni.OneClickClient(jwt=""), tokens_path=str(TOKENS), now=NOW)
    except ni.NearIntentsError as exc:
        return {"error": exc.code}
    leg = {**planned["leg"], "amount": str(planned["leg"]["amount"]),
           "min_receive": None if planned["leg"]["min_receive"] is None else str(planned["leg"]["min_receive"])}
    return {"leg": leg, "request": planned["quote_request"]}


VARIANTS = [
    {},
    {"requires": ["require slippage <= 0.29%"]},
    {"requires": ["require slippage < 0.5%", "require fees <= 0.75%"]},
    {"requires": ["require slippage <= 0.003", "require nonce unused abc_1"]},
    {"timeout": "timeout 90s refund Solana.USDC to So11111111111111111111111111111111111111112"},
    {"requires": []},
    {"requires": ["require slippage >= 0.5%"]},
    {"requires": ["require slippage <= 30"]},
    {"requires": ["require slippage <= 50 bps"]},
    {"requires": ["require slippage <= 50bps", "require fees <= 100 bps"]},
    {"requires": ["require slippage <= 0.5"]},
    {"requires": ["require slippage <= 0.5%", "require finality Solana >= 32"]},
    {"timeout": None},
    {"timeout": "timeout 2blocks refund Solana.USDC"},
    {"timeout": "timeout 10m refund Solana.SOL"},
    {"on_fail": "on_fail halt"},
    {"route": "bridge X3 Solana.USDC -> Near.USDC;"},
    {"route": "bridge NearIntents Solana.SOL -> Near.USDC;"},
    {"route": "swap Raydium Solana.USDC -> Solana.SOL amount 25000000 min_output 1;\n        bridge NearIntents Solana.SOL -> Near.USDC;"},
]


@pytest.mark.parametrize("variant", VARIANTS, ids=lambda v: ",".join(f"{k}" for k in v) or "default")
def test_programs_lower_identically(tmp_path, variant):
    path = program(tmp_path, **variant)
    expected = python_lower(path)
    got = js("lower", path, TOKENS, NOW_MS)
    if "error" in expected:
        assert got.get("error", {}).get("code") == expected["error"], got
    else:
        assert got == expected


def test_the_shipped_example_lowers_identically():
    assert js("lower", EXAMPLE, TOKENS, NOW_MS) == python_lower(EXAMPLE)


@pytest.mark.parametrize("name", CAPTURES)
def test_real_quotes_verify_identically(name):
    response = _capture(name)
    got = js("verify", ROOT / "tests" / "fixtures" / name, ONE_CLICK_KEY)
    assert got["valid"] is True
    assert got["hash"] == ni.oneclick_quote_hash(response)
    assert got["payload"] == ni.signed_quote_payload(response)


def _guard_view(results):
    keep = ("quoted_spread_bps", "effective_cost_bps", "refund_fee_bps", "guaranteed", "quote_hash", "within_program_timeout", "signed")
    return [{"guard": r["guard"], "ok": r["ok"], **{k: r[k] for k in keep if k in r and r[k] is not None}} for r in results]


def _python_guards(leg, request, response, now, key):
    try:
        return ni.verify_quote(leg, request, response, now=now, manager_public_key=key)
    except ni.NearIntentsError as exc:
        return exc.detail


def _case(tmp_path, leg, request, response, now, key):
    case = tmp_path / "case.json"
    js_leg = {**leg, "amount": str(leg["amount"]), "min_receive": None if leg.get("min_receive") is None else str(leg["min_receive"])}
    case.write_text(json.dumps({"leg": js_leg, "request": request, "response": response,
                                "now_ms": int(now.timestamp() * 1000), "manager_public_key": key}))
    return js("guards", case)


MUTATIONS = [
    None,
    lambda q: q["quote"].update(minAmountOut="24700000", amountOut="24750000"),
    lambda q: q["quote"].update(minAmountOut="24850000", amountOut="25500000"),
    lambda q: q["quote"].update(amountOutUsd="24.60"),
    lambda q: q["quote"].update(refundFee="250001"),
    lambda q: q["quote"].pop("depositAddress"),
    lambda q: q["quoteRequest"].update(recipient="attacker.near"),
    lambda q: q["quote"].update(timeEstimate=900),
]


@pytest.mark.parametrize("mutate", MUTATIONS, ids=lambda m: "good" if m is None else "mutated")
@pytest.mark.parametrize("resign", [True, False], ids=["signed-by-1click", "tampered"])
def test_guards_agree_on_live_quotes(tmp_path, mutate, resign):
    if mutate is None and not resign:
        pytest.skip("an unmodified quote is not tampered")
    leg, req = _leg_and_request(live=True)
    response = good_quote(req)
    if mutate:
        mutate(response)
    if resign:
        sign(response)
    expected = _python_guards(leg, req, response, NOW, TEST_KEY)
    got = _case(tmp_path, leg, req, response, NOW, TEST_KEY)
    assert _guard_view(got["results"]) == _guard_view(expected)


@pytest.mark.parametrize("name", CAPTURES)
def test_guards_agree_on_real_dry_quotes(tmp_path, name):
    response = _capture(name)
    sent = ("dry", "swapType", "slippageTolerance", "originAsset", "depositType", "destinationAsset",
            "amount", "refundTo", "refundType", "recipient", "recipientType", "deadline")
    request = {k: response["quoteRequest"][k] for k in sent}
    leg = {"amount": 25_000_000, "min_receive": 24_800_000, "slippage_bps": 50, "fee_cap_bps": 100,
           "deadline": request["deadline"]}
    now = ni.parse_timestamp(response["timestamp"])
    expected = _python_guards(leg, request, response, now, ONE_CLICK_KEY)
    got = _case(tmp_path, leg, request, response, now, ONE_CLICK_KEY)
    assert _guard_view(got["results"]) == _guard_view(expected)
    assert got["verdict"] == "dry_pass"


# --- agent mandates ---------------------------------------------------------------------------

from test_near_mandate import (  # noqa: E402
    AGENT_INTENT, BASE, MANDATE, POLICY_CASES, agent_intent, mandate_file, nm, replaced, tokens,
)
from test_near_mandate_boundaries import (  # noqa: E402
    BPS_CONTROLS, NUMERIC_CONTROLS, NUMERIC_REFUSALS, numeric_mandate_file,
)


def test_the_agent_example_lowers_identically():
    expected = python_lower(AGENT_INTENT)
    assert expected["leg"]["mandate_owner"] == "x3-owner.near"
    assert js("lower", AGENT_INTENT, TOKENS, NOW_MS) == expected


def test_the_example_mandate_compiles_identically():
    got = js("mandate", MANDATE, TOKENS, NOW_MS)["compiled"]
    assert got == nm.compile_mandate(str(MANDATE), tokens(), now=NOW)


MANDATE_REFUSALS = [
    BASE + ["spend_everything yes"],
    BASE + ["per_payment 1"],
    replaced("expires", None),
    BASE + ["require nonce unused x"],
    replaced("agent", "agent x3-owner.near"),
    replaced("per_24h", "per_24h 1000"),
    replaced("per_payment", "per_payment 0"),
    replaced("refund_to", "refund_to x3-owner.near"),
    replaced("allow recipients", "allow recipients UPPER.near"),
    replaced("allow assets", "allow assets Near.NOPE"),
    replaced("require fees", None),
    replaced("expires", "expires 0d"),
    BASE + ["approve above 50000000 by 1 of x3-owner.near"],
    BASE + ["approve above 1 by 1 of x3-agent.near"],
    BASE + ["approve above 1 by 3 of x3-owner.near, carol.near"],
]


@pytest.mark.parametrize("lines", MANDATE_REFUSALS, ids=lambda ls: ls[-1] if ls is not BASE else "base")
def test_mandates_are_refused_with_the_same_code(tmp_path, lines):
    path = mandate_file(tmp_path, *lines)
    with pytest.raises(ni.NearIntentsError) as err:
        nm.compile_mandate(path, tokens(), now=NOW)
    assert js("mandate", path, TOKENS, NOW_MS).get("error", {}).get("code") == err.value.code


@pytest.mark.parametrize("changes,code,field", NUMERIC_REFUSALS)
def test_mandate_numeric_range_errors_have_browser_parity(tmp_path, changes, code, field):
    path = numeric_mandate_file(tmp_path, changes)
    with pytest.raises(ni.NearIntentsError) as err:
        nm.compile_mandate(path, tokens(), now=NOW)
    assert err.value.code == code
    browser_error = js("mandate", path, TOKENS, NOW_MS).get("error", {})
    assert browser_error.get("code") == code
    if field in {"per_payment", "per_24h", "approve", "expires"}:
        assert browser_error.get("line") == err.value.detail["line"]


@pytest.mark.parametrize("changes,expected", NUMERIC_CONTROLS)
def test_valid_mandate_numeric_boundaries_have_browser_parity(tmp_path, changes, expected):
    path = numeric_mandate_file(tmp_path, changes)
    compiled = nm.compile_mandate(path, tokens(), now=NOW)
    assert {key: compiled["terms"][key] for key in expected} == expected
    assert js("mandate", path, TOKENS, NOW_MS).get("compiled") == compiled


@pytest.mark.parametrize("kind,term", [("slippage", "max_slippage_bps"), ("fees", "max_fee_bps")])
@pytest.mark.parametrize("bound,expected", BPS_CONTROLS)
def test_mandate_bps_boundaries_have_browser_parity(tmp_path, kind, term, bound, expected):
    path = numeric_mandate_file(tmp_path, {f"require {kind}": bound})
    compiled = nm.compile_mandate(path, tokens(), now=NOW)
    assert compiled["terms"][term] == expected
    assert js("mandate", path, TOKENS, NOW_MS).get("compiled") == compiled


@pytest.mark.parametrize("kind", ["slippage", "fees"])
def test_mandate_strictest_bps_clause_has_browser_parity(tmp_path, kind):
    path = mandate_file(tmp_path, *BASE, f"require {kind} <= 10001bps")
    assert js("mandate", path, TOKENS, NOW_MS).get("compiled") == nm.compile_mandate(path, tokens(), now=NOW)


@pytest.mark.parametrize("name,kwargs,expected", POLICY_CASES, ids=[c[0] for c in POLICY_CASES])
def test_an_agent_intent_gets_the_same_mandate_verdict(tmp_path, name, kwargs, expected):
    intent = agent_intent(tmp_path, **kwargs)
    got = js("mandate", MANDATE, TOKENS, NOW_MS, intent)
    _, leg = ni.compile_program(intent, now=NOW)
    request = ni.build_quote_request(leg, tokens(), dry=True)
    policy = ni.policy_for(leg, request, got["policy"]["program_sha256"])
    assert got["policy"] == policy
    python = nm.check_mandate({**nm.compile_mandate(str(MANDATE), tokens(), now=NOW), "version": None,
                               "revoked": False, "spends": []}, policy, NOW_MS)
    assert [(c["guard"], c["ok"]) for c in got["checks"]] == [(c["guard"], c["ok"]) for c in python]
    assert [c["guard"] for c in python if not c["ok"]] == expected
