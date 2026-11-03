#!/usr/bin/env python3
"""X3 intents settled through NEAR Intents (the 1Click API).

An X3 `intent` whose route is a single `bridge NearIntents <from> -> <to>` leg is lowered to a
1Click `QuoteRequest`, and the quote that comes back is checked against the program's own
guards **before** a deposit address is ever used. The solver network does the routing; X3
decides whether the result is acceptable.

    intent sol_usdc_to_near {
        from Solana.USDC amount 25000000 receiver <solana address>
        to Near.USDC receiver <near account>
        route { bridge NearIntents Solana.USDC -> Near.USDC min_receive 24800000 }
        require slippage <= 0.5%
        require fees <= 100 bps
        require nonce unused sol_usdc_to_near_001
        timeout 10m refund Solana.USDC
        on_fail rollback
    }

## What each clause becomes

| X3 | 1Click |
|---|---|
| `from <chain>.<asset> amount N` | `originAsset` (resolved from `/v0/tokens`), `amount` = N base units, `swapType` EXACT_INPUT |
| `to <chain>.<asset> receiver R` | `destinationAsset`, `recipient` R, `recipientType` DESTINATION_CHAIN |
| `require slippage <= x` | `slippageTolerance` in basis points, rounded *down* (never looser than written) |
| `timeout D refund <from asset> [to R]` | `deadline` = now + D, `refundTo` R (default: the `from` receiver), `refundType` ORIGIN_CHAIN |
| `on_fail rollback` / `on_fail refund <from asset>` | 1Click refunds a failed swap to `refundTo` |

## What is checked on the quote (`verify_quote`)

- the echoed `quoteRequest` is the request that was sent;
- `amountIn` is the amount the program spends;
- `minAmountOut` clears the leg's `min_receive` floor;
- the spread between `amountOut` and `minAmountOut` is within the requested slippage;
- `require fees <= N` holds on every path: the quote's effective cost (`amountInUsd` vs
  `amountOutUsd`) and, separately, the `refundFee` a failed swap would be charged;
- a live quote carries a deposit address, a 1Click signature, and a deadline far enough out
  for its own `timeEstimate`.

## What is refused, by code

The adapter fails closed. A program it cannot honour exactly is refused rather than
approximated: more than one route leg (1Click settles one swap; composing it with other legs
would allow partial execution with no rollback), a guard 1Click gives no way to enforce
(`finality`, `proof_complete`, …), a missing slippage bound or timeout, a refund in an asset
other than the one deposited, and `on_fail halt` / `quarantine`, which 1Click cannot express.

## Account preflight

Before any quote is requested, every NEAR account the swap pays out to (the recipient, or the
refund address when the origin is NEAR) is looked up on chain. 1Click answers a quote toward a
NEAR account that does not exist with HTTP 400 `Internal server error`; the preflight refuses
it as `X3_NEAR_ACCOUNT_UNKNOWN`, naming the account. Implicit accounts are not looked up.

## Replay protection

`require nonce unused <id>` is enforced by a local nonce store (`X3_NEAR_NONCE_STORE`, default
`~/.x3/near_intents_nonces.json`). A nonce is consumed when a **live** quote passes every guard,
because that is the moment a deposit address exists for it. Dry quotes never consume one.

## Configuration

- `NEAR_INTENTS_JWT` — 1Click API key. Without it the unauthenticated fee schedule applies
  (an extra 0.25% per the 1Click docs), which `require fees` will see.
- `NEAR_INTENTS_BASE_URL` — defaults to `https://1click.chaindefuser.com`.
- `NEAR_INTENTS_REFERRAL` — optional lowercase `referral` identifier.
- `NEAR_RPC_URL` — NEAR JSON-RPC used to check that NEAR receivers exist before quoting;
  defaults to FastNEAR's free endpoint (`rpc.mainnet.near.org` is deprecated).

This module never signs or sends the deposit. `quote --live` returns the deposit address and
the exact amount; the deposit is made from the sender's wallet, after which `submit-deposit`
and `status` track the swap to a terminal X3 outcome.
"""
import argparse
import contextlib
import hashlib
import importlib.util
import json
import os
import re
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from datetime import datetime, timedelta, timezone
from decimal import ROUND_FLOOR, Decimal, InvalidOperation
from typing import Any, Callable, Dict, List, Optional, Tuple

_HERE = os.path.dirname(os.path.abspath(__file__))
if _HERE not in sys.path:
    # `typechecker.py` and `runner.py` import `numeric` as a top-level module.
    sys.path.insert(0, _HERE)


def _load(name: str):
    path = os.path.join(_HERE, f"{name}.py")
    spec = importlib.util.spec_from_file_location(f"x3lang_{name}", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


registry = _load("registry")

DEFAULT_BASE_URL = "https://1click.chaindefuser.com"
DEFAULT_NONCE_STORE = os.path.join(os.path.expanduser("~"), ".x3", "near_intents_nonces.json")
RECEIPT_KIND = "x3.near_intents.v1"

# X3 chain name -> 1Click `blockchain` value (`GET /v0/tokens`).
CHAIN_TO_BLOCKCHAIN = {
    "near": "near",
    "ethereum": "eth",
    "solana": "sol",
    "arbitrum": "arb",
    "polygon": "pol",
    "bitcoin": "btc",
}

# X3 asset names that 1Click lists under another symbol. NEAR is deposited and delivered as
# the wrapped NEP-141 token.
SYMBOL_ALIASES = {("near", "NEAR"): "wNEAR"}

# Guards this adapter enforces. Anything else in `requires` is refused by name.
ENFORCED_GUARDS = {"slippage", "fees", "nonce"}

STATUS_OUTCOMES = {
    "PENDING_DEPOSIT": "awaiting_deposit",
    "KNOWN_DEPOSIT_TX": "deposit_seen",
    "PROCESSING": "in_flight",
    "INCOMPLETE_DEPOSIT": "incomplete_deposit",
    "SUCCESS": "settled",
    "REFUNDED": "refunded",
    "FAILED": "failed",
}
TERMINAL_STATUSES = {"SUCCESS", "REFUNDED", "FAILED"}
# Exit codes for `status`, so a script can branch on the outcome.
OUTCOME_EXIT_CODES = {"settled": 0, "refunded": 2, "failed": 3}
EXIT_NOT_TERMINAL = 4


class NearIntentsError(Exception):
    def __init__(self, code: str, message: str, field: Optional[str] = None, detail: Any = None):
        super().__init__(message)
        self.code = code
        self.message = message
        self.field = field
        self.detail = detail

    def to_dict(self) -> Dict[str, Any]:
        out: Dict[str, Any] = {"code": self.code, "message": self.message}
        if self.field:
            out["field"] = self.field
        if self.detail is not None:
            out["detail"] = self.detail
        return out


# --------------------------------------------------------------------------------------------
# Time
# --------------------------------------------------------------------------------------------

_DURATION_RE = re.compile(r"(\d+)(ms|s|m|h)")
_DURATION_UNITS = {"ms": 0.001, "s": 1, "m": 60, "h": 3600}


def parse_duration(text: str) -> timedelta:
    match = _DURATION_RE.fullmatch(str(text).strip())
    if not match:
        raise NearIntentsError(
            "X3_NEAR_TIMEOUT_UNIT",
            f"timeout {text!r} is not a wall-clock duration (<n>ms|s|m|h); 1Click deadlines are "
            "timestamps, so a block-count timeout has no exact equivalent",
            "policies.timeout.duration",
        )
    seconds = int(match.group(1)) * _DURATION_UNITS[match.group(2)]
    if seconds <= 0:
        raise NearIntentsError("X3_NEAR_TIMEOUT_UNIT", "timeout must be positive", "policies.timeout.duration")
    return timedelta(seconds=seconds)


def format_timestamp(moment: datetime) -> str:
    return moment.astimezone(timezone.utc).isoformat(timespec="milliseconds").replace("+00:00", "Z")


def parse_timestamp(text: str) -> datetime:
    value = str(text).strip()
    if value.endswith("Z"):
        value = value[:-1] + "+00:00"
    # Python < 3.11 accepts only 3 or 6 fractional digits; normalize to 6.
    match = re.fullmatch(r"(.*T\d{2}:\d{2}:\d{2})(\.\d+)?([+-]\d{2}:\d{2})?", value)
    if not match:
        raise NearIntentsError("X3_NEAR_TIMESTAMP", f"unreadable timestamp {text!r}")
    base, frac, tz = match.groups()
    frac = (frac or ".0")[1:7].ljust(6, "0")
    parsed = datetime.fromisoformat(f"{base}.{frac}{tz or '+00:00'}")
    return parsed.astimezone(timezone.utc)


# --------------------------------------------------------------------------------------------
# Numbers
# --------------------------------------------------------------------------------------------

def _decimal(value: Any, field: str) -> Decimal:
    try:
        result = Decimal(str(value).strip())
    except (InvalidOperation, ValueError):
        raise NearIntentsError("X3_NEAR_NUMBER", f"{field} is not a number: {value!r}", field)
    if not result.is_finite():
        raise NearIntentsError("X3_NEAR_NUMBER", f"{field} is not finite: {value!r}", field)
    return result


def base_units(value: Any, field: str) -> int:
    """An integer amount in the asset's smallest unit. Fractions are refused, never rounded."""
    text = str(value).strip().replace("_", "")
    if not re.fullmatch(r"\d+", text):
        raise NearIntentsError(
            "X3_NEAR_AMOUNT",
            f"{field} must be an integer in base units (1Click refuses decimals like '0.01'), "
            f"got {value!r}",
            field,
        )
    amount = int(text)
    if amount <= 0:
        raise NearIntentsError("X3_NEAR_AMOUNT", f"{field} must be positive", field)
    return amount


def to_bps(value: Any, field: str) -> Decimal:
    """`0.5%`, `50 bps`, `50bps`, or a bare fraction (`0.005`) -> basis points."""
    text = str(value).strip().lower().replace(" ", "")
    if text.endswith("bps"):
        bps = _decimal(text[:-3], field)
    elif text.endswith("%"):
        bps = _decimal(text[:-1], field) * 100
    else:
        fraction = _decimal(text, field)
        if fraction >= 1:
            raise NearIntentsError(
                "X3_NEAR_BOUND_UNIT",
                f"{field} {value!r} has no unit; write it as a percent (0.5%) or in bps (50 bps)",
                field,
            )
        bps = fraction * 10000
    if bps < 0:
        raise NearIntentsError("X3_NEAR_BOUND_UNIT", f"{field} must not be negative", field)
    return bps


def _upper_bound_bps(req: Dict[str, Any], field: str) -> int:
    """The largest whole bps value the guard allows. Rounds toward the stricter side."""
    op = req.get("op")
    if op not in {"<=", "<"}:
        raise NearIntentsError(
            "X3_NEAR_GUARD_OPERATOR",
            f"{field} must be an upper bound (<= or <), got {op!r}",
            field,
        )
    bps = to_bps(req.get("value"), field)
    whole = int(bps.to_integral_value(rounding=ROUND_FLOOR))
    if op == "<" and Decimal(whole) == bps:
        whole -= 1
    if whole < 0:
        raise NearIntentsError("X3_NEAR_BOUND_UNIT", f"{field} allows no value at all", field)
    return whole


# --------------------------------------------------------------------------------------------
# Lowering: X3 intent -> NEAR Intents leg
# --------------------------------------------------------------------------------------------

def _endpoint(intent: Dict[str, Any], key: str) -> Dict[str, Any]:
    endpoint = intent.get(key)
    if not isinstance(endpoint, dict) or not endpoint.get("chain") or not endpoint.get("asset"):
        raise NearIntentsError("X3_NEAR_ENDPOINT", f"intent has no `{key}` chain.asset", key)
    chain = registry.normalize_chain(endpoint["chain"])
    if chain not in CHAIN_TO_BLOCKCHAIN:
        raise NearIntentsError(
            "X3_NEAR_CHAIN_UNSUPPORTED",
            f"{key} chain {endpoint['chain']!r} has no NEAR Intents mapping here "
            f"(supported: {', '.join(sorted(CHAIN_TO_BLOCKCHAIN))})",
            f"{key}.chain",
        )
    return {**endpoint, "chain": chain}


def lower(intent: Dict[str, Any], *, now: Optional[datetime] = None) -> Dict[str, Any]:
    """Every decision the program makes about the swap, before any asset is resolved."""
    now = now or datetime.now(timezone.utc)
    route = intent.get("route") or intent.get("path") or []
    if len(route) != 1 or route[0].get("type") != "bridge" or str(route[0].get("via", "")).lower() not in registry.NEAR_INTENTS_VIAS:
        raise NearIntentsError(
            "X3_NEAR_ROUTE_SHAPE",
            "a NEAR Intents program has exactly one route leg, `bridge NearIntents <from> -> <to>`: "
            "1Click settles one swap, and composing it with other legs would allow partial "
            "execution with no rollback",
            "route",
        )
    leg = route[0]
    source = _endpoint(intent, "from")
    dest = _endpoint(intent, "to")

    for ref_key, endpoint, label in (("from_ref", source, "from"), ("to_ref", dest, "to")):
        ref = leg.get(ref_key) or {}
        if registry.normalize_chain(ref.get("chain")) != endpoint["chain"] or ref.get("asset") != endpoint["asset"]:
            raise NearIntentsError(
                "X3_NEAR_ROUTE_ENDPOINTS",
                f"the NearIntents leg's {label} side ({ref.get('chain')}.{ref.get('asset')}) is not "
                f"the intent's `{label}` ({endpoint['chain']}.{endpoint['asset']})",
                f"route[0].{ref_key}",
            )

    if source.get("amount") is None:
        raise NearIntentsError("X3_NEAR_AMOUNT", "`from` must state an amount", "from.amount")
    amount = base_units(source["amount"], "from.amount")
    if leg.get("amount") is not None and base_units(leg["amount"], "route[0].amount") != amount:
        raise NearIntentsError(
            "X3_NEAR_AMOUNT",
            "the leg's amount differs from the intent's `from` amount; the leg spends all of it",
            "route[0].amount",
        )
    min_receive = base_units(leg["min_receive"], "route[0].min_receive") if leg.get("min_receive") is not None else None

    recipient = leg.get("receiver") or dest.get("receiver")
    if not recipient or recipient == "sender":
        raise NearIntentsError("X3_NEAR_RECIPIENT", "`to` must name a receiver address", "to.receiver")
    if not registry.is_valid_receiver(dest["chain"], recipient):
        raise NearIntentsError("X3_NEAR_RECIPIENT", f"{recipient!r} is not a {dest['chain']} address", "to.receiver")

    # Guards.
    requires = intent.get("requires") or []
    unenforceable = sorted({r.get("kind") for r in requires if r.get("kind") not in ENFORCED_GUARDS})
    if unenforceable:
        raise NearIntentsError(
            "X3_NEAR_GUARD_UNENFORCEABLE",
            f"NEAR Intents gives this adapter no way to enforce: {', '.join(map(str, unenforceable))}. "
            f"It enforces {', '.join(sorted(ENFORCED_GUARDS))}; remove the others or settle this "
            "intent through an adapter that can prove them",
            "requires",
        )
    slippage = [_upper_bound_bps(r, "require slippage") for r in requires if r["kind"] == "slippage"]
    if not slippage:
        raise NearIntentsError(
            "X3_NEAR_SLIPPAGE_REQUIRED",
            "a NEAR Intents swap needs `require slippage <= <bound>`; 1Click requires a tolerance "
            "and this adapter will not pick one for you",
            "requires",
        )
    fee_caps = [_upper_bound_bps(r, "require fees") for r in requires if r["kind"] == "fees"]
    nonces = []
    for r in requires:
        if r["kind"] == "nonce":
            words = str(r.get("value", "")).split()
            if words and words[0] == "unused":
                words = words[1:]
            if len(words) != 1:
                raise NearIntentsError("X3_NEAR_NONCE", "expected `require nonce unused <id>`", "requires.nonce")
            nonces.append(words[0])
    if len(set(nonces)) > 1:
        raise NearIntentsError("X3_NEAR_NONCE", "an intent carries one nonce", "requires.nonce")

    # Timeout and refund.
    policies = intent.get("policies") or {}
    timeout = policies.get("timeout")
    if not isinstance(timeout, dict) or not timeout.get("duration"):
        raise NearIntentsError(
            "X3_NEAR_TIMEOUT_REQUIRED",
            "a NEAR Intents swap needs `timeout <duration> refund <from asset>`: the deadline is "
            "when 1Click starts refunding, and it is the program's to state",
            "policies.timeout",
        )
    window = parse_duration(timeout["duration"])
    refund = timeout.get("action") or {"type": "refund", "chain": source["chain"], "asset": source["asset"], "to": "sender"}
    on_fail = policies.get("on_fail")
    if isinstance(on_fail, dict):
        if on_fail.get("type") in {"halt", "quarantine"}:
            raise NearIntentsError(
                "X3_NEAR_ON_FAIL_UNSUPPORTED",
                f"on_fail {on_fail['type']} cannot be expressed: 1Click refunds a failed swap to "
                "the refund address. Use `on_fail rollback` or `on_fail refund <from asset>`",
                "policies.on_fail",
            )
        if on_fail.get("type") == "refund":
            refund_actions = [refund, on_fail]
        else:
            refund_actions = [refund]
    else:
        refund_actions = [refund]

    refund_to: Optional[str] = None
    for action in refund_actions:
        if registry.normalize_chain(action.get("chain")) != source["chain"] or action.get("asset") != source["asset"]:
            raise NearIntentsError(
                "X3_NEAR_REFUND_ASSET",
                f"1Click refunds the deposited asset ({source['chain']}.{source['asset']}) on the "
                f"origin chain, not {action.get('chain')}.{action.get('asset')}",
                "policies",
            )
        target = action.get("to") or "sender"
        target = source.get("receiver") if target == "sender" else target
        if not target:
            raise NearIntentsError(
                "X3_NEAR_REFUND_TO",
                "the refund goes to `sender`, and `from` names no receiver to refund",
                "from.receiver",
            )
        if not registry.is_valid_receiver(source["chain"], target):
            raise NearIntentsError("X3_NEAR_REFUND_TO", f"{target!r} is not a {source['chain']} address", "policies")
        if refund_to is not None and refund_to != target:
            raise NearIntentsError("X3_NEAR_REFUND_TO", "timeout and on_fail refund to different addresses; 1Click takes one", "policies")
        refund_to = target

    return {
        "intent": intent.get("intent"),
        "origin": {"chain": source["chain"], "asset": source["asset"]},
        "destination": {"chain": dest["chain"], "asset": dest["asset"]},
        "amount": amount,
        "min_receive": min_receive,
        "recipient": recipient,
        "refund_to": refund_to,
        "slippage_bps": min(slippage),
        "fee_cap_bps": min(fee_caps) if fee_caps else None,
        "nonce": nonces[0] if nonces else None,
        "deadline": format_timestamp(now + window),
    }


def resolve_asset(tokens: List[Dict[str, Any]], chain: str, asset: str) -> Dict[str, Any]:
    blockchain = CHAIN_TO_BLOCKCHAIN[chain]
    symbol = SYMBOL_ALIASES.get((chain, asset), asset)
    matches = [t for t in tokens if t.get("blockchain") == blockchain and str(t.get("symbol", "")).upper() == symbol.upper()]
    if not matches:
        raise NearIntentsError(
            "X3_NEAR_ASSET_UNSUPPORTED",
            f"1Click lists no {symbol} on {blockchain}",
            f"{chain}.{asset}",
        )
    if len(matches) > 1:
        raise NearIntentsError(
            "X3_NEAR_ASSET_AMBIGUOUS",
            f"1Click lists {len(matches)} tokens with symbol {symbol} on {blockchain}; refusing to guess",
            f"{chain}.{asset}",
            [m.get("assetId") for m in matches],
        )
    return matches[0]


def build_quote_request(leg: Dict[str, Any], tokens: List[Dict[str, Any]], *, dry: bool, referral: Optional[str] = None) -> Dict[str, Any]:
    origin = resolve_asset(tokens, leg["origin"]["chain"], leg["origin"]["asset"])
    destination = resolve_asset(tokens, leg["destination"]["chain"], leg["destination"]["asset"])
    request: Dict[str, Any] = {
        "dry": dry,
        "swapType": "EXACT_INPUT",
        "slippageTolerance": leg["slippage_bps"],
        "originAsset": origin["assetId"],
        "depositType": "ORIGIN_CHAIN",
        "destinationAsset": destination["assetId"],
        "amount": str(leg["amount"]),
        "refundTo": leg["refund_to"],
        "refundType": "ORIGIN_CHAIN",
        "recipient": leg["recipient"],
        "recipientType": "DESTINATION_CHAIN",
        "deadline": leg["deadline"],
    }
    if referral:
        request["referral"] = referral.lower()
    return request


# --------------------------------------------------------------------------------------------
# Quote verification
# --------------------------------------------------------------------------------------------

_ECHO_FIELDS = (
    "swapType", "slippageTolerance", "originAsset", "depositType", "destinationAsset",
    "amount", "refundTo", "refundType", "recipient", "recipientType", "deadline",
)


def _same(field: str, sent: Any, echoed: Any) -> bool:
    if field == "deadline":
        try:
            return parse_timestamp(sent) == parse_timestamp(echoed)
        except NearIntentsError:
            return False
    if field in {"amount", "slippageTolerance"}:
        try:
            return Decimal(str(sent)) == Decimal(str(echoed))
        except InvalidOperation:
            return False
    return sent == echoed


def verify_quote(leg: Dict[str, Any], request: Dict[str, Any], response: Dict[str, Any], *, now: Optional[datetime] = None) -> List[Dict[str, Any]]:
    """Check a 1Click quote against the program. Raises with every result if any guard fails."""
    now = now or datetime.now(timezone.utc)
    results: List[Dict[str, Any]] = []

    def record(guard: str, ok: bool, **detail: Any) -> None:
        results.append({"guard": guard, "ok": bool(ok), **detail})

    quote = response.get("quote")
    if not isinstance(quote, dict):
        raise NearIntentsError("X3_NEAR_QUOTE_MALFORMED", "quote response has no `quote` object", "quote")

    echoed = response.get("quoteRequest") or {}
    mismatched = [f for f in _ECHO_FIELDS if not _same(f, request.get(f), echoed.get(f))]
    record("quote_request_echo", not mismatched, mismatched=mismatched)

    try:
        amount_in = int(quote["amountIn"])
        amount_out = int(quote["amountOut"])
        min_out = int(quote["minAmountOut"])
    except (KeyError, TypeError, ValueError):
        raise NearIntentsError("X3_NEAR_QUOTE_MALFORMED", "quote lacks integer amountIn/amountOut/minAmountOut", "quote")

    record("amount_in", amount_in == leg["amount"], expected=str(leg["amount"]), quoted=str(amount_in))

    if leg.get("min_receive") is not None:
        record("min_receive", min_out >= leg["min_receive"], floor=str(leg["min_receive"]), guaranteed=str(min_out))

    if amount_out <= 0 or min_out > amount_out:
        record("slippage", False, reason="minAmountOut is above amountOut or amountOut is zero",
               amount_out=str(amount_out), min_amount_out=str(min_out))
    else:
        spread_bps = Decimal(amount_out - min_out) * 10000 / Decimal(amount_out)
        # One bps of headroom covers the solver rounding minAmountOut down to a whole unit.
        record("slippage", spread_bps <= leg["slippage_bps"] + 1,
               bound_bps=leg["slippage_bps"], quoted_spread_bps=str(spread_bps.quantize(Decimal("0.01"))))

    if leg.get("fee_cap_bps") is not None:
        try:
            usd_in = Decimal(str(quote.get("amountInUsd")))
            usd_out = Decimal(str(quote.get("amountOutUsd")))
        except InvalidOperation:
            usd_in = usd_out = Decimal(0)
        if usd_in <= 0:
            record("fees", False, reason="quote has no USD price for the input, so the cost cannot be measured")
        else:
            cost_bps = max(Decimal(0), (usd_in - usd_out) * 10000 / usd_in)
            record("fees", cost_bps <= leg["fee_cap_bps"], cap_bps=leg["fee_cap_bps"],
                   effective_cost_bps=str(cost_bps.quantize(Decimal("0.01"))),
                   amount_in_usd=str(usd_in), amount_out_usd=str(usd_out))
        # The refund is a path of this intent too, and 1Click charges for it (`refundFee`, in
        # origin base units). `require fees` bounds every path, so a quote whose refund costs
        # more than the cap is refused even though the happy path is cheap. An absent
        # `refundFee` is an unknown cost, not a zero one.
        refund_fee = quote.get("refundFee")
        if refund_fee is None:
            record("refund_fee", False, reason="quote states no refundFee, so the refund path's cost is unknown")
        else:
            try:
                refund_fee_units = int(refund_fee)
            except (TypeError, ValueError):
                raise NearIntentsError("X3_NEAR_QUOTE_MALFORMED", f"refundFee is not an integer: {refund_fee!r}", "quote.refundFee")
            refund_bps = Decimal(refund_fee_units) * 10000 / Decimal(leg["amount"])
            record("refund_fee", refund_bps <= leg["fee_cap_bps"], cap_bps=leg["fee_cap_bps"],
                   refund_fee=str(refund_fee_units), refund_fee_bps=str(refund_bps.quantize(Decimal("0.01"))))

    if not request.get("dry"):
        record("deposit_address", bool(quote.get("depositAddress")), deposit_address=quote.get("depositAddress"))
        record("quote_signature", bool(response.get("signature")))
        deadline = quote.get("deadline")
        if not deadline:
            record("deadline", False, reason="live quote carries no deadline")
        else:
            remaining = (parse_timestamp(deadline) - now).total_seconds()
            estimate = Decimal(str(quote.get("timeEstimate", 0)))
            record("deadline", remaining > 0 and Decimal(str(remaining)) >= estimate,
                   seconds_remaining=int(remaining), time_estimate_s=str(estimate))

    failed = [r for r in results if not r["ok"]]
    if failed:
        raise NearIntentsError(
            "X3_NEAR_GUARD_FAILED",
            f"quote rejected by {', '.join(r['guard'] for r in failed)}; do not deposit",
            "quote",
            results,
        )
    return results


# --------------------------------------------------------------------------------------------
# 1Click client
# --------------------------------------------------------------------------------------------

Transport = Callable[[str, str, Dict[str, str], Optional[bytes], float], Tuple[int, bytes]]


def urllib_transport(method: str, url: str, headers: Dict[str, str], body: Optional[bytes], timeout: float) -> Tuple[int, bytes]:
    req = urllib.request.Request(url, data=body, headers=headers, method=method)
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            return resp.status, resp.read()
    except urllib.error.HTTPError as exc:
        return exc.code, exc.read()
    except urllib.error.URLError as exc:
        raise NearIntentsError("X3_NEAR_UNREACHABLE", f"cannot reach {url}: {exc.reason}")


class OneClickClient:
    def __init__(self, base_url: Optional[str] = None, jwt: Optional[str] = None,
                 transport: Transport = urllib_transport, timeout: float = 30.0):
        self.base_url = (base_url or os.environ.get("NEAR_INTENTS_BASE_URL") or DEFAULT_BASE_URL).rstrip("/")
        self.jwt = jwt if jwt is not None else os.environ.get("NEAR_INTENTS_JWT")
        self.transport = transport
        self.timeout = timeout

    @property
    def authenticated(self) -> bool:
        return bool(self.jwt)

    def _call(self, method: str, path: str, payload: Optional[Dict[str, Any]] = None) -> Any:
        headers = {"Accept": "application/json", "User-Agent": "x3-lang-near-intents/1"}
        body = None
        if payload is not None:
            headers["Content-Type"] = "application/json"
            body = json.dumps(payload).encode("utf-8")
        if self.jwt:
            headers["Authorization"] = f"Bearer {self.jwt}"
        status, raw = self.transport(method, f"{self.base_url}{path}", headers, body, self.timeout)
        try:
            data = json.loads(raw.decode("utf-8")) if raw else None
        except (UnicodeDecodeError, json.JSONDecodeError):
            data = None
        if status >= 400:
            message = data.get("message") if isinstance(data, dict) else None
            detail: Dict[str, Any] = {"status": status}
            if isinstance(data, dict) and data.get("correlationId"):
                # 1Click support traces a failed request by this ID.
                detail["correlationId"] = data["correlationId"]
            raise NearIntentsError(
                "X3_NEAR_HTTP",
                f"{method} {path} returned {status}: {message or raw[:300].decode('utf-8', 'replace')}",
                path,
                detail,
            )
        if data is None:
            raise NearIntentsError("X3_NEAR_HTTP", f"{method} {path} returned no JSON", path)
        return data

    def tokens(self) -> List[Dict[str, Any]]:
        data = self._call("GET", "/v0/tokens")
        if not isinstance(data, list):
            raise NearIntentsError("X3_NEAR_HTTP", "/v0/tokens did not return a list", "/v0/tokens")
        return data

    def quote(self, request: Dict[str, Any]) -> Dict[str, Any]:
        return self._call("POST", "/v0/quote", request)

    def submit_deposit(self, deposit_address: str, tx_hash: str) -> Dict[str, Any]:
        return self._call("POST", "/v0/deposit/submit", {"depositAddress": deposit_address, "txHash": tx_hash})

    def status(self, deposit_address: str, deposit_memo: Optional[str] = None) -> Dict[str, Any]:
        query = urllib.parse.urlencode({k: v for k, v in (("depositAddress", deposit_address), ("depositMemo", deposit_memo)) if v})
        return self._call("GET", f"/v0/status?{query}")


# --------------------------------------------------------------------------------------------
# NEAR account preflight
# --------------------------------------------------------------------------------------------

DEFAULT_NEAR_RPC_URL = "https://free.rpc.fastnear.com"
_IMPLICIT_ACCOUNT_RE = re.compile(r"[a-f0-9]{64}|0x[a-f0-9]{40}")


class NearRpc:
    """Just enough NEAR JSON-RPC to ask whether a named account exists.

    1Click answers a quote for a NEAR receiver that does not exist with HTTP 400 and the body
    `Internal server error`, which names nothing. Asking the chain first turns that into a
    refusal that says which address is wrong — and keeps a live run from ever being quoted
    toward an account that cannot receive the funds.
    """

    def __init__(self, url: Optional[str] = None, transport: Transport = urllib_transport, timeout: float = 15.0):
        self.url = url or os.environ.get("NEAR_RPC_URL") or DEFAULT_NEAR_RPC_URL
        self.transport = transport
        self.timeout = timeout

    def account_exists(self, account_id: str) -> bool:
        if _IMPLICIT_ACCOUNT_RE.fullmatch(account_id):
            # An implicit account is its key; it exists as soon as it is funded, so a deposit
            # to it is valid before any transaction has touched it.
            return True
        body = json.dumps({
            "jsonrpc": "2.0",
            "id": "x3-near-intents",
            "method": "query",
            "params": {"request_type": "view_account", "finality": "final", "account_id": account_id},
        }).encode("utf-8")
        status, raw = self.transport("POST", self.url, {"Content-Type": "application/json"}, body, self.timeout)
        try:
            data = json.loads(raw.decode("utf-8"))
        except (UnicodeDecodeError, json.JSONDecodeError):
            raise NearIntentsError("X3_NEAR_RPC", f"NEAR RPC {self.url} returned {status} with no JSON")
        if isinstance(data, dict) and isinstance(data.get("result"), dict) and "amount" in data["result"]:
            return True
        error = data.get("error") if isinstance(data, dict) else None
        if isinstance(error, dict):
            cause = (error.get("cause") or {}).get("name")
            if cause == "UNKNOWN_ACCOUNT":
                return False
            raise NearIntentsError("X3_NEAR_RPC", f"NEAR RPC {self.url} could not answer for {account_id!r}: "
                                   f"{cause or error.get('message') or error}")
        raise NearIntentsError("X3_NEAR_RPC", f"NEAR RPC {self.url} returned an unexpected body ({status})")


def preflight_accounts(leg: Dict[str, Any], rpc: NearRpc) -> List[Dict[str, Any]]:
    """Every NEAR account the swap pays out to must exist before a quote is requested."""
    checks = []
    for role, chain, account in (("recipient", leg["destination"]["chain"], leg["recipient"]),
                                 ("refund_to", leg["origin"]["chain"], leg["refund_to"])):
        if chain != "near":
            continue
        exists = rpc.account_exists(account)
        checks.append({"guard": f"{role}_account_exists", "ok": exists, "account": account})
        if not exists:
            raise NearIntentsError(
                "X3_NEAR_ACCOUNT_UNKNOWN",
                f"NEAR account {account!r} ({role}) does not exist; 1Click cannot deliver to it",
                "to.receiver" if role == "recipient" else "policies.timeout.action.to",
                checks,
            )
    return checks



# --------------------------------------------------------------------------------------------
# Nonce store
# --------------------------------------------------------------------------------------------

class NonceStore:
    def __init__(self, path: Optional[str] = None):
        self.path = path or os.environ.get("X3_NEAR_NONCE_STORE") or DEFAULT_NONCE_STORE

    @contextlib.contextmanager
    def _locked(self):
        import fcntl
        os.makedirs(os.path.dirname(os.path.abspath(self.path)), exist_ok=True)
        with open(self.path + ".lock", "a+") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX)
            try:
                yield
            finally:
                fcntl.flock(lock, fcntl.LOCK_UN)

    def _read(self) -> Dict[str, Any]:
        try:
            with open(self.path, "r", encoding="utf-8") as f:
                data = json.load(f)
        except FileNotFoundError:
            return {"used": {}}
        except json.JSONDecodeError:
            raise NearIntentsError("X3_NEAR_NONCE_STORE", f"nonce store {self.path} is corrupt; refusing to run without replay protection")
        data.setdefault("used", {})
        return data

    def check_unused(self, nonce: str) -> None:
        with self._locked():
            used = self._read()["used"]
            if nonce in used:
                raise NearIntentsError("X3_NEAR_NONCE_USED", f"nonce {nonce!r} has already been used", "requires.nonce", used[nonce])

    def consume(self, nonce: str, record: Dict[str, Any]) -> None:
        with self._locked():
            data = self._read()
            if nonce in data["used"]:
                raise NearIntentsError("X3_NEAR_NONCE_USED", f"nonce {nonce!r} has already been used", "requires.nonce", data["used"][nonce])
            data["used"][nonce] = record
            tmp = f"{self.path}.tmp.{os.getpid()}"
            with open(tmp, "w", encoding="utf-8") as f:
                json.dump(data, f, indent=2, sort_keys=True)
                f.flush()
                os.fsync(f.fileno())
            os.replace(tmp, self.path)


# --------------------------------------------------------------------------------------------
# Pipeline
# --------------------------------------------------------------------------------------------

def _sha256_json(value: Any) -> str:
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(",", ":")).encode("utf-8")).hexdigest()


def compile_program(path: str, *, now: Optional[datetime] = None) -> Tuple[Dict[str, Any], Dict[str, Any]]:
    """Parse, type-check and lower an X3 file. Returns (intent, leg)."""
    cli = _load("cli")
    typechecker = _load("typechecker")
    try:
        intent = cli.parse_file(path)
    except cli.X3ParseError as exc:
        raise NearIntentsError(exc.code, exc.message, exc.field, {"line": exc.line})
    ok, errors = typechecker.typecheck(intent)
    if not ok:
        raise NearIntentsError("X3_NEAR_TYPECHECK", "the program does not type-check", "intent",
                               typechecker.errors_to_json(errors))
    return intent, lower(intent, now=now)


def load_tokens(client: OneClickClient, tokens_path: Optional[str]) -> List[Dict[str, Any]]:
    if tokens_path:
        with open(tokens_path, "r", encoding="utf-8") as f:
            tokens = json.load(f)
        if not isinstance(tokens, list):
            raise NearIntentsError("X3_NEAR_TOKENS", f"{tokens_path} is not a /v0/tokens list")
        return tokens
    return client.tokens()


def plan(path: str, *, client: OneClickClient, tokens_path: Optional[str] = None, live: bool = False,
         now: Optional[datetime] = None) -> Dict[str, Any]:
    intent, leg = compile_program(path, now=now)
    tokens = load_tokens(client, tokens_path)
    request = build_quote_request(leg, tokens, dry=not live, referral=os.environ.get("NEAR_INTENTS_REFERRAL"))
    return {"intent": intent, "leg": leg, "quote_request": request}


def quote(path: str, *, client: OneClickClient, tokens_path: Optional[str] = None, live: bool = False,
          nonce_store: Optional[NonceStore] = None, receipt_path: Optional[str] = None,
          rpc: Optional[NearRpc] = None, now: Optional[datetime] = None) -> Dict[str, Any]:
    planned = plan(path, client=client, tokens_path=tokens_path, live=live, now=now)
    leg, request = planned["leg"], planned["quote_request"]
    store = nonce_store or NonceStore()
    if live and leg["nonce"]:
        store.check_unused(leg["nonce"])
    account_checks = preflight_accounts(leg, rpc or NearRpc())

    response = client.quote(request)
    guards = account_checks + verify_quote(leg, request, response, now=now)

    result: Dict[str, Any] = {
        "status": "quoted" if not live else "awaiting_deposit",
        "mode": "live" if live else "dry",
        "authenticated": client.authenticated,
        "leg": leg,
        "quote_request": request,
        "quote_response": response,
        "guards": guards,
    }
    if not client.authenticated:
        result["note"] = "no NEAR_INTENTS_JWT: the unauthenticated 1Click fee schedule applies"
    if not live:
        return result

    quote_obj = response["quote"]
    with open(path, "rb") as f:
        program_sha256 = hashlib.sha256(f.read()).hexdigest()
    runner = _load("runner")
    receipt = {
        "kind": RECEIPT_KIND,
        "receipt_version": 1,
        "created_at": format_timestamp(now or datetime.now(timezone.utc)),
        "intent": leg["intent"],
        "program_sha256": program_sha256,
        "validated_intent_v1": runner.rust_intent_envelope(planned["intent"]),
        "leg": leg,
        "quote_request": request,
        "quote_response": response,
        "quote_response_sha256": _sha256_json(response),
        "guards": guards,
        "deposit": {
            "address": quote_obj.get("depositAddress"),
            "memo": quote_obj.get("depositMemo"),
            "asset": request["originAsset"],
            "amount": quote_obj.get("amountIn"),
            "deadline": quote_obj.get("deadline"),
        },
        "status_history": [],
    }
    if leg["nonce"]:
        store.consume(leg["nonce"], {
            "intent": leg["intent"],
            "deposit_address": quote_obj.get("depositAddress"),
            "consumed_at": receipt["created_at"],
            "quote_response_sha256": receipt["quote_response_sha256"],
        })
    receipt_path = receipt_path or f"{leg['intent']}.near-receipt.json"
    write_receipt(receipt_path, receipt)
    result["receipt_path"] = receipt_path
    result["deposit"] = receipt["deposit"]
    return result


def write_receipt(path: str, receipt: Dict[str, Any]) -> None:
    tmp = f"{path}.tmp.{os.getpid()}"
    with open(tmp, "w", encoding="utf-8") as f:
        json.dump(receipt, f, indent=2, sort_keys=True)
    os.replace(tmp, path)


def interpret_status(response: Dict[str, Any]) -> Dict[str, Any]:
    status = response.get("status")
    if status not in STATUS_OUTCOMES:
        raise NearIntentsError("X3_NEAR_STATUS_UNKNOWN", f"1Click reported an unknown status {status!r}", "status")
    details = response.get("swapDetails") or {}
    return {
        "status": status,
        "outcome": STATUS_OUTCOMES[status],
        "terminal": status in TERMINAL_STATUSES,
        "updated_at": response.get("updatedAt"),
        "amount_out": details.get("amountOut"),
        "refunded_amount": details.get("refundedAmount"),
        "refund_reason": details.get("refundReason"),
        "origin_tx": [t.get("hash") for t in details.get("originChainTxHashes") or []],
        "destination_tx": [t.get("hash") for t in details.get("destinationChainTxHashes") or []],
        "near_tx": list(details.get("nearTxHashes") or []),
    }


def track(client: OneClickClient, deposit_address: str, *, memo: Optional[str] = None, wait: bool = False,
          interval: float = 5.0, max_wait: float = 3600.0, receipt_path: Optional[str] = None,
          sleep: Callable[[float], None] = time.sleep, clock: Callable[[], float] = time.monotonic) -> Dict[str, Any]:
    started = clock()
    last_status = None
    while True:
        observed = interpret_status(client.status(deposit_address, memo))
        if receipt_path and observed["status"] != last_status:
            _append_status(receipt_path, observed)
        last_status = observed["status"]
        if observed["terminal"] or not wait:
            return observed
        if clock() - started + interval > max_wait:
            observed["note"] = f"still {observed['status']} after waiting {int(clock() - started)}s"
            return observed
        sleep(interval)


def _append_status(receipt_path: str, observed: Dict[str, Any]) -> None:
    with open(receipt_path, "r", encoding="utf-8") as f:
        receipt = json.load(f)
    if receipt.get("kind") != RECEIPT_KIND:
        raise NearIntentsError("X3_NEAR_RECEIPT", f"{receipt_path} is not an {RECEIPT_KIND} receipt")
    receipt.setdefault("status_history", []).append(observed)
    receipt["outcome"] = observed["outcome"]
    write_receipt(receipt_path, receipt)


# --------------------------------------------------------------------------------------------
# CLI
# --------------------------------------------------------------------------------------------

def _emit(payload: Dict[str, Any], code: int = 0) -> int:
    print(json.dumps(payload, indent=2, sort_keys=True))
    return code


def main(argv: Optional[List[str]] = None, *, client: Optional[OneClickClient] = None,
         rpc: Optional[NearRpc] = None) -> int:
    parser = argparse.ArgumentParser(description="Settle X3 intents through NEAR Intents (1Click API)")
    sub = parser.add_subparsers(dest="command", required=True)

    p_plan = sub.add_parser("plan", help="compile an intent to a 1Click quote request (no quote is requested)")
    p_plan.add_argument("input")
    p_plan.add_argument("--tokens", help="a saved /v0/tokens response; fetched live when omitted")

    p_quote = sub.add_parser("quote", help="request a quote and check it against the program's guards")
    p_quote.add_argument("input")
    p_quote.add_argument("--tokens")
    p_quote.add_argument("--live", action="store_true",
                         help="request a real deposit address, consume the nonce and write a receipt")
    p_quote.add_argument("--receipt", help="receipt path for --live (default <intent>.near-receipt.json)")
    p_quote.add_argument("--nonce-store", help="nonce store path (default $X3_NEAR_NONCE_STORE or ~/.x3/...)")

    p_submit = sub.add_parser("submit-deposit", help="tell 1Click the deposit transaction hash")
    p_submit.add_argument("--deposit-address", required=True)
    p_submit.add_argument("--tx-hash", required=True)

    p_status = sub.add_parser("status", help="report the swap's X3 outcome")
    p_status.add_argument("--deposit-address", required=True)
    p_status.add_argument("--memo")
    p_status.add_argument("--wait", action="store_true", help="poll until the swap settles, refunds or fails")
    p_status.add_argument("--interval", type=float, default=5.0)
    p_status.add_argument("--max-wait", type=float, default=3600.0)
    p_status.add_argument("--receipt", help="append observed statuses to this receipt")

    args = parser.parse_args(argv)
    client = client or OneClickClient()
    try:
        if args.command == "plan":
            planned = plan(args.input, client=client, tokens_path=args.tokens)
            return _emit({"status": "planned", "leg": planned["leg"], "quote_request": planned["quote_request"]})
        if args.command == "quote":
            result = quote(args.input, client=client, tokens_path=args.tokens, live=args.live,
                           nonce_store=NonceStore(args.nonce_store), receipt_path=args.receipt, rpc=rpc)
            return _emit(result)
        if args.command == "submit-deposit":
            return _emit({"status": "submitted", "response": client.submit_deposit(args.deposit_address, args.tx_hash)})
        if args.command == "status":
            observed = track(client, args.deposit_address, memo=args.memo, wait=args.wait,
                             interval=args.interval, max_wait=args.max_wait, receipt_path=args.receipt)
            code = OUTCOME_EXIT_CODES.get(observed["outcome"], EXIT_NOT_TERMINAL)
            return _emit(observed, code)
    except NearIntentsError as exc:
        print(json.dumps({"status": "error", "errors": [exc.to_dict()]}, indent=2, sort_keys=True), file=sys.stderr)
        return 1
    return 1


if __name__ == "__main__":
    sys.exit(main())
