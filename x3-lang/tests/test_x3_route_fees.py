"""X3 Route integrator fees: `fee <bps> bps to <account>` in an intent, sent to 1Click as
`appFees`, bounded by the program's own `require fees` cap and measured from signed amounts."""
from __future__ import annotations

import json
from pathlib import Path

import pytest

from test_near_intents import (  # noqa: F401  (_trust_the_test_key is an autouse fixture)
    NOW, FakeNearChain, _trust_the_test_key, ni, near_rpc, sign,
)
from test_x3_route_chains import (  # noqa: F401
    BASE_USDC, EVM_DEPOSIT, EVM_TX, evm_rpc, evm_tx, route_api, tokens, transfer_log,
)

ROOT = Path(__file__).resolve().parents[1]
FEE_EXAMPLE = ROOT / "examples" / "x3_route_payment_with_fee.x3"
FEE_LINE = "    fee 25 bps to fees.yourapp.near\n"


def source_with(tmp_path: Path, *, replace=None, add_after_route=None) -> str:
    text = FEE_EXAMPLE.read_text()
    for old, new in (replace or {}).items():
        assert old in text, old
        text = text.replace(old, new)
    if add_after_route:
        text = text.replace("    }\n    fee", "    }\n" + add_after_route + "    fee", 1)
    path = tmp_path / "fee.x3"
    path.write_text(text)
    return str(path)


def fee_quote(request, *, usd_out="24.910", echo_fees=True):
    """A signed Base→NEAR quote. 25 bps app fee plus ~11 bps of 1Click cost ≈ 36 bps total."""
    echoed = dict(request)
    if not echo_fees:
        echoed.pop("appFees", None)
    quote = {"amountIn": request["amount"], "amountInFormatted": "25.0", "amountInUsd": "25.000",
             "minAmountIn": request["amount"], "amountOut": "24910000", "amountOutFormatted": "24.91",
             "amountOutUsd": usd_out, "minAmountOut": str(24_910_000 * (10_000 - request["slippageTolerance"]) // 10_000),
             "timeEstimate": 30, "refundFee": "50000"}
    if not request["dry"]:
        quote.update({"depositAddress": EVM_DEPOSIT, "deadline": request["deadline"],
                      "timeWhenInactive": request["deadline"]})
    return sign({"correlationId": "fee-c0ffee", "timestamp": "2026-11-03T03:00:00.000Z", "signature": "",
                 "quoteRequest": echoed, "quote": quote})


# --- the clause ------------------------------------------------------------------------------

def test_the_fee_clause_reaches_the_1click_request():
    intent, leg = ni.compile_program(str(FEE_EXAMPLE), now=NOW)
    assert intent["app_fees"] == [{"bps": 25, "recipient": "fees.yourapp.near"}]
    assert leg["app_fees"] == [{"recipient": "fees.yourapp.near", "fee": 25}]
    request = ni.build_quote_request(leg, tokens(), dry=True)
    assert request["appFees"] == [{"recipient": "fees.yourapp.near", "fee": 25}]


def test_a_program_without_a_fee_sends_no_app_fees():
    _, leg = ni.compile_program(str(ROOT / "examples" / "x3_route_base_usdc_to_near.x3"), now=NOW)
    assert "app_fees" not in leg and "appFees" not in ni.build_quote_request(leg, tokens(), dry=True)


def test_two_recipients_split_the_fee(tmp_path):
    path = source_with(tmp_path, replace={FEE_LINE: "    fee 20 bps to fees.yourapp.near\n    fee 5 bps to route.x3lang.near\n"})
    _, leg = ni.compile_program(path, now=NOW)
    assert ni.build_quote_request(leg, tokens(), dry=True)["appFees"] == [
        {"recipient": "fees.yourapp.near", "fee": 20}, {"recipient": "route.x3lang.near", "fee": 5}]


@pytest.mark.parametrize("line", [
    "fee 0 bps to fees.yourapp.near",
    "fee 10001 bps to fees.yourapp.near",
    "fee 25 pct to fees.yourapp.near",
    "fee 25 bps for fees.yourapp.near",
    "fee 25 bps to Fees.near",
    "fee 0.25 bps to fees.yourapp.near",
    "fee bps to fees.yourapp.near",
    "fee 25 bps to 0x" + "a" * 40,
])
def test_malformed_fee_clauses_are_refused(tmp_path, line):
    with pytest.raises(ni.NearIntentsError) as err:
        ni.compile_program(source_with(tmp_path, replace={"fee 25 bps to fees.yourapp.near": line}), now=NOW)
    assert err.value.code == "X3_PARSE_FEE", err.value


def test_the_same_recipient_twice_is_refused(tmp_path):
    path = source_with(tmp_path, replace={FEE_LINE: FEE_LINE + "    fee 5 bps to fees.yourapp.near\n"})
    with pytest.raises(ni.NearIntentsError) as err:
        ni.compile_program(path, now=NOW)
    assert err.value.code == "X3_PARSE_FEE" and "declared twice" in err.value.message


def test_a_fee_needs_a_cost_cap(tmp_path):
    with pytest.raises(ni.NearIntentsError) as err:
        ni.compile_program(source_with(tmp_path, replace={"    require fees <= 100 bps\n": ""}), now=NOW)
    assert err.value.code == "X3_NEAR_FEE_CAP_REQUIRED"


def test_a_fee_above_the_programs_own_cap_is_refused(tmp_path):
    with pytest.raises(ni.NearIntentsError) as err:
        ni.compile_program(source_with(tmp_path, replace={"fee 25 bps": "fee 150 bps"}), now=NOW)
    assert err.value.code == "X3_NEAR_FEE_OVER_CAP"


def test_an_agent_cannot_write_itself_a_fee(tmp_path):
    """A hijacked agent writing `fee … to attacker.near` under a mandate is refused outright."""
    path = source_with(tmp_path, replace={"fee 25 bps to fees.yourapp.near": "fee 50 bps to attacker.near"},
                       add_after_route="    under mandate x3-owner.near\n")
    with pytest.raises(ni.NearIntentsError) as err:
        ni.compile_program(path, now=NOW)
    assert err.value.code == "X3_MANDATE_APP_FEE"


# --- the quote -------------------------------------------------------------------------------

def quoted(**kw):
    _, leg = ni.compile_program(str(FEE_EXAMPLE), now=NOW)
    request = ni.build_quote_request(leg, tokens(), dry=False)
    request["deadline"] = leg["deadline"]
    return leg, request, fee_quote(request, **kw)


def test_a_quote_that_takes_the_fee_within_the_cap_passes():
    leg, request, response = quoted()
    results = {r["guard"]: r for r in ni.verify_quote(leg, request, response, now=NOW)}
    assert results["app_fees_echo"]["ok"] and results["app_fees_echo"]["signed"] is False
    assert results["fees"]["ok"] and results["fees"]["app_fee_bps"] == 25
    assert results["fees"]["effective_cost_bps"] == "36.00"


def test_a_quote_that_drops_the_fee_terms_is_refused():
    leg, request, response = quoted(echo_fees=False)
    with pytest.raises(ni.NearIntentsError) as err:
        ni.verify_quote(leg, request, response, now=NOW)
    failed = {r["guard"] for r in err.value.detail if not r["ok"]}
    assert "app_fees_echo" in failed


def test_the_cap_bounds_the_fee_and_every_other_charge_together():
    # The fee is 25 bps, but the signed amounts show 1.5% gone in total: over the 100 bps cap.
    leg, request, response = quoted(usd_out="24.625")
    with pytest.raises(ni.NearIntentsError) as err:
        ni.verify_quote(leg, request, response, now=NOW)
    failed = {r["guard"] for r in err.value.detail if not r["ok"]}
    assert failed == {"fees"}


# --- accounts, receipts ------------------------------------------------------------------------

def live_fee_receipt(tmp_path, *, missing=()):
    rpc, _ = near_rpc(chain=FakeNearChain(missing=missing, balances={}))
    path = tmp_path / "fee.json"
    ni.quote(str(FEE_EXAMPLE), client=ni.OneClickClient(jwt="k", transport=route_api(fee_quote)), live=True,
             nonce_store=ni.NonceStore(str(tmp_path / "n.json")), receipt_path=str(path), rpc=rpc, now=NOW)
    return json.loads(path.read_text())


def test_a_fee_to_an_account_that_does_not_exist_is_refused_before_quoting(tmp_path):
    with pytest.raises(ni.NearIntentsError) as err:
        live_fee_receipt(tmp_path, missing={"fees.yourapp.near"})
    assert err.value.code == "X3_NEAR_ACCOUNT_UNKNOWN" and err.value.field == "app_fees"


def test_the_receipt_carries_the_fee_and_the_deposit_still_proves(tmp_path):
    receipt = live_fee_receipt(tmp_path)
    assert receipt["leg"]["app_fees"] == [{"recipient": "fees.yourapp.near", "fee": 25}]
    assert receipt["quote_request"]["appFees"] == [{"recipient": "fees.yourapp.near", "fee": 25}]
    tx = evm_tx(logs=[transfer_log(BASE_USDC, EVM_DEPOSIT, 25_000_000)])
    rpc, _ = evm_rpc("base", {"https://base-a.invalid": {EVM_TX: tx}})
    assert ni.verify_deposit(receipt, tx_signature=EVM_TX, evm={"base": rpc})["verified"] is True


@pytest.mark.parametrize("tamper", [
    lambda r: r["leg"]["app_fees"][0].update(fee=10),
    lambda r: r["leg"]["app_fees"][0].update(recipient="attacker.near"),
    lambda r: r["leg"].pop("app_fees"),
])
def test_edited_fee_terms_in_a_receipt_are_refused_before_any_rpc(tmp_path, tamper):
    receipt = live_fee_receipt(tmp_path)
    tamper(receipt)
    rpc, fake = evm_rpc("base", {"https://base-a.invalid": {}})
    with pytest.raises(ni.NearIntentsError) as err:
        ni.verify_deposit(receipt, tx_signature=EVM_TX, evm={"base": rpc})
    assert err.value.code == "X3_EVIDENCE_BINDING" and fake.calls == []


# --- verify-receipt ---------------------------------------------------------------------------

def test_a_fee_receipt_verifies_offline_with_its_program(tmp_path, capsys):
    receipt = live_fee_receipt(tmp_path)
    path = tmp_path / "receipt.json"
    path.write_text(json.dumps(receipt))
    code = ni.main(["verify-receipt", "--receipt", str(path), "--program", str(FEE_EXAMPLE)])
    report = json.loads(capsys.readouterr().out)
    assert code == 0 and report["status"] == "valid", report["checks"]
    assert {c["check"] for c in report["checks"]} == {"signed_quote_bound", "program_sha256", "program_terms",
                                                       "guards_recomputed"}
    assert report["fees"]["app_fee_bps"] == 25 and report["fees"]["within_cap"] is True
    assert report["fees"]["app_fee_recipients_signed"] is False
    assert report["verified_outcome"] is None  # nothing proven on chain yet


def test_verify_receipt_flags_the_wrong_program(tmp_path):
    receipt = live_fee_receipt(tmp_path)
    other = tmp_path / "other.x3"
    other.write_text(FEE_EXAMPLE.read_text().replace("fee 25 bps", "fee 10 bps"))
    report = ni.verify_receipt(receipt, program_path=str(other))
    failed = {c["check"] for c in report["checks"] if not c["ok"]}
    assert report["status"] == "invalid" and failed == {"program_sha256", "program_terms"}


def test_verify_receipt_flags_a_tampered_quote(tmp_path):
    receipt = live_fee_receipt(tmp_path)
    receipt["quote_response"]["quote"]["amountOutUsd"] = "25.000"
    report = ni.verify_receipt(receipt)
    assert report["status"] == "invalid"
    assert next(c for c in report["checks"] if c["check"] == "signed_quote_bound")["ok"] is False


def test_verify_receipt_reports_recorded_evidence_without_rereading_chains(tmp_path):
    receipt = live_fee_receipt(tmp_path)
    tx = evm_tx(logs=[transfer_log(BASE_USDC, EVM_DEPOSIT, 25_000_000)])
    rpc, fake = evm_rpc("base", {"https://base-a.invalid": {EVM_TX: tx}})
    ni.verify_deposit(receipt, tx_signature=EVM_TX, evm={"base": rpc})
    calls = len(fake.calls)
    report = ni.verify_receipt(receipt)
    assert report["recorded_evidence"]["deposit"]["verified"] is True
    assert report["recorded_evidence"]["deposit"]["kind"] == "evm_token_transfer"
    assert len(fake.calls) == calls
