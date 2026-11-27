"""Issue #40: ProviderQuote record, 1Click conversion, aggregation and ranking.

Fixtures:
- `fixtures/near_intents_quote_dry_live_capture.json` is a RECORDED 1Click dry quote
  (2026-11-03, real manager signature) already used by `test_near_intents.py`.
- Live (non-dry) quotes below are CONSTRUCTED from that capture and signed with the
  throwaway test key from `test_near_intents.py`; nothing here was quoted live and nothing
  contacts a network.
"""
import copy
import dataclasses
import hashlib
import json
import sys
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))

import near_intents as ni  # noqa: E402
from near_liquidity import ranking as agg  # noqa: E402
from near_liquidity import oneclick, quote as q  # noqa: E402
from near_liquidity.quote import LiquidityError  # noqa: E402

CAPTURE = json.loads((ROOT / "tests" / "fixtures" / "near_intents_quote_dry_live_capture.json").read_text())
TOKENS = json.loads((ROOT / "tests" / "fixtures" / "near_intents_tokens.json").read_text())
ONE_CLICK_KEY = "ed25519:reYaWhvwu8Jzo3WUM3zhn6VrhuMEF4eADL17qtRVifc"
TEST_SEED = hashlib.sha256(b"x3-lang test key: never fund").digest()
TEST_KEY = "ed25519:" + ni.ed25519.b58encode(ni.ed25519.public_key(TEST_SEED))
NOW = 1793678210  # 3 s after the capture's 2026-11-03T03:56:47.575Z timestamp
SOL_USDC = "nep141:sol-5ce3bf3a31af18be40ba30f721101b4341690186.omft.near"
NEAR_USDC = "nep141:17208628f84f5d6ad33f0da3bbbeb27ffcb398eac501a31bd6ad2011e36133a1"
ASSET_IN = q.AssetRef("sol", SOL_USDC, 6)
ASSET_OUT = q.AssetRef("near", NEAR_USDC, 6)


def captured_request():
    """What X3 sent for the recorded capture: the echoed fields minus 1Click's additions."""
    echoed = CAPTURE["quoteRequest"]
    return {k: echoed[k] for k in ni._ECHO_FIELDS if k in echoed}


def sign(response):
    message = ni.oneclick_quote_hash(response).encode()
    response["signature"] = "ed25519:" + ni.ed25519.b58encode(ni.ed25519.sign(TEST_SEED, message))
    return response


def live(*, amount_out="24997929", min_out="24872939", withdraw_fee="0", correlation="c-1",
         deadline="2026-11-03T04:06:46.297Z", app_fees=None, recipient_type="DESTINATION_CHAIN",
         min_amount_in="25000000"):
    """A CONSTRUCTED live quote, signed with the test key."""
    request = dict(captured_request(), dry=False, recipientType=recipient_type)
    response = copy.deepcopy(CAPTURE)
    response["quoteRequest"].update(dry=False, recipientType=recipient_type)
    if app_fees is None:
        response["quoteRequest"].pop("appFees", None)
    else:
        response["quoteRequest"]["appFees"] = app_fees
    out_units = int(amount_out)
    response["quote"].update(amountOut=amount_out, minAmountOut=min_out, withdrawFee=withdraw_fee,
                             amountOutFormatted=f"{out_units // 10**6}.{out_units % 10**6:06d}",
                             minAmountIn=min_amount_in, deadline=deadline,
                             depositAddress="ConstructedDepositAddressNeverFund1111111111")
    response["correlationId"] = correlation
    return request, sign(response)


@pytest.fixture
def trust_test_key(monkeypatch):
    monkeypatch.setattr(ni, "ONE_CLICK_MANAGER_PUBLIC_KEY", TEST_KEY)


def convert(request, response, tokens=TOKENS, now=NOW):
    return oneclick.quote_from_oneclick(request, response, tokens, now=now)


def agg_request(**overrides):
    values = dict(asset_in=ASSET_IN, asset_out=ASSET_OUT, amount_in=25_000_000, now=NOW, max_age_s=300,
                  min_remaining_s=60)
    values.update(overrides)
    return agg.AggregationRequest(**values)


# ---------------------------------------------------------------- task 2: the record ----

def test_recorded_1click_quote_verifies_under_pinned_key_but_is_not_executable():
    assert ni.ONE_CLICK_MANAGER_PUBLIC_KEY == ONE_CLICK_KEY
    quote = convert(captured_request(), CAPTURE)
    assert quote.signature.status is q.SignatureStatus.VERIFIED
    assert quote.signature.signer == ONE_CLICK_KEY
    assert quote.executable is False
    assert "signed_indicative" in quote.non_executable_reason
    assert quote.guaranteed_net_output() is None  # a dry quote guarantees nothing
    assert quote.expiry_source.startswith("policy:")
    assert quote.provenance.reference == CAPTURE["correlationId"]


def test_record_carries_every_field_the_issue_lists():
    record = convert(captured_request(), CAPTURE).to_json()
    for key in ("schema_version", "provider_id", "asset_in", "asset_out", "amount_in", "amount_out", "min_amount_out",
                "expires_at", "fees", "provenance", "request_hash", "signature", "executable"):
        assert key in record
    assert record["schema_version"] == "x3.provider_quote.v1"
    assert record["asset_in"] == {"network": "sol", "asset_id": SOL_USDC, "decimals": 6}
    assert {f["source_field"] for f in record["fees"]} == {"quote.withdrawFee", "quote.refundFee",
                                                           "quoteRequest.appFees[0].fee"}
    assert record["executable"] is False


def test_executable_flag_has_no_default():
    fields = {f.name: f for f in dataclasses.fields(q.ProviderQuote)}
    assert fields["executable"].default is dataclasses.MISSING
    assert fields["non_executable_reason"].default is dataclasses.MISSING


def test_signature_status_cannot_be_constructed_or_mutated():
    with pytest.raises(TypeError):
        q.SignatureVerification(status=q.SignatureStatus.VERIFIED)
    status = convert(captured_request(), CAPTURE).signature
    with pytest.raises(AttributeError):
        status.status = q.SignatureStatus.VERIFIED


def test_hand_built_verified_status_is_rejected():
    real = convert(captured_request(), CAPTURE)
    forged = object.__new__(q.SignatureVerification)
    for name in ("status", "scheme", "signer", "provider_id", "response_sha256", "terms_sha256", "reason"):
        object.__setattr__(forged, name, getattr(real.signature, name))
    object.__setattr__(forged, "_mac", b"\0" * 32)
    with pytest.raises(LiquidityError) as err:
        dataclasses.replace(real, signature=forged)
    assert err.value.code == "X3_LIQ_SIGNATURE_STATUS_FORGED"


def test_verified_status_cannot_be_moved_onto_other_terms():
    real = convert(captured_request(), CAPTURE)
    with pytest.raises(LiquidityError) as err:
        dataclasses.replace(real, min_amount_out=real.min_amount_out + 1)
    assert err.value.code == "X3_LIQ_SIGNATURE_STATUS_MISMATCH"


def test_dry_quote_cannot_be_flipped_to_executable():
    real = convert(captured_request(), CAPTURE)
    with pytest.raises(LiquidityError) as err:
        dataclasses.replace(real, executable=True, non_executable_reason=None)
    assert err.value.code == "X3_LIQ_NOT_EXECUTABLE"


def test_unsigned_status_cannot_make_a_quote_executable(trust_test_key):
    real = convert(*live())
    assert real.executable
    unsigned = q.unsigned_status(real.provider_id, real.provenance.response_sha256, real.terms_sha256, "caller says so")
    with pytest.raises(LiquidityError) as err:
        dataclasses.replace(real, signature=unsigned)
    assert err.value.code == "X3_LIQ_NOT_EXECUTABLE"
    assert "verified provider signature" in err.value.message


def test_verified_status_requires_a_pinned_signer_and_a_valid_signature(trust_test_key):
    request, response = live()
    message = ni.oneclick_quote_hash(response).encode()
    other_seed = hashlib.sha256(b"attacker").digest()
    other_key = "ed25519:" + ni.ed25519.b58encode(ni.ed25519.public_key(other_seed))
    with pytest.raises(LiquidityError) as err:
        q._issue_verified("near-intents-1click", public_key=other_key, message=message,
                          signature=response["signature"], response_sha256="0" * 64, terms_sha256="0" * 64)
    assert err.value.code == "X3_LIQ_UNTRUSTED_SIGNER"
    with pytest.raises(LiquidityError) as err:
        q._issue_verified("near-intents-1click", public_key=TEST_KEY, message=message + b"x",
                          signature=response["signature"], response_sha256="0" * 64, terms_sha256="0" * 64)
    assert err.value.code == "X3_LIQ_INVALID_SIGNATURE"
    with pytest.raises(LiquidityError):
        q._issue_verified("rhea-v2", public_key=TEST_KEY, message=message,
                          signature=response["signature"], response_sha256="0" * 64, terms_sha256="0" * 64)


def test_rotated_key_invalidates_previously_verified_quotes(trust_test_key, monkeypatch):
    real = convert(*live())
    monkeypatch.setattr(ni, "ONE_CLICK_MANAGER_PUBLIC_KEY", ONE_CLICK_KEY)
    with pytest.raises(LiquidityError) as err:
        dataclasses.replace(real)
    assert err.value.code == "X3_LIQ_UNTRUSTED_SIGNER"


# ------------------------------------------------------------ 1Click conversion ----

def test_invalid_signature_is_refused_before_any_number_is_used():
    tampered = copy.deepcopy(CAPTURE)
    tampered["quote"]["minAmountOut"] = "24990000"
    with pytest.raises(LiquidityError) as err:
        convert(captured_request(), tampered)
    assert err.value.code == "X3_LIQ_INVALID_SIGNATURE"
    unsigned = copy.deepcopy(CAPTURE)
    unsigned.pop("signature")
    with pytest.raises(LiquidityError) as err:
        convert(captured_request(), unsigned)
    assert err.value.code == "X3_LIQ_INVALID_SIGNATURE"


def test_signed_live_quote_with_signed_fees_is_executable(trust_test_key):
    quote = convert(*live(withdraw_fee="1000"))
    assert quote.executable and quote.non_executable_reason is None
    assert quote.commitment is q.Commitment.SIGNED_DEPOSIT_QUOTE
    assert quote.expiry_source == "provider:quote.deadline"
    # withdrawFee is not stated to be inside minAmountOut, so it is deducted.
    assert quote.guaranteed_net_output() == 24872939 - 1000


def test_unsigned_app_fees_keep_a_live_quote_non_executable(trust_test_key):
    quote = convert(*live(app_fees=[{"recipient": "someone.near", "fee": 25}]))
    assert quote.executable is False
    assert "app" in quote.non_executable_reason


def test_silent_asset_substitution_is_refused(trust_test_key):
    request, response = live()
    request["destinationAsset"] = "nep141:wrap.near"
    with pytest.raises(LiquidityError) as err:
        convert(request, response)
    assert err.value.code == "X3_LIQ_REQUEST_MISMATCH"
    assert "destinationAsset" in err.value.detail["mismatched"]


def test_provider_minimum_is_a_named_refusal(trust_test_key):
    with pytest.raises(LiquidityError) as err:
        convert(*live(min_amount_in="30000000"))
    assert err.value.code == "X3_LIQ_PROVIDER_MINIMUM"


def test_catalog_decimals_that_disagree_with_signed_formatting_are_refused(trust_test_key):
    tokens = copy.deepcopy(TOKENS)
    for token in tokens:
        if token["assetId"] == NEAR_USDC:
            token["decimals"] = 18
    with pytest.raises(LiquidityError) as err:
        convert(*live(), tokens=tokens)
    assert err.value.code == "X3_LIQ_DECIMALS"


def test_asset_missing_from_catalog_is_refused(trust_test_key):
    tokens = [t for t in TOKENS if t["assetId"] != NEAR_USDC]
    with pytest.raises(LiquidityError) as err:
        convert(*live(), tokens=tokens)
    assert err.value.code == "X3_LIQ_UNKNOWN_ASSET"


def test_fees_larger_than_the_guarantee_underflow_and_are_never_executable(trust_test_key):
    quote = convert(*live(min_out="1000", withdraw_fee="5000"))
    assert quote.executable is False
    assert "underflows" in quote.non_executable_reason
    with pytest.raises(LiquidityError) as err:
        quote.guaranteed_net_output()
    assert err.value.code == "X3_LIQ_UNDERFLOW"
    result = agg.aggregate(agg_request(), [quote])
    assert not result.ranked


# ---------------------------------------------------------------- checked math ----

@pytest.mark.parametrize("value", [True, 1.0, "-1", "01", "1e3", " 1", "1_000", "0x10", None])
def test_parse_uint_refuses_non_canonical_integers(value):
    with pytest.raises(LiquidityError):
        q.parse_uint(value, "x")


def test_parse_uint_bounds():
    assert q.parse_uint(str(q.U256_MAX), "x") == q.U256_MAX
    with pytest.raises(LiquidityError):
        q.parse_uint(q.U256_MAX + 1, "x")
    with pytest.raises(LiquidityError):
        q.parse_uint(str(q.U128_MAX + 1), "x", maximum=q.U128_MAX)


def test_rescale_is_exact_or_refuses():
    assert q.rescale(25_000_000, 6, 24) == 25 * 10**24
    assert q.rescale(25 * 10**24, 24, 6) == 25_000_000
    with pytest.raises(LiquidityError) as err:
        q.rescale(25 * 10**24 + 1, 24, 6)
    assert err.value.code == "X3_LIQ_PRECISION_LOSS"
    with pytest.raises(LiquidityError) as err:
        q.rescale(q.U256_MAX, 0, 1)
    assert err.value.code == "X3_LIQ_OVERFLOW"
    with pytest.raises(LiquidityError):
        q.rescale(1, True, 6)


def test_formatted_amounts_convert_exactly():
    assert q.formatted_to_units("24.997929", 6, "f") == 24997929
    assert q.formatted_to_units("25.0", 6, "f") == 25_000_000
    assert q.formatted_to_units("25", 0, "f") == 25
    for bad in ("24.9979291", "2.5e1", "-1", "1,000", ".5"):
        with pytest.raises(LiquidityError):
            q.formatted_to_units(bad, 6, "f")


def test_checked_sub_underflow():
    assert q.checked_sub(5, 5, "x") == 0
    with pytest.raises(LiquidityError) as err:
        q.checked_sub(4, 5, "x")
    assert err.value.code == "X3_LIQ_UNDERFLOW"


# ------------------------------------------------------- task 5: aggregation ----

def test_ranks_by_guaranteed_net_output_not_quoted_price(trust_test_key):
    flashy = convert(*live(amount_out="25100000", min_out="24800000", correlation="flashy"))
    steady = convert(*live(amount_out="24950000", min_out="24900000", withdraw_fee="10000", correlation="steady"))
    plain = convert(*live(amount_out="24950000", min_out="24880000", correlation="plain"))
    result = agg.aggregate(agg_request(), [flashy, steady, plain])
    assert [r.quote.provenance.reference for r in result.ranked] == ["steady", "plain", "flashy"]
    assert [r.guaranteed_net_output for r in result.ranked] == [24890000, 24880000, 24800000]
    assert result.best_executable.quote is steady


def test_equal_net_outputs_tie_break_deterministically(trust_test_key):
    a = convert(*live(correlation="a"))
    b = convert(*live(correlation="b"))
    first = agg.aggregate(agg_request(), [a, b]).ranked
    second = agg.aggregate(agg_request(), [b, a]).ranked
    assert [r.quote.terms_sha256 for r in first] == [r.quote.terms_sha256 for r in second]


def test_mismatched_chain_asset_decimals_and_amount_are_rejected(trust_test_key):
    intents_delivery = convert(*live(recipient_type="INTENTS", correlation="intents"))
    assert intents_delivery.asset_out.network == "near-intents"
    good = convert(*live(correlation="good"))
    cases = {
        "X3_LIQ_NETWORK_MISMATCH": agg_request(),
        "X3_LIQ_ASSET_MISMATCH": agg_request(asset_out=q.AssetRef("near", "nep141:wrap.near", 6)),
        "X3_LIQ_DECIMALS_MISMATCH": agg_request(asset_out=q.AssetRef("near", NEAR_USDC, 18)),
        "X3_LIQ_AMOUNT_MISMATCH": agg_request(amount_in=25_000_001),
    }
    result = agg.aggregate(cases["X3_LIQ_NETWORK_MISMATCH"], [intents_delivery])
    assert [r.code for r in result.rejected] == ["X3_LIQ_NETWORK_MISMATCH"] and not result.ranked
    for code in ("X3_LIQ_ASSET_MISMATCH", "X3_LIQ_DECIMALS_MISMATCH", "X3_LIQ_AMOUNT_MISMATCH"):
        result = agg.aggregate(cases[code], [good])
        assert [r.code for r in result.rejected] == [code] and not result.ranked


def test_stale_and_future_dated_quotes_are_rejected(trust_test_key):
    quote = convert(*live())
    stale = agg.aggregate(agg_request(now=quote.issued_at + 301, min_remaining_s=0), [quote])
    assert [r.code for r in stale.rejected] == ["X3_LIQ_STALE"]
    future = agg.aggregate(agg_request(now=quote.issued_at - 6), [quote])
    assert [r.code for r in future.rejected] == ["X3_LIQ_STALE"]


def test_quote_expiry_race(trust_test_key):
    # Deadline is 600 s after NOW; with 600 s needed to act it is already too late to rank.
    quote = convert(*live(deadline="2026-11-03T04:06:50.000Z"))
    assert quote.expires_at - NOW == 600
    late = agg.aggregate(agg_request(min_remaining_s=601), [quote])
    assert [r.code for r in late.rejected] == ["X3_LIQ_EXPIRY_RACE"]
    # Ranked while fresh, but the clock passes the window before selection: refused, not used.
    result = agg.aggregate(agg_request(min_remaining_s=60), [quote])
    assert result.best_executable is not None
    with pytest.raises(LiquidityError) as err:
        agg.select_executable(result, now=quote.expires_at - 59, ledger=agg.QuoteLedger())
    assert err.value.code == "X3_LIQ_EXPIRY_RACE"
    expired = agg.aggregate(agg_request(now=quote.expires_at, min_remaining_s=0, max_age_s=10_000), [quote])
    assert [r.code for r in expired.rejected] == ["X3_LIQ_EXPIRED"]


def test_duplicate_inventory_is_rejected_entirely(trust_test_key):
    once = convert(*live(correlation="same"))
    twice = convert(*live(correlation="same", min_out="24880000"))
    other = convert(*live(correlation="other"))
    result = agg.aggregate(agg_request(), [once, twice, other])
    assert [r.code for r in result.rejected] == ["X3_LIQ_DUPLICATE_INVENTORY"] * 2
    assert [r.quote.provenance.reference for r in result.ranked] == ["other"]


def test_one_signed_quote_cannot_back_two_orders(trust_test_key):
    quote = convert(*live())
    result = agg.aggregate(agg_request(), [quote])
    ledger = agg.QuoteLedger()
    assert agg.select_executable(result, now=NOW, ledger=ledger) is quote
    with pytest.raises(LiquidityError) as err:
        agg.select_executable(result, now=NOW, ledger=ledger)
    assert err.value.code == "X3_LIQ_OVERBOOKED"


def test_no_liquidity_and_observations_are_never_selected():
    with pytest.raises(LiquidityError) as err:
        agg.select_executable(agg.aggregate(agg_request(), []), now=NOW, ledger=agg.QuoteLedger())
    assert err.value.code == "X3_LIQ_NO_LIQUIDITY"
    dry = convert(captured_request(), CAPTURE)
    result = agg.aggregate(agg_request(), [dry])
    assert not result.ranked and len(result.observations) == 1
    assert result.observations[0].label.startswith("observation-only")
    with pytest.raises(LiquidityError) as err:
        agg.select_executable(result, now=NOW, ledger=agg.QuoteLedger())
    assert err.value.code == "X3_LIQ_NO_LIQUIDITY"


def test_rate_limits_and_provider_errors_are_isolated_per_provider(trust_test_key):
    clock = [0.0]
    limiter = agg.ProviderRateLimiter({"near-intents-1click": (1, 60), "rhea-v2": (5, 60)}, clock=lambda: clock[0])
    request, response = live()
    calls = []

    def oneclick_source():
        calls.append("1click")
        return convert(request, response)

    def rhea_rate_limited():
        raise ni.NearIntentsError("X3_NEAR_HTTP", "POST /v0/quote returned 429", "/v0/quote", {"status": 429})

    quotes, rejected = agg.collect_quotes({"near-intents-1click": oneclick_source, "rhea-v2": rhea_rate_limited},
                                          limiter=limiter)
    assert len(quotes) == 1 and [r.code for r in rejected] == ["X3_LIQ_RATE_LIMITED"]
    quotes, rejected = agg.collect_quotes({"near-intents-1click": oneclick_source}, limiter=limiter)
    assert not quotes and [r.code for r in rejected] == ["X3_LIQ_RATE_LIMITED"]
    assert calls == ["1click"]  # the exhausted budget stopped the call; nothing was substituted
    clock[0] = 60.0
    quotes, _ = agg.collect_quotes({"near-intents-1click": oneclick_source}, limiter=limiter)
    assert len(quotes) == 1
    _, rejected = agg.collect_quotes({"lifi": oneclick_source}, limiter=limiter)
    assert [r.code for r in rejected] == ["X3_LIQ_RATE_LIMITED"]  # no budget configured


def test_provider_minimum_error_and_cross_provider_answers_are_isolated(trust_test_key):
    limiter = agg.ProviderRateLimiter({"near-intents-1click": (5, 60), "rhea-v2": (5, 60)}, clock=lambda: 0.0)
    request, response = live()

    def minimum():
        raise ni.NearIntentsError("X3_NEAR_PROVIDER_MINIMUM", "temporary $1000 minimum", "/v0/quote", {"status": 400})

    quotes, rejected = agg.collect_quotes({"near-intents-1click": minimum,
                                           "rhea-v2": lambda: convert(request, response)}, limiter=limiter)
    assert not quotes
    assert sorted(r.code for r in rejected) == ["X3_LIQ_PROVIDER_MINIMUM", "X3_LIQ_PROVIDER_MISMATCH"]


def test_signed_quote_endpoint_never_contacts_the_network(trust_test_key):
    request, response = live()

    class Client:
        base_url = "https://1click.chaindefuser.com"

        def quote(self, sent):
            assert sent == request
            return response

    assert oneclick.fetch_oneclick_quote(Client(), request, TOKENS, now=NOW).executable


def test_fee_in_another_asset_cannot_be_netted_against_output():
    """A fee paid on top in the input (or any third) asset has no output-asset value without
    a price, so no net output is produced and the quote cannot rank."""
    fee = q.FeeComponent(kind="network_gas", charge=q.FeeCharge.ADDITIONAL, asset=ASSET_IN.key, amount=1,
                         provider_value=None, source_field="constructed.gas", authenticated=True)
    provenance = q.Provenance("provider_response", "https://provider.example/quote", "a" * 64, NOW, "constructed")
    fields = dict(provider_id="constructed", asset_in=ASSET_IN, asset_out=ASSET_OUT, amount_in=25_000_000,
                  amount_out=24_000_000, min_amount_out=0, issued_at=NOW, expires_at=NOW + 600,
                  expiry_source="constructed", commitment=q.Commitment.CHAIN_OBSERVATION, fees=(fee,),
                  provenance=provenance, request_hash="b" * 64)
    terms = q.quote_terms_sha256(**fields)
    status = q.unsigned_status("constructed", "a" * 64, terms, "constructed test fixture")
    quote = q.ProviderQuote(schema_version=q.SCHEMA_VERSION, **fields, signature=status, executable=False,
                            non_executable_reason="constructed")
    assert quote.expected_net_output() is None
    result = agg.aggregate(agg_request(), [quote])
    assert [r.code for r in result.rejected] == [] and result.observations[0].expected_net_output is None
