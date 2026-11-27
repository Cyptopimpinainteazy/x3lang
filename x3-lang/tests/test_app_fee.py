"""The `fee <rate> to <account>` clause: an integrator fee paid through 1Click `appFees`.

What is asserted, in the order a route meets it:

- the parser reads the clause and refuses a malformed or repeated one by name;
- lowering refuses a fee that is not whole bps, is zero, exceeds 1Click's 500 bps ceiling, is
  paid to something that is not a NEAR account, or is not covered by the program's own
  `require fees` cap (including a program with no cap at all);
- the quote request carries exactly the declared fee, and a program without one sends none;
- the quote is refused when 1Click's echo drops, re-prices or re-addresses the declared fee,
  while 1Click's own extra fee entries pass and are reported;
- the payer's protection is the signed amounts: a quote whose signed cost exceeds the cap is
  refused even though the fee itself fits under it;
- under a mandate the fee recipient must be an allowed recipient;
- a receipt whose saved fee or request was edited after the fact no longer binds;
- surfaces that cannot pay a fee refuse the program instead of dropping the fee.
"""
import copy
import json

import pytest

import cli
from test_near_intents import (  # noqa: F401  (_trust_the_test_key is an autouse fixture)
    NOW, ROOT, SOL_ADDR, TOKENS, FakeNearChain, NEAR_USDC, _trust_the_test_key, api, good_quote, near_rpc, ni,
    program, sign,
)

FEE_EXAMPLE = ROOT / "examples" / "near_intents_app_fee.x3"
FEE_ACCOUNT = "fees.x3-demo.near"
BASE_REQUIRES = ["require slippage <= 0.5%", "require fees <= 100 bps", "require nonce unused n_001"]


def fee_program(tmp_path, fee=f"fee 25 bps to {FEE_ACCOUNT}", requires=None):
    lines = list(BASE_REQUIRES if requires is None else requires)
    if fee is not None:
        lines.append(fee)
    return program(tmp_path, requires=lines)


def plan(path, live=False):
    return ni.plan(str(path), client=ni.OneClickClient(jwt="", transport=api()), tokens_path=str(TOKENS),
                   live=live, now=NOW)


def lower_error(tmp_path, **kwargs):
    with pytest.raises(ni.NearIntentsError) as err:
        plan(fee_program(tmp_path, **kwargs))
    return err.value


# --- parsing -------------------------------------------------------------------------------

def test_the_parser_reads_a_fee_clause():
    intent = cli.parse_file(str(FEE_EXAMPLE))
    assert intent["app_fee"]["value"] == "25 bps"
    assert intent["app_fee"]["recipient"] == FEE_ACCOUNT


@pytest.mark.parametrize("line", [
    "fee",
    "fee 25 bps",
    "fee to fees.near",
    f"fee 25 bps to {FEE_ACCOUNT} extra.near",
    "fee 25 bps to",
])
def test_a_malformed_fee_clause_is_refused_by_name(tmp_path, line):
    with pytest.raises(cli.X3ParseError) as err:
        cli.parse_file(fee_program(tmp_path, fee=line))
    assert err.value.code == "X3_PARSE_FEE"
    assert err.value.line is not None


def test_a_second_fee_clause_is_refused(tmp_path):
    path = fee_program(tmp_path, requires=[*BASE_REQUIRES, "fee 10 bps to a.near"])
    with pytest.raises(cli.X3ParseError) as err:
        cli.parse_file(path)
    assert err.value.code == "X3_PARSE_FEE"


# --- lowering ------------------------------------------------------------------------------

def test_the_example_lowers_its_fee_and_sends_it_as_app_fees():
    planned = plan(FEE_EXAMPLE)
    assert planned["leg"]["app_fee"] == {"recipient": FEE_ACCOUNT, "bps": 25}
    assert planned["quote_request"]["appFees"] == [{"recipient": FEE_ACCOUNT, "fee": 25}]


def test_a_percent_rate_is_converted_to_whole_bps(tmp_path):
    planned = plan(fee_program(tmp_path, fee=f"fee 0.25% to {FEE_ACCOUNT}"))
    assert planned["leg"]["app_fee"]["bps"] == 25


def test_a_fee_equal_to_the_cap_is_allowed(tmp_path):
    planned = plan(fee_program(tmp_path, fee=f"fee 100 bps to {FEE_ACCOUNT}"))
    assert planned["leg"]["app_fee"]["bps"] == 100


def test_a_program_without_a_fee_sends_no_app_fees(tmp_path):
    planned = plan(fee_program(tmp_path, fee=None))
    assert "app_fee" not in planned["leg"]
    assert "appFees" not in planned["quote_request"]


@pytest.mark.parametrize("fee,code", [
    (f"fee 2.5 bps to {FEE_ACCOUNT}", "X3_NEAR_APP_FEE"),
    (f"fee 0 bps to {FEE_ACCOUNT}", "X3_NEAR_APP_FEE"),
    (f"fee 25 to {FEE_ACCOUNT}", "X3_NEAR_BOUND_UNIT"),
    (f"fee -5 bps to {FEE_ACCOUNT}", "X3_NEAR_APP_FEE"),
    (f"fee 1e2 bps to {FEE_ACCOUNT}", "X3_NEAR_APP_FEE"),
    ("fee 25 bps to Fees.Near", "X3_NEAR_APP_FEE_RECIPIENT"),
    ("fee 25 bps to 4Nd1mzi8Y1QYxJt9wZWBYZpG7S4pYkZs6YzD3Vt9aBcD", "X3_NEAR_APP_FEE_RECIPIENT"),
    (f"fee 101 bps to {FEE_ACCOUNT}", "X3_NEAR_APP_FEE_OVER_CAP"),
])
def test_a_fee_the_route_cannot_honour_is_refused(tmp_path, fee, code):
    assert lower_error(tmp_path, fee=fee).code == code


def test_a_fee_above_one_clicks_ceiling_is_refused_whatever_the_cap(tmp_path):
    err = lower_error(tmp_path, fee=f"fee 501 bps to {FEE_ACCOUNT}",
                      requires=["require slippage <= 0.5%", "require fees <= 10%", "require nonce unused n_001"])
    assert err.code == "X3_NEAR_APP_FEE"


def test_a_fee_without_a_cost_cap_is_refused(tmp_path):
    err = lower_error(tmp_path, requires=["require slippage <= 0.5%", "require nonce unused n_001"])
    assert err.code == "X3_NEAR_APP_FEE_CAP_REQUIRED"


def test_a_strict_cap_is_rounded_toward_the_payer(tmp_path):
    # `< 25 bps` allows at most 24 whole bps, so a 25 bps fee does not fit.
    err = lower_error(tmp_path, requires=["require slippage <= 0.5%", "require fees < 25 bps", "require nonce unused n_001"])
    assert err.code == "X3_NEAR_APP_FEE_OVER_CAP"


# --- quote verification --------------------------------------------------------------------

def _fee_leg_and_request(live=True):
    planned = plan(FEE_EXAMPLE, live=live)
    return planned["leg"], planned["quote_request"]


def _guards(leg, request, response):
    try:
        return {r["guard"]: r for r in ni.verify_quote(leg, request, response, now=NOW)}
    except ni.NearIntentsError as exc:
        assert exc.code == "X3_NEAR_GUARD_FAILED", exc.code
        return {r["guard"]: r for r in exc.detail}


def test_a_quote_echoing_the_declared_fee_passes_every_guard():
    leg, req = _fee_leg_and_request()
    results = ni.verify_quote(leg, req, good_quote(req), now=NOW)
    names = [r["guard"] for r in results]
    assert names.index("app_fee_echo") == names.index("quote_request_echo") + 1
    by = {r["guard"]: r for r in results}
    assert by["app_fee_echo"]["ok"] and by["app_fee_echo"]["signed"] is False
    assert by["app_fee_echo"]["undeclared"] == []
    assert by["app_fee_cap"] == {"guard": "app_fee_cap", "ok": True, "fee_bps": 25, "recipient": FEE_ACCOUNT,
                                 "cap_bps": 100}


@pytest.mark.parametrize("echoed", [
    None,                                                       # dropped
    [],                                                         # dropped
    [{"recipient": FEE_ACCOUNT, "fee": 24}],                    # re-priced
    [{"recipient": "attacker.near", "fee": 25}],                # re-addressed
    [{"recipient": FEE_ACCOUNT, "fee": "25"}],                  # wrong type
    [{"recipient": FEE_ACCOUNT, "fee": True}],                  # bool is not a fee
    [{"recipient": FEE_ACCOUNT, "fee": 25, "limitOrderId": "x"}],  # a set extra key changes meaning
    {"recipient": FEE_ACCOUNT, "fee": 25},                      # not a list
])
def test_a_quote_whose_echo_does_not_carry_the_declared_fee_is_refused(echoed):
    leg, req = _fee_leg_and_request()
    response = good_quote(req)
    if echoed is None:
        response["quoteRequest"].pop("appFees")
    else:
        response["quoteRequest"]["appFees"] = echoed
    sign(response)  # signed by "1Click": the echo check, not the signature, has to catch it
    results = _guards(leg, req, response)
    assert results["quote_signature"]["ok"]
    assert results["app_fee_echo"]["ok"] is False


def test_tampering_with_the_echoed_fee_does_not_touch_the_signature():
    leg, req = _fee_leg_and_request()
    response = good_quote(req)
    response["quoteRequest"]["appFees"] = [{"recipient": "attacker.near", "fee": 25}]
    # Not re-signed: appFees is outside the signed payload, so the signature still verifies.
    results = _guards(leg, req, response)
    assert results["quote_signature"]["ok"]
    assert results["app_fee_echo"]["ok"] is False


def test_one_clicks_own_fee_entries_pass_and_are_reported():
    leg, req = _fee_leg_and_request()
    response = good_quote(req)
    own = {"recipient": "5880ad2b362620fadf759cbceb1cd5737ce8c6ed7fb8e9942881e6731f9247dd", "fee": 1,
           "limitOrderId": None}
    response["quoteRequest"]["appFees"] = [own, {"recipient": FEE_ACCOUNT, "fee": 25, "limitOrderId": None}]
    sign(response)
    results = _guards(leg, req, response)
    assert results["app_fee_echo"]["ok"]
    assert results["app_fee_echo"]["undeclared"] == [{"recipient": own["recipient"], "fee": 1}]


def test_the_signed_cost_still_bounds_the_payer_when_the_fee_fits():
    leg, req = _fee_leg_and_request()
    # 25 USDC in at $24.995, $24.70 out: about 118 bps, over the 100 bps cap.
    response = good_quote(req, amount_out=24_700_000, min_out=24_700_000 - 120_000, usd_out="24.70")
    results = _guards(leg, req, response)
    assert results["app_fee_echo"]["ok"] and results["app_fee_cap"]["ok"]
    assert results["fees"]["ok"] is False


def test_a_fee_program_must_pay_an_account_that_exists(tmp_path):
    rpc, _ = near_rpc(missing={FEE_ACCOUNT})
    with pytest.raises(ni.NearIntentsError) as err:
        ni.quote(str(FEE_EXAMPLE), client=ni.OneClickClient(jwt="", transport=api()), tokens_path=str(TOKENS),
                 nonce_store=ni.NonceStore(str(tmp_path / "n.json")), rpc=rpc, now=NOW)
    assert err.value.code == "X3_NEAR_ACCOUNT_UNKNOWN"
    assert err.value.field == "fee.recipient"


# --- receipts ------------------------------------------------------------------------------

def live_fee_receipt(tmp_path):
    chain = FakeNearChain(balances={(NEAR_USDC.split(":", 1)[1], "x3-demo.near"): 1_000_000})
    rpc, _ = near_rpc(chain=chain)
    path = tmp_path / "fee-receipt.json"
    result = ni.quote(str(FEE_EXAMPLE), client=ni.OneClickClient(jwt="k", transport=api()), tokens_path=str(TOKENS),
                      live=True, nonce_store=ni.NonceStore(str(tmp_path / "n.json")), receipt_path=str(path),
                      rpc=rpc, now=NOW)
    return path, result


def test_a_live_fee_quote_writes_a_receipt_that_records_the_fee(tmp_path):
    path, result = live_fee_receipt(tmp_path)
    receipt = json.loads(path.read_text())
    assert receipt["leg"]["app_fee"] == {"recipient": FEE_ACCOUNT, "bps": 25}
    assert receipt["quote_request"]["appFees"] == [{"recipient": FEE_ACCOUNT, "fee": 25}]
    guards = {g["guard"]: g for g in receipt["guards"]}
    assert guards["app_fee_recipient_account_exists"]["ok"]
    assert guards["app_fee_echo"]["ok"]
    ni._signed_quote(receipt)  # binds as written


@pytest.mark.parametrize("edit", [
    lambda r: r["quote_request"].update(appFees=[{"recipient": "attacker.near", "fee": 25}]),
    lambda r: r["quote_request"].pop("appFees"),
    lambda r: r["leg"].update(app_fee={"recipient": FEE_ACCOUNT, "bps": 10}),
    lambda r: r["leg"].pop("app_fee"),
])
def test_a_receipt_whose_fee_was_edited_no_longer_binds(tmp_path, edit):
    path, _ = live_fee_receipt(tmp_path)
    receipt = json.loads(path.read_text())
    edit(receipt)
    with pytest.raises(ni.NearIntentsError) as err:
        ni._signed_quote(receipt)
    assert err.value.code == "X3_EVIDENCE_BINDING"


def test_a_handoff_refuses_a_program_whose_fee_changed_after_the_quote(tmp_path):
    path, _ = live_fee_receipt(tmp_path)
    receipt = json.loads(path.read_text())
    edited = tmp_path / "edited.x3"
    edited.write_text(FEE_EXAMPLE.read_text().replace("fee 25 bps", "fee 26 bps"))
    receipt["program_sha256"] = ni._sha256_file(str(edited))  # an attacker can recompute the hash
    with pytest.raises(ni.NearIntentsError) as err:
        ni.prepare_handoff(receipt, receipt_path=str(path), program_path=str(edited), contract="guard.near",
                           signer="owner.near", key_file="/dev/null")
    assert err.value.code == "X3_EVIDENCE_BINDING"
    assert "app_fee" in err.value.message


# --- mandates ------------------------------------------------------------------------------

MANDATE = """mandate fee_agent {
    owner x3-owner.near
    agent x3-agent.near
    spend Solana.USDC
    per_payment 50000000
    per_24h 200000000
    allow recipients %s
    allow assets Near.USDC
    refund_to %s
    require slippage <= 1%%
    require fees <= 100 bps
    expires 30d
}
"""


def _mandate_check(tmp_path, recipients):
    mandate = tmp_path / "m.x3"
    mandate.write_text(MANDATE % (recipients, SOL_ADDR))
    source = FEE_EXAMPLE.read_text().replace("    require slippage", "    under mandate x3-owner.near\n    require slippage", 1)
    agent_program = tmp_path / "agent.x3"
    agent_program.write_text(source)
    planned = plan(agent_program)
    tokens = json.loads(TOKENS.read_text())
    return ni.check_against_mandate(planned["leg"], planned["quote_request"], "0" * 64, str(mandate),
                                    tokens=tokens, now=NOW)


def test_a_mandate_allows_a_fee_to_an_allowed_recipient(tmp_path):
    out = _mandate_check(tmp_path, f"x3-demo.near, {FEE_ACCOUNT}")
    check = next(c for c in out["checks"] if c["guard"] == "mandate_app_fee_recipient")
    assert check["ok"] and check["enforced_by"] == "adapter"


def test_a_mandate_refuses_a_fee_to_a_recipient_the_owner_did_not_allow(tmp_path):
    with pytest.raises(ni.NearIntentsError) as err:
        _mandate_check(tmp_path, "x3-demo.near")
    assert err.value.code == "X3_MANDATE_DENIED"
    failed = [c["guard"] for c in err.value.detail["checks"] if not c["ok"]]
    assert failed == ["mandate_app_fee_recipient"]


# --- other surfaces ------------------------------------------------------------------------

def test_the_generic_runner_refuses_a_fee_it_cannot_pay():
    import runner
    out = runner.run(str(FEE_EXAMPLE), dry_run=True)
    assert out["status"] == "error"
    assert out["errors"][0]["code"] == "X3_FEE_UNSUPPORTED_SURFACE"


def test_a_program_without_a_fee_is_unchanged_by_this_feature():
    leg_before = plan(ROOT / "examples" / "near_intents_sol_to_near.x3")
    assert "app_fee" not in leg_before["leg"]
    assert "appFees" not in leg_before["quote_request"]
    assert copy.deepcopy(leg_before["leg"]).get("fee_cap_bps") == 100
