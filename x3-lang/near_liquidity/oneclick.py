"""NEAR Intents 1Click quote -> `ProviderQuote`, reusing the existing signed-quote path.

This module does not re-implement 1Click signing: the signed payload and its hash come from
`near_intents.signed_quote_payload` / `oneclick_quote_hash`, the trust anchor is
`near_intents.ONE_CLICK_MANAGER_PUBLIC_KEY`, and the request-echo comparison is
`near_intents._same` over `near_intents._ECHO_FIELDS`. It adds only what a liquidity
comparison needs: chain-qualified assets, exact decimals from the provider's token catalog,
the fee breakdown and an explicit executable decision.

Field meanings are from the 1Click API reference
(https://docs.near-intents.org/api-reference/oneclick/request-a-swap-quote, read 2026-11-13):
`minAmountOut` is the "Minimum output amount after slippage is applied"; `withdrawFee` is
charged "for withdrawing assets to the recipient in the smallest unit of the destination
asset"; `refundFee` is charged "for refunding assets ... in the smallest unit of the origin
asset". The reference does not say whether `amountOut` is already net of `withdrawFee`, so
this adapter treats it as an additional deduction (`FeeCharge.UNKNOWN`). App fees on
EXACT_INPUT swaps are "deducted from your input amount before the swap"
(https://docs.near-intents.org/near-intents/integration/distribution-channels/1click-app-fees-calculation);
`appFees` is echoed but not signed, so a quote carrying them is never executable.

Nothing here requests a deposit, signs or submits anything.
"""
from __future__ import annotations

from datetime import datetime
from typing import Any, List, Mapping, Optional

import near_intents as ni
import oneclick_catalog

from .quote import (
    SCHEMA_VERSION, AssetRef, Commitment, FeeCharge, FeeComponent, LiquidityError, ProviderQuote, Provenance,
    _issue_verified, canonical_json, formatted_to_units, parse_uint, quote_terms_sha256, sha256_hex,
)

PROVIDER_ID = "near-intents-1click"
#: A 1Click asset delivered into (or taken from) the intents.near ledger is not the same
#: asset as the token on its home chain, even with the same asset ID.
INTENTS_NETWORK = "near-intents"

_DEPOSIT_TYPES = {"ORIGIN_CHAIN", "INTENTS"}
_RECIPIENT_TYPES = {"DESTINATION_CHAIN", "INTENTS"}


def _unix(text: Any, field_name: str) -> int:
    try:
        moment: datetime = ni.parse_timestamp(text)
    except ni.NearIntentsError:
        raise LiquidityError("X3_LIQ_MALFORMED", f"{field_name} is not a timestamp: {text!r}") from None
    return int(moment.timestamp())


def _asset(token: Mapping[str, Any], network_override: Optional[str]) -> AssetRef:
    network = network_override or str(token["blockchain"]).lower()
    return AssetRef(network=network, asset_id=token["assetId"], decimals=token["decimals"])


def quote_from_oneclick(request: Mapping[str, Any], response: Mapping[str, Any], tokens: List[Mapping[str, Any]], *,
                        now: int, endpoint: str = ni.DEFAULT_BASE_URL + "/v0/quote") -> ProviderQuote:
    """Convert a 1Click `/v0/quote` response for `request` into a `ProviderQuote`.

    Refuses (raises `LiquidityError`) on a signature that does not verify under the pinned
    key, on any echoed request field that differs from what was sent (no silent route or
    asset substitution), on an asset missing from the catalog, on amounts or decimals that
    disagree, and on an input below the provider's stated `minAmountIn`.
    """
    if not isinstance(request, Mapping) or not isinstance(response, Mapping):
        raise LiquidityError("X3_LIQ_MALFORMED", "request and response must be JSON objects")
    response = dict(response)
    response_sha256 = sha256_hex(canonical_json(response))
    request_hash = sha256_hex(canonical_json(dict(request)))

    # 1. Signature first, under the pinned key, before any other field is trusted.
    try:
        message = ni.oneclick_quote_hash(response).encode("utf-8")
    except ni.QuoteSignatureFormatError as exc:
        raise LiquidityError("X3_LIQ_INVALID_SIGNATURE", f"quote cannot be serialized for verification: {exc}") from None
    signature = response.get("signature")
    signer = ni.ONE_CLICK_MANAGER_PUBLIC_KEY
    valid, reason = ni.check_oneclick_quote_signature(response, signer)
    if not valid:
        raise LiquidityError("X3_LIQ_INVALID_SIGNATURE", reason)

    # 2. The signed request must be the request we sent: same assets, amount, recipient, type.
    echoed = response.get("quoteRequest")
    if not isinstance(echoed, dict):
        raise LiquidityError("X3_LIQ_MALFORMED", "response has no quoteRequest echo")
    mismatched = [f for f in ni._ECHO_FIELDS if not ni._same(f, request.get(f), echoed.get(f))]
    if mismatched:
        raise LiquidityError("X3_LIQ_REQUEST_MISMATCH",
                             f"provider answered a different request: {', '.join(mismatched)}",
                             {"mismatched": mismatched})
    if request.get("swapType") != "EXACT_INPUT":
        raise LiquidityError("X3_LIQ_UNSUPPORTED", "only EXACT_INPUT quotes aggregate by identical input quantity")
    if request.get("depositType") not in _DEPOSIT_TYPES or request.get("recipientType") not in _RECIPIENT_TYPES:
        raise LiquidityError("X3_LIQ_UNSUPPORTED", "unknown depositType or recipientType")

    # 3. Exact decimals from the provider's own catalog.
    try:
        catalog = oneclick_catalog.catalog(tokens)
    except ValueError as exc:
        raise LiquidityError("X3_LIQ_CATALOG", str(exc)) from None
    origin = catalog.get(request.get("originAsset"))
    destination = catalog.get(request.get("destinationAsset"))
    if origin is None or destination is None:
        raise LiquidityError("X3_LIQ_UNKNOWN_ASSET", "origin or destination asset is not in the 1Click catalog")
    asset_in = _asset(origin, INTENTS_NETWORK if request["depositType"] == "INTENTS" else None)
    asset_out = _asset(destination, INTENTS_NETWORK if request["recipientType"] == "INTENTS" else None)

    quote = response.get("quote")
    if not isinstance(quote, dict):
        raise LiquidityError("X3_LIQ_MALFORMED", "response has no quote object")
    amount_in = parse_uint(quote.get("amountIn"), "quote.amountIn")
    amount_out = parse_uint(quote.get("amountOut"), "quote.amountOut")
    min_amount_out = parse_uint(quote.get("minAmountOut"), "quote.minAmountOut")
    if amount_in != parse_uint(request.get("amount"), "request.amount"):
        raise LiquidityError("X3_LIQ_REQUEST_MISMATCH", "quoted amountIn differs from the requested amount")
    if min_amount_out > amount_out:
        raise LiquidityError("X3_LIQ_MALFORMED", "minAmountOut is above amountOut")
    if quote.get("minAmountIn") is not None:
        provider_minimum = parse_uint(quote["minAmountIn"], "quote.minAmountIn")
        if amount_in < provider_minimum:
            raise LiquidityError("X3_LIQ_PROVIDER_MINIMUM",
                                 f"amountIn {amount_in} is below the provider minimum {provider_minimum}",
                                 {"provider_minimum": str(provider_minimum)})
    # The signed *Formatted fields must agree with the catalog decimals; a disagreement
    # means one of them is wrong, and an amount read at the wrong scale is not refused later.
    for units, formatted, decimals, name in ((amount_in, quote.get("amountInFormatted"), asset_in.decimals, "amountIn"),
                                             (amount_out, quote.get("amountOutFormatted"), asset_out.decimals, "amountOut")):
        if formatted is None:
            continue
        if formatted_to_units(formatted, decimals, f"quote.{name}Formatted") != units:
            raise LiquidityError("X3_LIQ_DECIMALS",
                                 f"quote.{name}Formatted={formatted!r} disagrees with {units} at {decimals} decimals")

    issued_at = _unix(response.get("timestamp"), "timestamp")
    dry = request.get("dry")
    if type(dry) is not bool:
        raise LiquidityError("X3_LIQ_MALFORMED", "request.dry must be a boolean")
    if dry:
        # A dry quote carries no deadline; its usable life is this repository's freshness
        # policy, recorded as such rather than presented as a provider term.
        expires_at = issued_at + ni.MAX_QUOTE_AGE_S
        expiry_source = f"policy:near_intents.MAX_QUOTE_AGE_S={ni.MAX_QUOTE_AGE_S}"
    else:
        if not quote.get("deadline"):
            raise LiquidityError("X3_LIQ_MALFORMED", "live quote carries no deadline")
        expires_at = _unix(quote["deadline"], "quote.deadline")
        expiry_source = "provider:quote.deadline"
    if expires_at <= issued_at:
        raise LiquidityError("X3_LIQ_MALFORMED", "quote expires before it was issued")

    # 4. Fees, each with its field and whether the signature covers it. 1Click signs
    # refundFee/withdrawFee on live quotes only, and only when truthy: a zero is covered by
    # its omission (a changed nonzero value would break the signature), a dry value is not.
    fees: List[FeeComponent] = []
    for key, kind, charge, asset in (("withdrawFee", "withdraw", FeeCharge.UNKNOWN, asset_out),
                                     ("refundFee", "refund", FeeCharge.REFUND_PATH_ONLY, asset_in)):
        if quote.get(key) is None:
            continue
        fees.append(FeeComponent(kind=kind, charge=charge, asset=asset.key,
                                 amount=parse_uint(quote[key], f"quote.{key}"), provider_value=None,
                                 source_field=f"quote.{key}", authenticated=not dry))
    app_fees = echoed.get("appFees")
    if app_fees:
        if not isinstance(app_fees, list):
            raise LiquidityError("X3_LIQ_MALFORMED", "appFees is not a list")
        for index, entry in enumerate(app_fees):
            if not isinstance(entry, dict) or isinstance(entry.get("fee"), bool) or not isinstance(entry.get("fee"), int):
                raise LiquidityError("X3_LIQ_MALFORMED", f"appFees[{index}] is malformed")
            fees.append(FeeComponent(kind="app", charge=FeeCharge.INCLUDED_IN_QUOTE, asset=asset_in.key, amount=None,
                                     provider_value=str(entry["fee"]),
                                     source_field=f"quoteRequest.appFees[{index}].fee", authenticated=False))

    if dry:
        commitment = Commitment.SIGNED_INDICATIVE
    elif quote.get("depositAddress"):
        commitment = Commitment.SIGNED_DEPOSIT_QUOTE
    else:
        raise LiquidityError("X3_LIQ_MALFORMED", "live quote carries no deposit address")

    provenance = Provenance(source="provider_response", endpoint=endpoint, response_sha256=response_sha256,
                            observed_at=now, reference=str(response.get("correlationId") or response.get("timestamp")))
    fee_tuple = tuple(fees)
    terms = quote_terms_sha256(provider_id=PROVIDER_ID, asset_in=asset_in, asset_out=asset_out, amount_in=amount_in,
                               amount_out=amount_out, min_amount_out=min_amount_out, issued_at=issued_at,
                               expires_at=expires_at, expiry_source=expiry_source, commitment=commitment,
                               fees=fee_tuple, provenance=provenance, request_hash=request_hash)
    status = _issue_verified(PROVIDER_ID, public_key=signer, message=message, signature=signature,
                             response_sha256=response_sha256, terms_sha256=terms)

    common = dict(schema_version=SCHEMA_VERSION, provider_id=PROVIDER_ID, asset_in=asset_in, asset_out=asset_out,
                  amount_in=amount_in, amount_out=amount_out, min_amount_out=min_amount_out, issued_at=issued_at,
                  expires_at=expires_at, expiry_source=expiry_source, commitment=commitment, fees=fee_tuple,
                  provenance=provenance, request_hash=request_hash, signature=status)
    probe = ProviderQuote(**common, executable=False, non_executable_reason="evaluating")
    refusal = probe.executable_refusal()
    if refusal is None:
        return ProviderQuote(**common, executable=True, non_executable_reason=None)
    return ProviderQuote(**common, executable=False, non_executable_reason=refusal)


def fetch_oneclick_quote(client: "ni.OneClickClient", request: Mapping[str, Any], tokens: List[Mapping[str, Any]], *,
                         now: int) -> ProviderQuote:
    """Ask 1Click for one quote through the existing client and convert it.

    Provider errors keep their `near_intents` codes (e.g. `X3_NEAR_PROVIDER_MINIMUM`, HTTP
    429) so `ranking.collect_quotes` can isolate them per provider.
    """
    response = client.quote(dict(request))
    return quote_from_oneclick(request, response, tokens, now=now, endpoint=client.base_url + "/v0/quote")


__all__ = ["PROVIDER_ID", "INTENTS_NETWORK", "quote_from_oneclick", "fetch_oneclick_quote"]
