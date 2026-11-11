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

- **first, on dry and live quotes:** the 1Click Ed25519 signature, against the pinned manager
  key, over the exact payload the official SDK signs (`verify_oneclick_quote_signature`).
  Nothing else in the response is trusted until it verifies;
- the quote is fresh: issued no more than 5 minutes ago and not dated in the future;
- the echoed `quoteRequest` is the request that was sent;
- `amountIn` is the amount the program spends;
- `minAmountOut` clears the leg's `min_receive` floor;
- the spread between `amountOut` and `minAmountOut` is within the requested slippage;
- `require fees <= N` holds on every path: the quote's effective cost (`amountInUsd` vs
  `amountOutUsd`) and, separately, the `refundFee` a failed swap would be charged;
- a live quote carries a deposit address and a deadline far enough out for its own
  `timeEstimate`, and no later than the program's `timeout`.

A dry quote's `refundFee`, `deadline`, `timeEstimate` and `depositAddress` are not signed by
1Click; only a live quote's are, and only a live quote can authorize a deposit.

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
it as `X3_NEAR_ACCOUNT_UNKNOWN`, naming the account. This includes syntactically valid 64-hex
implicit account IDs: syntax does not prove that the account has been initialized on chain.

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
- `BSC_RPC_URLS` — comma-separated BSC JSON-RPC endpoints used to verify native-BNB deposits
  and refunds against the `finalized` block; defaults to two public endpoints.

This module never signs or sends the deposit. `quote --live` returns the deposit address and
the exact amount; the deposit is made from the sender's wallet, after which `submit-deposit`
and `status` track the swap to a terminal X3 outcome.

## Proving the outcome from the chains (`chain_evidence.py`)

1Click's status endpoint is not signed, so a receipt never takes it as proof:

- `quote --live` reads the recipient's NEP-141 balance on NEAR before any deposit exists. A
  live quote whose delivery could never be measured is refused (`X3_EVIDENCE_BASELINE`).
- `verify-deposit` proves from Solana (`getTransaction`, finalized) that at least `amountIn`
  of the origin mint reached the signed deposit address before the signed deadline. For BSC,
  it proves a successful native-BNB transfer from the transaction and receipt and requires
  the transaction block to be at or below the endpoint's `finalized` block.
- `verify-settlement` records a limited balance comparison against the locally saved
  baseline. It cannot authenticate that baseline or link a payout transaction to the quote,
  so it never declares a settled outcome and exits unverified even if the balance rises.
  With `--refund-tx`, it checks a later incoming origin-chain transfer to the signed refund
  address for at least `amountIn - refundFee`; that observation does not prove solver causality.
- `verified_outcome=settled` and settled Guard outcome arguments are disabled until
  payout transaction verification is implemented. Refund observations retain the existing
  behavior, including whether 1Click's reported status agrees. Every check records its RPC endpoints,
  slot or block, and a SHA-256 of each raw result. `SOLANA_RPC_URLS` / `BSC_RPC_URLS` /
  `NEAR_RPC_URLS`
  (comma-separated) add endpoints that must all agree.
"""
import argparse
import contextlib
import hashlib
import math
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
    # Sibling modules import `numeric` as a top-level module. Make sure this directory is on
    # the path while they load, however this module itself was imported.
    added = _HERE not in sys.path
    if added:
        sys.path.insert(0, _HERE)
    try:
        spec.loader.exec_module(module)
    finally:
        if added:
            sys.path.remove(_HERE)
    return module


registry = _load("registry")
ed25519 = _load("ed25519")
chain_evidence = _load("chain_evidence")
helius_evidence = importlib.import_module("helius_evidence")
_MANDATE_MODULE = None
_CLI_MODULE = None


def cli_module():
    global _CLI_MODULE
    if _CLI_MODULE is None:
        _CLI_MODULE = _load("cli")
    return _CLI_MODULE


def mandates():
    """`near_mandate.py`, bound to this adapter's errors, units and asset resolution."""
    global _MANDATE_MODULE
    if _MANDATE_MODULE is None:
        module = _load("near_mandate")
        module.bind(sys.modules.get(__name__) or _AdapterView())
        _MANDATE_MODULE = module
    return _MANDATE_MODULE


class _AdapterView:
    """This module's namespace, for when it was loaded by path and is not in `sys.modules`."""

    def __getattr__(self, name: str) -> Any:
        try:
            return globals()[name]
        except KeyError:
            raise AttributeError(name)

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
    "bsc": "bsc",
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
EXIT_MANDATE_DENIED = 6


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


upper_bound_bps = _upper_bound_bps


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

    leg_out: Dict[str, Any] = {
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
    mandate = intent.get("mandate")
    if mandate is not None:
        owner = (mandate or {}).get("owner")
        if not owner or not registry.is_valid_near_account(owner):
            raise NearIntentsError("X3_MANDATE_OWNER", f"`under mandate` names {owner!r}, which is not a NEAR account",
                                   "mandate.owner")
        if not leg_out["nonce"]:
            raise NearIntentsError("X3_NEAR_NONCE", "an intent under a mandate needs `require nonce unused <id>`: "
                                   "x3-guard records it under the owner by its nonce", "requires.nonce")
        # Only present for agent intents, so every other program lowers exactly as before.
        leg_out["mandate_owner"] = owner
    return leg_out


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

# --------------------------------------------------------------------------------------------
# 1Click quote signatures
# --------------------------------------------------------------------------------------------
#
# A port of `verifyQuoteSignature` from @defuse-protocol/one-click-sdk-typescript 0.1.26
# (`src/quote-signature.ts`), checked against that SDK on real 1Click responses
# (`tests/test_near_intents.py`). The signed message is
#
#     base58(sha256(json_stable_stringify({ ...signedRequest, ...signedQuote, timestamp })))
#
# as UTF-8 bytes, Ed25519-signed by the 1Click manager key. Only allow-listed fields are
# signed, and a dry quote signs fewer than a live one: `depositAddress`, `deadline`,
# `timeEstimate`, `refundFee` and `withdrawFee` are covered **only on live quotes**. Fields
# 1Click echoes but does not sign (`appFees`, `depositMode`, `confidentiality`, …) are left out.

#: The 1Click manager's Ed25519 key, pinned from SDK 0.1.26. It is the trust anchor for every
#: quote, so it is a constant in reviewed code, not configuration: a rotated key is a code change.
ONE_CLICK_MANAGER_PUBLIC_KEY = "ed25519:reYaWhvwu8Jzo3WUM3zhn6VrhuMEF4eADL17qtRVifc"
ED25519_PREFIX = "ed25519:"
#: A quote older than this, or dated further than the skew into the future, is not fresh.
MAX_QUOTE_AGE_S = 300
MAX_CLOCK_SKEW_S = 60

SIGNED_REQUEST_FIELDS = (
    "dry", "swapType", "slippageTolerance", "originAsset", "depositType", "destinationAsset",
    "amount", "refundTo", "refundType", "recipient", "recipientType", "deadline",
)
#: Request fields the SDK includes only when they are truthy (`x ? x : undefined`).
SIGNED_REQUEST_FIELDS_IF_SET = (
    "quoteWaitingTimeMs", "referral", "virtualChainRecipient", "virtualChainRefundRecipient",
    "customRecipientMsg",
)
SIGNED_QUOTE_FIELDS = (
    "amountIn", "amountInFormatted", "amountInUsd", "minAmountIn",
    "amountOut", "amountOutFormatted", "amountOutUsd", "minAmountOut",
)
#: Live-only quote fields, included only when truthy (`x || undefined`).
SIGNED_LIVE_QUOTE_FIELDS_IF_SET = (
    "depositAddress", "depositMemo", "deadline", "timeWhenInactive", "timeEstimate", "refundFee", "withdrawFee",
)


class _Undefined:
    """JavaScript `undefined`: a key holding it is omitted from the serialized object."""

    def __repr__(self) -> str:
        return "undefined"


UNDEFINED = _Undefined()


class QuoteSignatureFormatError(ValueError):
    """The response cannot be serialized the way the 1Click SDK would serialize it."""


def _js_truthy(value: Any) -> bool:
    if value is UNDEFINED or value is None or value is False:
        return False
    if isinstance(value, (int, float)) and not isinstance(value, bool):
        return value == value and value != 0  # NaN and 0 are falsy
    if isinstance(value, str):
        return value != ""
    return True


def _js_number(value: Any) -> str:
    """`JSON.stringify` of a number, for the integers 1Click sends. Anything else is refused."""
    if isinstance(value, int):
        return str(value)
    if isinstance(value, float) and value == value and abs(value) < 1e21 and value.is_integer():
        return str(int(value))
    raise QuoteSignatureFormatError(f"unsupported number in a signed field: {value!r}")


def js_stable_stringify(value: Any) -> str:
    """`json-stable-stringify` 1.3.0 with default options, for JSON-shaped Python values."""
    if value is UNDEFINED:
        raise QuoteSignatureFormatError("undefined has no JSON form")
    if value is None:
        return "null"
    if value is True:
        return "true"
    if value is False:
        return "false"
    if isinstance(value, (int, float)):
        return _js_number(value)
    if isinstance(value, str):
        # JSON.stringify escapes the same set Python does with ensure_ascii=False: `"`, `\\`,
        # \b \f \n \r \t and other control characters as lowercase \u00xx.
        return json.dumps(value, ensure_ascii=False)
    if isinstance(value, list):
        return "[" + ",".join("null" if v is UNDEFINED else js_stable_stringify(v) for v in value) + "]"
    if isinstance(value, dict):
        for key in value:
            if not isinstance(key, str):
                raise QuoteSignatureFormatError(f"non-string key {key!r}")
        # JavaScript's default sort compares UTF-16 code units.
        keys = sorted(value, key=lambda k: k.encode("utf-16-be"))
        parts = [json.dumps(k, ensure_ascii=False) + ":" + js_stable_stringify(value[k]) for k in keys if value[k] is not UNDEFINED]
        return "{" + ",".join(parts) + "}"
    raise QuoteSignatureFormatError(f"unsupported value of type {type(value).__name__}")


def signed_quote_payload(response: Dict[str, Any]) -> str:
    """The exact string 1Click signs the hash of, built as the SDK builds it."""
    if not isinstance(response, dict):
        raise QuoteSignatureFormatError("quote response is not an object")
    request, quote = response.get("quoteRequest"), response.get("quote")
    if not isinstance(request, dict) or not isinstance(quote, dict):
        raise QuoteSignatureFormatError("quote response lacks quoteRequest/quote objects")
    if "timestamp" not in response or not isinstance(response["timestamp"], str):
        raise QuoteSignatureFormatError("quote response lacks a string timestamp")

    merged: Dict[str, Any] = {k: request.get(k, UNDEFINED) for k in SIGNED_REQUEST_FIELDS}
    for k in SIGNED_REQUEST_FIELDS_IF_SET:
        merged[k] = request[k] if _js_truthy(request.get(k, UNDEFINED)) else UNDEFINED
    signed_quote: Dict[str, Any] = {k: quote.get(k, UNDEFINED) for k in SIGNED_QUOTE_FIELDS}
    if not _js_truthy(request.get("dry", UNDEFINED)):
        for k in SIGNED_LIVE_QUOTE_FIELDS_IF_SET:
            signed_quote[k] = quote[k] if _js_truthy(quote.get(k, UNDEFINED)) else UNDEFINED
    # `{ ...request, ...quote }`: a quote key overrides the request's, even with `undefined` —
    # so a live quote with no `deadline` drops the request's deadline from the signed payload.
    merged.update(signed_quote)
    merged["timestamp"] = response["timestamp"]
    return js_stable_stringify(merged)


def oneclick_quote_hash(response: Dict[str, Any]) -> str:
    """The SDK's `quoteHash`: base58 of SHA-256 over the signed payload."""
    return ed25519.b58encode(hashlib.sha256(signed_quote_payload(response).encode("utf-8")).digest())


def _decode_ed25519(value: Any, length: int) -> bytes:
    if not isinstance(value, str) or not value.startswith(ED25519_PREFIX):
        raise QuoteSignatureFormatError(f"expected an {ED25519_PREFIX}<base58> value")
    encoded = value[len(ED25519_PREFIX):]
    # Base58 is one alphabet character per leading zero byte and at most
    # ceil(8 * length / log2(58)) characters otherwise. Refuse oversized
    # network input before converting the whole string to a big integer.
    maximum = math.ceil(length * 8 / math.log2(58))
    if not length <= len(encoded) <= maximum:
        raise QuoteSignatureFormatError(
            f"expected {length} bytes encoded in {length}..{maximum} base58 characters"
        )
    raw = ed25519.b58decode(encoded)
    if len(raw) != length:
        raise QuoteSignatureFormatError(f"expected {length} bytes, decoded {len(raw)}")
    return raw


def check_oneclick_quote_signature(response: Dict[str, Any], manager_public_key: Optional[str] = None) -> Tuple[bool, str]:
    """(valid, reason). Never raises: anything it cannot check is invalid."""
    try:
        public = _decode_ed25519(manager_public_key or ONE_CLICK_MANAGER_PUBLIC_KEY, 32)
        signature = _decode_ed25519(response.get("signature") if isinstance(response, dict) else None, 64)
        message = oneclick_quote_hash(response).encode("utf-8")
    except (QuoteSignatureFormatError, ValueError) as exc:
        return False, f"unsupported quote format: {exc}"
    if not ed25519.verify(public, message, signature):
        return False, "signature does not verify under the 1Click manager key"
    return True, "verified"


def verify_oneclick_quote_signature(response: Dict[str, Any], manager_public_key: Optional[str] = None) -> bool:
    """True only if `response` is signed by the 1Click manager key over its signed fields."""
    return check_oneclick_quote_signature(response, manager_public_key)[0]


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


def verify_quote(leg: Dict[str, Any], request: Dict[str, Any], response: Dict[str, Any], *, now: Optional[datetime] = None,
                 manager_public_key: Optional[str] = None) -> List[Dict[str, Any]]:
    """Check a 1Click quote against the program. Raises with every result if any guard fails.

    The signature is checked first, on dry and live quotes alike, and nothing else in the
    response is read until it verifies: every later guard reads fields the signature covers.
    """
    now = now or datetime.now(timezone.utc)
    results: List[Dict[str, Any]] = []

    def record(guard: str, ok: bool, **detail: Any) -> None:
        results.append({"guard": guard, "ok": bool(ok), **detail})

    signature_valid, reason = check_oneclick_quote_signature(response, manager_public_key)
    record("quote_signature", signature_valid, reason=reason,
           manager_public_key=manager_public_key or ONE_CLICK_MANAGER_PUBLIC_KEY, backend=ed25519.BACKEND,
           quote_hash=oneclick_quote_hash(response) if signature_valid else None)
    if not signature_valid:
        raise NearIntentsError(
            "X3_NEAR_INVALID_SIGNATURE",
            "Cryptographic quote verification failed; deposit prohibited",
            "quote.signature",
            results,
        )
    issued = parse_timestamp(response["timestamp"])
    age = (now - issued).total_seconds()
    record("quote_freshness", -MAX_CLOCK_SKEW_S <= age <= MAX_QUOTE_AGE_S,
           age_s=int(age), max_age_s=MAX_QUOTE_AGE_S, issued_at=response["timestamp"])

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
                if isinstance(refund_fee, bool):
                    raise ValueError("refundFee is a boolean")
                refund_fee_units = int(refund_fee)
            except (TypeError, ValueError):
                raise NearIntentsError("X3_NEAR_QUOTE_MALFORMED", f"refundFee is not an integer: {refund_fee!r}", "quote.refundFee")
            # Zero is omitted by 1Click's truthy-only live signed fields; both
            # zero and a negative value could otherwise pass the fee-cap check.
            fee_safe = 0 < refund_fee_units < int(leg["amount"])
            refund_bps = Decimal(refund_fee_units) * 10000 / Decimal(leg["amount"])
            record("refund_fee", fee_safe and refund_bps <= leg["fee_cap_bps"], cap_bps=leg["fee_cap_bps"],
                   refund_fee=str(refund_fee_units), refund_fee_bps=str(refund_bps.quantize(Decimal("0.01"))),
                   reason=None if fee_safe else "refund fee must be positive and less than amountIn",
                   # 1Click signs refundFee only on live quotes and only when truthy.
                   signed=not request.get("dry") and bool(refund_fee))

    if not request.get("dry"):
        record("deposit_address", bool(quote.get("depositAddress")), deposit_address=quote.get("depositAddress"))
        deadline = quote.get("deadline")
        if not deadline:
            record("deadline", False, reason="live quote carries no deadline")
        else:
            quote_deadline = parse_timestamp(deadline)
            remaining = (quote_deadline - now).total_seconds()
            estimate = Decimal(str(quote.get("timeEstimate", 0)))
            # A deposit address that outlives the program's `timeout` could settle after the
            # program said refund; the guard contract refuses it too (`x3-guard::evaluate`).
            within_timeout = quote_deadline <= parse_timestamp(leg["deadline"])
            record("deadline", remaining > 0 and Decimal(str(remaining)) >= estimate and within_timeout,
                   seconds_remaining=int(remaining), time_estimate_s=str(estimate),
                   within_program_timeout=within_timeout)

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
            minimum = re.search(r"minimum swap amount is \$([0-9][0-9,]*(?:\.[0-9]+)?)", str(message or ""), re.I)
            if path == "/v0/quote" and minimum:
                minimum_usd = minimum.group(1).replace(",", "")
                detail.update({
                    "providerMinimumUsd": minimum_usd,
                    "retryable": False,
                    "quote_request": payload,
                })
                raise NearIntentsError(
                    "X3_NEAR_PROVIDER_MINIMUM",
                    f"1Click refused this quote because its ${minimum_usd} temporary minimum is above "
                    f"the program amount {payload.get('amount')} in base units",
                    "/v0/quote",
                    detail,
                )
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
        # A syntactically valid implicit ID is not necessarily initialized on-chain.
        # The 1Click destination requires a real account; always ask the RPC.
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

@contextlib.contextmanager
def _locked_path(path: str):
    import fcntl
    with open(os.path.realpath(path) + ".lock", "a+") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        try:
            yield
        finally:
            fcntl.flock(lock, fcntl.LOCK_UN)


class NonceStore:
    def __init__(self, path: Optional[str] = None):
        self.path = path or os.environ.get("X3_NEAR_NONCE_STORE") or DEFAULT_NONCE_STORE

    def _locked(self):
        os.makedirs(os.path.dirname(os.path.abspath(self.path)), exist_ok=True)
        return _locked_path(self.path)

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


def _sha256_file(path: str) -> str:
    with open(path, "rb") as handle:
        return hashlib.sha256(handle.read()).hexdigest()


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
    resolved = {
        side: _token_summary(resolve_asset(tokens, leg[key]["chain"], leg[key]["asset"]))
        for side, key in (("origin", "origin"), ("destination", "destination"))
    }
    return {"intent": intent, "leg": leg, "quote_request": request, "tokens": resolved}


def _token_summary(token: Dict[str, Any]) -> Dict[str, Any]:
    return {k: token.get(k) for k in ("assetId", "blockchain", "symbol", "decimals", "contractAddress")}


def nep141_contract(asset_id: str) -> str:
    """`nep141:usdc.near` -> `usdc.near`. Anything else has no NEAR token contract to read."""
    if not isinstance(asset_id, str) or not asset_id.startswith("nep141:") or len(asset_id) <= len("nep141:"):
        raise NearIntentsError("X3_EVIDENCE_UNSUPPORTED", f"{asset_id!r} is not a NEP-141 token on NEAR")
    return asset_id[len("nep141:"):]


def quote(path: str, *, client: OneClickClient, tokens_path: Optional[str] = None, live: bool = False,
          nonce_store: Optional[NonceStore] = None, receipt_path: Optional[str] = None,
          rpc: Optional[NearRpc] = None, now: Optional[datetime] = None,
          manager_public_key: Optional[str] = None, near_views: Optional[Any] = None,
          mandate_path: Optional[str] = None, guard_contract: Optional[str] = None) -> Dict[str, Any]:
    planned = plan(path, client=client, tokens_path=tokens_path, live=live, now=now)
    leg, request = planned["leg"], planned["quote_request"]
    store = nonce_store or NonceStore()
    if live and leg["nonce"]:
        store.check_unused(leg["nonce"])
    rpc = rpc or NearRpc()
    # An agent's intent is checked against its mandate before 1Click is asked for anything,
    # so a payment the contract would refuse never gets a deposit address.
    mandate_check = None
    if leg.get("mandate_owner") or mandate_path:
        if live and not guard_contract:
            raise NearIntentsError(
                "X3_MANDATE_REQUIRED",
                "a live quote for a mandate-controlled intent requires --guard-contract "
                "so revocation and rolling spend are read from chain state",
                "mandate",
            )
        if not mandate_path:
            if live:
                raise NearIntentsError(
                    "X3_MANDATE_REQUIRED",
                    f"this intent is `under mandate {leg['mandate_owner']}`: a live quote needs --mandate <file> "
                    "(and --guard-contract to read the mandate's live state)",
                    "mandate",
                )
        else:
            with open(path, "rb") as f:
                intent_sha = hashlib.sha256(f.read()).hexdigest()
            mandate_check = check_against_mandate(
                leg, request, intent_sha, mandate_path,
                tokens=load_tokens(client, tokens_path), guard_contract=guard_contract,
                near_views=near_views or (near_views_for(rpc) if guard_contract else None), now=now)
    account_checks = preflight_accounts(leg, rpc)

    try:
        response = client.quote(request)
    except NearIntentsError as exc:
        if isinstance(exc.detail, dict):
            exc.detail.setdefault("quote_request", request)
            exc.detail["account_checks"] = account_checks
        raise
    guards = (mandate_check["checks"] if mandate_check else []) + account_checks + verify_quote(
        leg, request, response, now=now, manager_public_key=manager_public_key)

    result: Dict[str, Any] = {
        "status": "quoted" if not live else "awaiting_deposit",
        "mode": "live" if live else "dry",
        "authenticated": client.authenticated,
        "leg": leg,
        "quote_request": request,
        "quote_response": response,
        "guards": guards,
    }
    if mandate_check:
        result["mandate"] = mandate_check
    if not client.authenticated:
        result["note"] = "no NEAR_INTENTS_JWT: the unauthenticated 1Click fee schedule applies"
    # Reaching this line means the signature verified, the echoed request matched, every policy
    # guard passed, the destination account exists and the quote is fresh. Only a live quote's
    # deposit address and deadline are signed, so only a live quote can be eligible.
    result["eligible_for_wallet_authorization"] = live
    result["authorization_note"] = (
        "eligible: a separate wallet signature is still required before any funds move; this "
        "adapter never signs or sends the deposit"
        if live else "dry quote: not a deposit instruction"
    )
    if not live:
        return result

    quote_obj = response["quote"]
    # The recipient's balance before the deposit exists, so the delivery can later be proven
    # as a rise from it. Taken before the nonce is consumed or a receipt written: a live quote
    # whose settlement could never be verified is refused, not issued.
    baseline = None
    if leg["destination"]["chain"] == "near":
        views = near_views or near_views_for(rpc)
        try:
            baseline = views.ft_balance(nep141_contract(request["destinationAsset"]), leg["recipient"])
        except chain_evidence.ChainEvidenceError as exc:
            raise NearIntentsError("X3_EVIDENCE_BASELINE", f"could not read the recipient's starting balance: {exc.message}",
                                   "evidence", exc.detail)
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
        # What 1Click signed, and its hash: anyone can re-verify the receipt with these.
        "quote_hash": oneclick_quote_hash(response),
        "signed_payload": signed_quote_payload(response),
        "guards": guards,
        "deposit": {
            "address": quote_obj.get("depositAddress"),
            "memo": quote_obj.get("depositMemo"),
            "asset": request["originAsset"],
            "amount": quote_obj.get("amountIn"),
            "deadline": quote_obj.get("deadline"),
        },
        "status_history": [],
        "tokens": planned["tokens"],
        "evidence": {"settlement_baseline": baseline},
    }
    if mandate_check:
        receipt["mandate"] = mandate_check
    receipt_path = receipt_path or f"{leg['intent']}.near-receipt.json"
    os.makedirs(os.path.dirname(os.path.abspath(receipt_path)), exist_ok=True)
    with _locked_path(receipt_path):
        if os.path.lexists(receipt_path):
            raise NearIntentsError("X3_NEAR_RECEIPT", "live quotes require a fresh receipt path")
        if leg["nonce"]:
            store.consume(leg["nonce"], {
                "intent": leg["intent"],
                "deposit_address": quote_obj.get("depositAddress"),
                "consumed_at": receipt["created_at"],
                "quote_response_sha256": receipt["quote_response_sha256"],
            })
        write_receipt(receipt_path, receipt)
    result["receipt_path"] = receipt_path
    result["deposit"] = receipt["deposit"]
    return result


def check_against_mandate(leg: Dict[str, Any], request: Dict[str, Any], program_sha256: str, mandate_path: str, *,
                          tokens: List[Dict[str, Any]], guard_contract: Optional[str] = None,
                          near_views: Optional[Any] = None, now: Optional[datetime] = None) -> Dict[str, Any]:
    """Check an agent's lowered intent against its mandate, as x3-guard will.

    With `guard_contract`, the mandate the contract holds is read from chain (revocation,
    version, rolling spend) and is the one checked; the file only names owner and agent.
    Without it the compiled file is checked, which cannot know about revocation or spending.
    Raises `X3_MANDATE_DENIED` with every check if any fails.
    """
    m = mandates()
    compiled = m.compile_mandate(mandate_path, tokens, now=now)
    owner = leg.get("mandate_owner")
    if owner != compiled["owner"]:
        raise NearIntentsError("X3_MANDATE_OWNER",
                               f"the intent is under mandate {owner!r}, but {mandate_path} is {compiled['owner']}'s",
                               "mandate.owner")
    source: Dict[str, Any] = {"kind": "file", "path": mandate_path}
    mandate = {"owner": compiled["owner"], "agent": compiled["agent"], "terms": compiled["terms"], "version": None,
               "revoked": False, "spends": []}
    if guard_contract:
        try:
            view = near_views.view_json(guard_contract, "get_mandate", {"owner": compiled["owner"], "agent": compiled["agent"]})
        except chain_evidence.ChainEvidenceError as exc:
            raise NearIntentsError("X3_MANDATE_CHAIN", f"could not read the mandate from {guard_contract}: {exc.message}",
                                   "mandate", exc.detail)
        if view["value"] is None:
            raise NearIntentsError("X3_MANDATE_DENIED",
                                   f"{guard_contract} holds no mandate from {compiled['owner']} for {compiled['agent']}; "
                                   "the owner sends `mandate args` first", "mandate")
        mandate = m.mandate_from_chain(view["value"])
        source = {"kind": "chain", "contract": guard_contract, "block_height": view["block_height"],
                  "block_hash": view["block_hash"], "path": mandate_path}
    policy = policy_for(leg, request, program_sha256)
    now_ms = int((now or datetime.now(timezone.utc)).timestamp() * 1000)
    checks = m.check_mandate(mandate, policy, now_ms)
    out = {"owner": mandate["owner"], "agent": mandate["agent"], "version": mandate["version"], "source": source,
           "checks": checks, "approvals_needed": m.approvals_needed(mandate, int(policy["amount"]))}
    failed = [c["guard"] for c in checks if not c["ok"]]
    if failed:
        raise NearIntentsError("X3_MANDATE_DENIED", f"the mandate refuses this intent: {', '.join(failed)}", "mandate", out)
    return out


# --------------------------------------------------------------------------------------------
# x3-guard contract arguments
# --------------------------------------------------------------------------------------------

GUARD_STEPS = ("register", "quote", "outcome")
GUARD_METHODS = {"register": "register_intent", "quote": "accept_quote", "outcome": "record_outcome"}
# An agent's intent (`under mandate <owner>`) goes through the agent methods, which take the
# owner and check the mandate; `approve` is sent by an approver for payments above the
# mandate's approval threshold.
AGENT_STEPS = GUARD_STEPS + ("approve",)
AGENT_METHODS = {"register": "agent_register_intent", "quote": "agent_accept_quote",
                 "outcome": "agent_record_outcome", "approve": "approve_intent"}
# Attached NEAR per call. Storage is charged by the bytes written and the excess refunded in
# the same call, so these are ceilings, not prices.
GUARD_DEPOSITS = {"register": "0.05 NEAR", "quote": "0.05 NEAR", "outcome": "0.02 NEAR", "approve": "0.02 NEAR",
                  "mandate": "0.1 NEAR", "revoke": "0 NEAR"}
GUARD_GAS = "30 Tgas"
_STATUS_TO_GUARD = {"SUCCESS": "Settled", "REFUNDED": "Refunded", "FAILED": "Failed"}
MAX_GUARD_TX_HASHES = 16
PREPAID_AGENT_STORAGE_HELP = (
    "use zero attached deposit for the three agent mutation methods only; requires a reviewed "
    "Guard supporting owner-prepaid storage and sufficient owner-funded credit; verifies neither"
)


def validate_agent_key_pin(public_key, max_allowance_yocto):
    import near_agent_key
    try:
        near_agent_key.validate_pin(public_key, max_allowance_yocto)
    except near_agent_key.AgentKeyError as exc:
        raise NearIntentsError(str(exc), "scoped agent commands require a reviewed public key and allowance ceiling") from None


def agent_key_options(parser):
    parser.add_argument("--agent-public-key", help="reviewed signing public key; required for prepaid agent commands")
    parser.add_argument("--agent-max-allowance-yocto", help="explicit reviewed positive gas allowance ceiling in yoctoNEAR")


def guard_deposit(step: str, *, method: Optional[str] = None, prepaid_agent_storage: bool = False) -> str:
    """Select storage payment explicitly; never infer support or credit from RPC."""
    if prepaid_agent_storage and step in GUARD_STEPS and method == AGENT_METHODS[step]:
        return "0 NEAR"
    return GUARD_DEPOSITS[step]


def _unix_ms(text: str) -> int:
    return int(parse_timestamp(text).timestamp() * 1000)


def _u128(value: Any) -> Optional[str]:
    return None if value is None else str(int(value))


def policy_for(leg: Dict[str, Any], request: Dict[str, Any], program_sha256: str) -> Dict[str, Any]:
    """The x3-guard `Policy` for a lowered leg and its quote request."""
    return {
        "program_sha256": program_sha256,
        "origin_asset": request["originAsset"],
        "destination_asset": request["destinationAsset"],
        "amount": str(leg["amount"]),
        "min_receive": _u128(leg.get("min_receive")),
        "slippage_bps": leg["slippage_bps"],
        "fee_cap_bps": leg.get("fee_cap_bps"),
        "recipient": leg["recipient"],
        "refund_to": leg["refund_to"],
        "deadline_ms": _unix_ms(leg["deadline"]),
    }


def guard_method(receipt: Dict[str, Any], step: str) -> str:
    owner = (receipt.get("leg") or {}).get("mandate_owner")
    return AGENT_METHODS[step] if owner else GUARD_METHODS[step]


def guard_args(receipt: Dict[str, Any], step: str) -> Dict[str, Any]:
    """Arguments for one x3-guard call; an agent's receipt gets the agent methods' `owner`."""
    args = _guard_args(receipt, step)
    owner = (receipt.get("leg") or {}).get("mandate_owner")
    return {"owner": owner, **args} if owner else args


def _guard_args(receipt: Dict[str, Any], step: str) -> Dict[str, Any]:
    """The JSON arguments for one `x3-guard` call, built from a live-quote receipt.

    Built from the receipt rather than retyped, so the policy the contract stores is the one
    this adapter lowered and the quote facts are the ones it verified.
    """
    if receipt.get("kind") != RECEIPT_KIND:
        raise NearIntentsError("X3_NEAR_RECEIPT", f"not an {RECEIPT_KIND} receipt")
    agent = bool((receipt.get("leg") or {}).get("mandate_owner"))
    steps = AGENT_STEPS if agent else GUARD_STEPS
    if step not in steps:
        raise NearIntentsError("X3_NEAR_GUARD_STEP", f"step must be one of {', '.join(steps)}"
                               + ("" if agent else "; `approve` is for an intent under a mandate"))
    leg, request, response = receipt["leg"], receipt["quote_request"], receipt["quote_response"]
    nonce = leg.get("nonce")
    if not nonce:
        raise NearIntentsError(
            "X3_NEAR_NONCE",
            "x3-guard keys every intent by its nonce; add `require nonce unused <id>` to the program",
            "requires.nonce",
        )
    if step == "register":
        return {"nonce": nonce, "policy": policy_for(leg, request, receipt["program_sha256"])}
    if step == "approve":
        return {"nonce": nonce}
    if step == "quote":
        # The contract verifies 1Click's signature itself and reads every value from these
        # bytes, so nothing here is a claim the caller could edit.
        if response.get("quoteRequest", {}).get("dry"):
            raise NearIntentsError("X3_NEAR_RECEIPT", "the receipt's quote is a dry quote; only a live quote can be accepted")
        return {"nonce": nonce, "signed_payload": signed_quote_payload(response), "signature": response["signature"]}
    terminal = [s for s in receipt.get("status_history") or [] if s.get("terminal")]
    if not terminal:
        raise NearIntentsError(
            "X3_NEAR_NOT_TERMINAL",
            "the receipt records no terminal status yet; run `status --wait --receipt` first",
        )
    last = terminal[-1]
    if last["status"] == "SUCCESS":
        # No payout-transaction/quote linkage verifier is implemented here.
        # Neither unsigned status nor editable receipt flags can unlock this.
        raise NearIntentsError(
            "X3_EVIDENCE_UNSUPPORTED",
            "settled outcome recording requires a payout transaction verifier; "
            "provider SUCCESS and a balance increase are not settlement proof",
        )
    hashes = [h for h in (last.get("origin_tx") or []) + (last.get("destination_tx") or []) + (last.get("near_tx") or []) if h]
    refund = (receipt.get("evidence") or {}).get("refund") or {}
    verified_refund_amount = None
    if last["status"] == "REFUNDED":
        deposit = (receipt.get("evidence") or {}).get("deposit") or {}
        raw_amount = refund.get("observed")
        try:
            amount = int(raw_amount)
        except (TypeError, ValueError):
            amount = 0
        if not deposit.get("verified") or not refund.get("verified") or amount <= 0:
            raise NearIntentsError("X3_EVIDENCE_BINDING",
                                   "refund outcome requires a proven deposit and a positive, verified source-chain refund")
        verified_refund_amount = str(amount)
    return {
        "nonce": nonce,
        "status": _STATUS_TO_GUARD[last["status"]],
        "amount_out": None,
        "refunded_amount": verified_refund_amount if last["status"] == "REFUNDED" else None,
        "tx_hashes": hashes[:MAX_GUARD_TX_HASHES],
    }


def guard_command(step: str, args: Dict[str, Any], *, contract: str, signer: str, key_file: str,
                  network: str = "mainnet", method: Optional[str] = None,
                  prepaid_agent_storage: bool = False, agent_public_key: Optional[str] = None,
                  agent_max_allowance_yocto: Optional[str] = None) -> List[str]:
    """Generate an operator command; scoped calls validate again immediately before sending."""
    if prepaid_agent_storage and step in GUARD_STEPS and method == AGENT_METHODS[step]:
        validate_agent_key_pin(agent_public_key, agent_max_allowance_yocto)
        return [sys.executable, os.path.join(os.path.dirname(os.path.abspath(__file__)), "near_agent_key.py"),
                "send", "--account", signer, "--public-key", agent_public_key,
                "--contract", contract, "--method", method, "--max-allowance-yocto", agent_max_allowance_yocto,
                "--network", network, "--key-file", key_file,
                "--args-json", json.dumps(args, separators=(",", ":"), sort_keys=True)]
    return [
        "near", "contract", "call-function", "as-transaction", contract, method or GUARD_METHODS[step],
        "json-args", json.dumps(args, separators=(",", ":"), sort_keys=True),
        "prepaid-gas", GUARD_GAS,
        "attached-deposit", guard_deposit(step, method=method, prepaid_agent_storage=prepaid_agent_storage),
        "sign-as", signer,
        "network-config", network,
        "sign-with-access-key-file", key_file,
        "send",
    ]


def prepare_handoff(receipt: Dict[str, Any], *, receipt_path: str, program_path: str,
                    contract: Optional[str] = None,
                    signer: Optional[str] = None, key_file: Optional[str] = None,
                    network: str = "mainnet", prepaid_agent_storage: bool = False,
                    agent_public_key: Optional[str] = None,
                    agent_max_allowance_yocto: Optional[str] = None) -> Dict[str, Any]:
    """Prepare Guard calls and post-deposit evidence commands without signing or broadcasting."""
    _signed_quote(receipt)
    if receipt.get("program_sha256") != _sha256_file(program_path):
        raise NearIntentsError(
            "X3_EVIDENCE_BINDING",
            "the receipt's program SHA-256 does not match the supplied X3 program",
            "program_sha256",
        )
    response = receipt.get("quote_response")
    valid, reason = check_oneclick_quote_signature(response)
    signed_request = (response or {}).get("quoteRequest") or {}
    saved_request = receipt.get("quote_request") or {}
    mismatched = [field for field in _ECHO_FIELDS if not _same(field, saved_request.get(field), signed_request.get(field))]
    if (not valid
            or signed_request.get("dry") is not False
            or mismatched
            or receipt.get("signed_payload") != signed_quote_payload(response)
            or receipt.get("quote_hash") != oneclick_quote_hash(response)
            or receipt.get("quote_response_sha256") != _sha256_json(response)):
        raise NearIntentsError(
            "X3_EVIDENCE_BINDING",
            f"the live quote receipt is not authenticated and request-bound: {reason or ', '.join(mismatched)}",
            "quote_response",
        )
    # The nonce and other non-1Click program fields are not signed by the provider. Recompile
    # the owner-supplied source and require every such field to match the receipt.
    _, compiled_leg = compile_program(program_path, now=parse_timestamp(response["timestamp"]))
    stable_fields = (
        "intent", "origin", "destination", "amount", "recipient", "refund_to",
        "min_receive", "slippage_bps", "fee_cap_bps", "nonce",
    )
    changed = [field for field in stable_fields if receipt.get("leg", {}).get(field) != compiled_leg.get(field)]
    if changed:
        raise NearIntentsError(
            "X3_EVIDENCE_BINDING",
            f"receipt leg differs from the compiled program: {', '.join(changed)}",
            "leg",
        )
    steps = []
    for step in ("register", "quote"):
        call_args = guard_args(receipt, step)
        method = guard_method(receipt, step)
        item: Dict[str, Any] = {
            "step": step,
            "method": method,
            "args": call_args,
            "attached_deposit": guard_deposit(step, method=method, prepaid_agent_storage=prepaid_agent_storage),
            "gas": GUARD_GAS,
            "command": None,
        }
        if contract and signer and key_file:
            import shlex
            item["command"] = shlex.join(guard_command(
                step, call_args, contract=contract, signer=signer, key_file=key_file,
                network=network, method=method, prepaid_agent_storage=prepaid_agent_storage,
                agent_public_key=agent_public_key, agent_max_allowance_yocto=agent_max_allowance_yocto,
            ))
        steps.append(item)
    deposit = receipt.get("deposit") or {}
    return {
        "schema": "x3.near_intents.handoff.v1",
        "status": "prepared_not_authorized",
        "broadcasted": False,
        "wallet_signature_required": True,
        "authorization": "not_verified",
        "guard_steps": steps,
        "deposit": {
            "address": deposit.get("address"),
            "asset": deposit.get("asset"),
            "amount": deposit.get("amount"),
            "deadline": deposit.get("deadline"),
        },
        "tracking_commands": [
            f"python3 x3-lang/near_intents.py submit-deposit --deposit-address {deposit.get('address')} "
            f"--tx-hash <origin-transaction-hash> --receipt {receipt_path}",
            f"python3 x3-lang/near_intents.py status --deposit-address {deposit.get('address')} "
            f"--wait --receipt {receipt_path}",
            f"python3 x3-lang/near_intents.py verify-deposit --receipt {receipt_path}",
            f"python3 x3-lang/near_intents.py verify-settlement --receipt {receipt_path}",
        ],
        "not_claimed": [
            "Guard calls were not sent",
            "the wallet did not sign",
            "no funds moved",
            "settlement is not proven",
        ],
    }


# --------------------------------------------------------------------------------------------
# Chain evidence: did the money actually move?
# --------------------------------------------------------------------------------------------

EXIT_UNVERIFIED = 5


def near_views_for(rpc: NearRpc):
    urls = None if os.environ.get("NEAR_RPC_URLS") else [rpc.url]
    return chain_evidence.NearViews(transport=rpc.transport, urls=urls)


def solana_rpc_default():
    return chain_evidence.SolanaRpc(transport=urllib_transport)


def bsc_rpc_default():
    return chain_evidence.BscRpc(transport=urllib_transport)


def _signed_quote(receipt: Dict[str, Any]) -> Dict[str, Any]:
    """Return an authenticated live quote after binding every editable receipt field."""
    response = receipt.get("quote_response")
    if not isinstance(response, dict):
        raise NearIntentsError("X3_EVIDENCE_BINDING", "receipt has no quote response")
    valid, reason = check_oneclick_quote_signature(response)
    if not valid:
        raise NearIntentsError("X3_NEAR_INVALID_SIGNATURE", f"receipt quote signature failed: {reason}")
    signed_request = response.get("quoteRequest") or {}
    saved_request = receipt.get("quote_request") or {}
    mismatched = [
        field for field in _ECHO_FIELDS
        if not _same(field, saved_request.get(field), signed_request.get(field))
    ]
    if (signed_request.get("dry") is not False
            or saved_request.get("dry") is not False
            or mismatched
            or receipt.get("signed_payload") != signed_quote_payload(response)
            or receipt.get("quote_hash") != oneclick_quote_hash(response)
            or receipt.get("quote_response_sha256") != _sha256_json(response)):
        raise NearIntentsError(
            "X3_EVIDENCE_BINDING",
            f"receipt quote is not authenticated and request-bound: {', '.join(mismatched) or 'digest mismatch'}",
            "quote_response",
        )
    quote_obj = response.get("quote") or {}
    if not isinstance(quote_obj, dict):
        raise NearIntentsError("X3_EVIDENCE_BINDING", "receipt quote response has no quote object")
    deposit = receipt.get("deposit") or {}
    if (deposit.get("address") != quote_obj.get("depositAddress")
            or str(deposit.get("amount")) != str(quote_obj.get("amountIn"))
            or not deposit.get("deadline")
            or not quote_obj.get("deadline")
            or parse_timestamp(deposit["deadline"]) != parse_timestamp(quote_obj["deadline"])):
        raise NearIntentsError("X3_EVIDENCE_BINDING",
                               "receipt deposit metadata differs from the signed live quote")
    leg = receipt.get("leg") or {}
    if (str(leg.get("amount")) != str(saved_request.get("amount"))
            or leg.get("recipient") != saved_request.get("recipient")
            or leg.get("refund_to") != saved_request.get("refundTo")
            or not leg.get("deadline")
            or parse_timestamp(leg["deadline"]) != parse_timestamp(saved_request["deadline"])):
        raise NearIntentsError("X3_EVIDENCE_BINDING",
                               "receipt leg differs from the request signed by 1Click")
    tokens = receipt.get("tokens") or {}
    if ((tokens.get("origin") or {}).get("assetId") != saved_request.get("originAsset")
            or (tokens.get("destination") or {}).get("assetId") != saved_request.get("destinationAsset")):
        raise NearIntentsError("X3_EVIDENCE_BINDING",
                               "receipt token metadata differs from the request signed by 1Click")
    origin_chain = (leg.get("origin") or {}).get("chain")
    if not origin_chain or not registry.is_valid_receiver(origin_chain, quote_obj.get("depositAddress")):
        raise NearIntentsError("X3_EVIDENCE_BINDING",
                               "signed quote deposit address is not valid for the origin chain")
    if saved_request.get("dry", True):
        raise NearIntentsError("X3_EVIDENCE_UNSUPPORTED", "this receipt is for a dry quote; nothing was deposited")
    return quote_obj


def _solana_mint(receipt: Dict[str, Any]) -> str:
    leg, origin = receipt["leg"], (receipt.get("tokens") or {}).get("origin") or {}
    if leg["origin"]["chain"] != "solana":
        raise NearIntentsError("X3_EVIDENCE_UNSUPPORTED", f"deposit evidence is implemented for Solana origins, not {leg['origin']['chain']}")
    # The provider signature covers the asset ID, not editable token metadata.
    # Keep this deliberately narrow: additional assets need a reviewed binding.
    # USDC binding: the existing mainnet registry mint and the archived 1Click
    # asset ID; never accept a replacement mint from a receipt or token listing.
    pinned = {
        "nep141:sol-5ce3bf3a31af18be40ba30f721101b4341690186.omft.near":
            registry.TOKEN_ADDRESSES["solana"]["USDC"],
    }
    signed_asset = receipt["quote_response"]["quoteRequest"].get("originAsset")
    mint = pinned.get(signed_asset)
    if mint is None:
        raise NearIntentsError("X3_EVIDENCE_UNSUPPORTED",
                               "no independently pinned Solana mint for this signed asset")
    if (origin.get("contractAddress") != mint or origin.get("blockchain") != "sol"
            or origin.get("assetId") != signed_asset):
        raise NearIntentsError("X3_EVIDENCE_BINDING",
                               "receipt Solana token metadata differs from the pinned signed-asset mint")
    return mint


def _record_evidence(receipt: Dict[str, Any], key: str, fn) -> Dict[str, Any]:
    """Run one evidence check; an RPC or format failure is recorded as unverified evidence."""
    try:
        evidence = fn()
    except (chain_evidence.ChainEvidenceError, helius_evidence.HeliusEvidenceError) as exc:
        evidence = {"verified": False, "error": exc.to_dict()}
    evidence["checked_at"] = format_timestamp(datetime.now(timezone.utc))
    receipt.setdefault("evidence", {})[key] = evidence
    conclude(receipt)
    return evidence


def verify_deposit(receipt: Dict[str, Any], *, tx_signature: Optional[str] = None,
                   solana=None, bsc=None, helius=None) -> Dict[str, Any]:
    """Check finalized BSC native transfer or Solana transfer with optional Helius witness."""
    quote_obj = _signed_quote(receipt)
    recorded_signature = (receipt.get("deposit") or {}).get("tx_hash")
    if tx_signature and recorded_signature and tx_signature != recorded_signature:
        raise NearIntentsError("X3_NEAR_RECEIPT", "receipt already tracks a different deposit transaction")
    signature = tx_signature or recorded_signature
    if not signature:
        raise NearIntentsError("X3_EVIDENCE_TX", "no deposit transaction: pass --tx, or record it with submit-deposit --receipt")
    origin_chain = receipt["leg"]["origin"]["chain"]
    if origin_chain == "bsc":
        bsc = bsc or bsc_rpc_default()
        return _record_evidence(receipt, "deposit", lambda: bsc.verify_native_transfer(
            tx_hash=signature,
            recipient=quote_obj["depositAddress"],
            min_amount=int(quote_obj["amountIn"]),
            not_after_ms=int(parse_timestamp(quote_obj["deadline"]).timestamp() * 1000),
            purpose="deposit",
        ))
    mint = _solana_mint(receipt)
    solana = solana or solana_rpc_default()
    def check():
        canonical = solana.verify_transfer(
            signature=signature, mint=mint, owner=quote_obj["depositAddress"],
            min_amount=int(quote_obj["amountIn"]),
            not_after_ms=int(parse_timestamp(quote_obj["deadline"]).timestamp() * 1000),
            purpose="deposit")
        # Never allow a parser API to overrule a failed finalized Solana RPC proof.
        if helius is not None and helius is not False and canonical.get("verified"):
            witness = helius_evidence.ParsedEventsWitness() if helius is True else helius
            canonical["helius"] = witness.attest(
                signature=signature, mint=mint, owner=quote_obj["depositAddress"],
                min_amount=int(quote_obj["amountIn"]), canonical=canonical)
            canonical["verified"] = bool(canonical["helius"].get("verified"))
        return canonical

    return _record_evidence(receipt, "deposit", check)


def verify_settlement(receipt: Dict[str, Any], *, refund_tx: Optional[str] = None, near_views=None,
                      solana=None, bsc=None) -> Dict[str, Any]:
    """Observe NEAR balance movement (not settlement proof), or an origin-chain refund."""
    quote_obj = _signed_quote(receipt)
    leg = receipt["leg"]
    if refund_tx:
        # A local receipt's leg/refund target is editable. Authenticate the live
        # provider quote and derive the recipient only from its signed request.
        response = receipt.get("quote_response")
        valid, reason = check_oneclick_quote_signature(response)
        if not valid:
            raise NearIntentsError("X3_NEAR_INVALID_SIGNATURE",
                                   f"refund quote signature failed: {reason}")
        signed_request = response.get("quoteRequest") or {}
        saved_request = receipt.get("quote_request") or {}
        refund_target = signed_request.get("refundTo")
        if (signed_request.get("dry") is not False
                or signed_request.get("refundType") != "ORIGIN_CHAIN"
                or not isinstance(refund_target, str) or not refund_target
                or saved_request.get("refundTo") != refund_target
                or leg.get("refund_to") != refund_target
                or saved_request.get("originAsset") != signed_request.get("originAsset")
                or receipt.get("signed_payload") != signed_quote_payload(response)
                or receipt.get("quote_hash") != oneclick_quote_hash(response)
                or receipt.get("quote_response_sha256") != _sha256_json(response)):
            raise NearIntentsError("X3_EVIDENCE_BINDING",
                                   "refund destination or quote receipt differs from the authenticated live quote")
        # A numeric zero is not signed in 1Click's live-quote format: the
        # provider only includes truthy optional refund fees in the Ed25519
        # payload. Refuse zero, negative, malformed and excessive values.
        try:
            fee_value = quote_obj.get("refundFee")
            if isinstance(fee_value, bool) or fee_value is None:
                raise ValueError("missing refund fee")
            fee = int(fee_value)
            amount_in = int(quote_obj["amountIn"])
        except (TypeError, ValueError, KeyError) as exc:
            raise NearIntentsError("X3_EVIDENCE_BINDING", "invalid signed refund amount or fee") from exc
        if amount_in <= 0 or not (0 < fee < amount_in):
            raise NearIntentsError("X3_EVIDENCE_BINDING",
                                   "refund fee must be positive, signed, and below the deposited amount")
        original = ((receipt.get("evidence") or {}).get("deposit") or {})
        recorded_tx = (receipt.get("deposit") or {}).get("tx_hash")
        original_tx = original.get("signature") or original.get("transaction_hash")
        if (not original.get("verified") or not isinstance(original_tx, str)
                or not original_tx or (recorded_tx and recorded_tx != original_tx)):
            raise NearIntentsError("X3_EVIDENCE_BINDING",
                                   "refund requires an independently verified deposit transaction")
        # A locally saved 'verified' field is not permanent proof. Re-read the
        # actual finalized origin-chain deposit against THIS signed quote, including
        # its quoted address, amount, asset and deadline, before refund evidence.
        try:
            if receipt["leg"]["origin"]["chain"] == "bsc":
                bsc = bsc or bsc_rpc_default()
                fresh_deposit = bsc.verify_native_transfer(
                    tx_hash=original_tx,
                    recipient=quote_obj["depositAddress"],
                    min_amount=amount_in,
                    not_after_ms=int(parse_timestamp(quote_obj["deadline"]).timestamp() * 1000),
                    purpose="deposit",
                )
            else:
                mint = _solana_mint(receipt)
                solana = solana or solana_rpc_default()
                fresh_deposit = solana.verify_transfer(
                    signature=original_tx, mint=mint, owner=quote_obj["depositAddress"],
                    min_amount=amount_in,
                    not_after_ms=int(parse_timestamp(quote_obj["deadline"]).timestamp() * 1000),
                    purpose="deposit")
        except chain_evidence.ChainEvidenceError as exc:
            raise NearIntentsError("X3_EVIDENCE_BINDING",
                                   "could not independently recheck the original deposit") from exc
        if receipt["leg"]["origin"]["chain"] == "bsc":
            if (not fresh_deposit.get("verified")
                    or type(fresh_deposit.get("blockNumber")) is not int
                    or fresh_deposit["blockNumber"] < 0
                    or type(original.get("blockNumber")) is not int
                    or fresh_deposit.get("transaction_hash") != original_tx
                    or fresh_deposit.get("blockNumber") != original.get("blockNumber")
                    or fresh_deposit.get("recipient") != original.get("recipient")):
                raise NearIntentsError("X3_EVIDENCE_BINDING",
                                       "original deposit evidence does not match this signed live quote")
        elif (not fresh_deposit.get("verified")
              or type(fresh_deposit.get("slot")) is not int
              or fresh_deposit["slot"] < 0
              or type(original.get("slot")) is not int
              or fresh_deposit.get("signature") != original_tx
              or fresh_deposit.get("slot") != original.get("slot")
              or fresh_deposit.get("owner") != original.get("owner")
              or fresh_deposit.get("mint") != original.get("mint")):
            raise NearIntentsError("X3_EVIDENCE_BINDING",
                                   "original deposit evidence does not match this signed live quote")
        expected = amount_in - fee
        if receipt["leg"]["origin"]["chain"] == "bsc":
            return _record_evidence(receipt, "refund", lambda: bsc.verify_native_transfer(
                tx_hash=refund_tx,
                recipient=refund_target,
                min_amount=expected,
                not_after_ms=None,
                purpose="refund",
                after_block=fresh_deposit["blockNumber"],
            ))
        mint = _solana_mint(receipt)
        solana = solana or solana_rpc_default()
        return _record_evidence(receipt, "refund", lambda: solana.verify_transfer(
            signature=refund_tx, mint=mint, owner=refund_target, min_amount=expected,
            not_after_ms=None, purpose="refund", after_slot=fresh_deposit["slot"]))
    if leg["destination"]["chain"] != "near":
        raise NearIntentsError("X3_EVIDENCE_UNSUPPORTED", f"delivery evidence is implemented for NEAR destinations, not {leg['destination']['chain']}")
    baseline = (receipt.get("evidence") or {}).get("settlement_baseline")
    if not baseline:
        raise NearIntentsError("X3_EVIDENCE_BASELINE", "the receipt has no starting balance, so a delivery cannot be measured")
    signed_request = receipt["quote_response"]["quoteRequest"]
    token = nep141_contract(signed_request["destinationAsset"])
    account = signed_request["recipient"]
    if (baseline.get("token") != token or baseline.get("account") != account):
        raise NearIntentsError("X3_EVIDENCE_BINDING",
                               "balance baseline target differs from the signed destination")
    views = near_views or near_views_for(NearRpc())

    def observe_balance():
        observation = chain_evidence.judge_near_delivery(
            baseline, views.ft_balance(token, account), min_amount=int(quote_obj["minAmountOut"]))
        # Preserve the limited balance observation without promoting it to a
        # verified payout. The saved starting balance is not authenticated,
        # and even a genuine rise could be an unrelated incoming transfer.
        observation["balance_delta_observed"] = observation["verified"]
        observation["verified"] = False
        observation["baseline_authenticated"] = False
        observation["cross_chain_causality_proven"] = False
        observation["limitation"] = (
            "comparison against a locally saved baseline only; no authenticated "
            "baseline or payout transaction bound to this quote"
        )
        return observation

    return _record_evidence(receipt, "settlement", observe_balance)


def conclude(receipt: Dict[str, Any]) -> Dict[str, Any]:
    """Summarize available observations; no settled outcome verifier exists yet."""
    ev = receipt.get("evidence") or {}
    ok = lambda k: bool((ev.get(k) or {}).get("verified"))
    outcome = None
    # There is no payout transaction verifier in this adapter. In particular,
    # legacy or fabricated saved settlement.verified flags cannot prove it.
    if ok("deposit") and ok("refund"):
        outcome = "refunded"
    missing = [k for k in ("deposit",) if not ok(k)]
    if not ok("refund"):
        missing.append("payout transaction bound to quote (unsupported), or refund")
    reported = next((s["outcome"] for s in reversed(receipt.get("status_history") or []) if s.get("terminal")), None)
    verification = {"status": "verified" if outcome else "unverified", "outcome": outcome, "missing": missing,
                    "1click_reported": reported}
    if outcome and reported:
        verification["1click_agrees"] = reported == outcome
    receipt["verification"] = verification
    if outcome:
        receipt["verified_outcome"] = outcome
    else:
        receipt.pop("verified_outcome", None)
    return verification


def _load_receipt(path: str) -> Dict[str, Any]:
    with open(path, "r", encoding="utf-8") as f:
        receipt = json.load(f)
    if receipt.get("kind") != RECEIPT_KIND:
        raise NearIntentsError("X3_NEAR_RECEIPT", f"{path} is not an {RECEIPT_KIND} receipt")
    return receipt


def write_receipt(path: str, receipt: Dict[str, Any]) -> None:
    _write_private_json(path, receipt)


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
    with _locked_path(receipt_path):
        receipt = _load_receipt(receipt_path)
        receipt.setdefault("status_history", []).append(observed)
        receipt["outcome"] = observed["outcome"]
        write_receipt(receipt_path, receipt)


# --------------------------------------------------------------------------------------------
# CLI
# --------------------------------------------------------------------------------------------

def _emit(payload: Dict[str, Any], code: int = 0) -> int:
    print(json.dumps(payload, indent=2, sort_keys=True))
    return code


def _write_private_json(path: str, payload: Dict[str, Any]) -> None:
    """Write judge evidence atomically with owner-only permissions."""
    absolute = os.path.abspath(path)
    os.makedirs(os.path.dirname(absolute), exist_ok=True)
    tmp = f"{absolute}.tmp.{os.getpid()}"
    fd = os.open(tmp, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as handle:
            json.dump(payload, handle, indent=2, sort_keys=True)
            handle.write("\n")
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(tmp, absolute)
    finally:
        with contextlib.suppress(FileNotFoundError):
            os.unlink(tmp)


def main(argv: Optional[List[str]] = None, *, client: Optional[OneClickClient] = None,
         rpc: Optional[NearRpc] = None, near_views: Optional[Any] = None, solana: Optional[Any] = None,
         bsc: Optional[Any] = None) -> int:
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
    p_quote.add_argument("--mandate", help="the owner's mandate file, for an intent `under mandate <owner>`")
    p_quote.add_argument("--guard-contract", help="read the mandate's live state from this x3-guard account")
    p_quote.add_argument("--out", help="write the quote result, or structured failure evidence, to this private file")

    p_submit = sub.add_parser("submit-deposit", help="tell 1Click the deposit transaction hash")
    p_submit.add_argument("--deposit-address", required=True)
    p_submit.add_argument("--tx-hash", required=True)
    p_submit.add_argument("--receipt", required=True,
                          help="live quote receipt; verify finalized Solana transfer before notifying 1Click")

    p_vdep = sub.add_parser("verify-deposit", help="prove from the origin chain that the deposit reached 1Click")
    p_vdep.add_argument("--receipt", required=True)
    p_vdep.add_argument("--tx", help="deposit transaction hash/signature (default: the one submit-deposit recorded)")
    p_vdep.add_argument("--helius", action="store_true",
                        help="require Helius Parsed Events for finalized Solana deposits (needs HELIUS_API_KEY)")

    p_vset = sub.add_parser("verify-settlement", help="prove the delivery on NEAR, or a refund on the origin chain")
    p_vset.add_argument("--receipt", required=True)
    p_vset.add_argument("--refund-tx", help="origin-chain transaction hash/signature, when the swap was refunded")

    p_prepare = sub.add_parser("prepare", help="prepare Guard calls and tracking commands without broadcasting")
    p_prepare.add_argument("--receipt", required=True)
    p_prepare.add_argument("--program", required=True, help="the exact .x3 source used for the live quote")
    p_prepare.add_argument("--contract")
    p_prepare.add_argument("--signer")
    p_prepare.add_argument("--key-file")
    p_prepare.add_argument("--network", default="mainnet")
    p_prepare.add_argument("--prepaid-agent-storage", action="store_true", help=PREPAID_AGENT_STORAGE_HELP)
    agent_key_options(p_prepare)

    p_guard = sub.add_parser("guard-args", help="x3-guard contract call arguments (and command) from a receipt")
    p_guard.add_argument("--receipt", required=True)
    p_guard.add_argument("--step", required=True, choices=AGENT_STEPS)
    p_guard.add_argument("--contract", help="x3-guard account; with --signer and --key-file, prints the near CLI command")
    p_guard.add_argument("--signer")
    p_guard.add_argument("--key-file")
    p_guard.add_argument("--network", default="mainnet")
    p_guard.add_argument("--prepaid-agent-storage", action="store_true", help=PREPAID_AGENT_STORAGE_HELP)
    agent_key_options(p_guard)

    p_mandate = sub.add_parser("mandate", help="an owner's mandate for an AI agent, enforced by x3-guard")
    msub = p_mandate.add_subparsers(dest="mandate_command", required=True)
    for name, text in (("args", "create_mandate arguments (and command) from a mandate file"),
                       ("revoke", "revoke_mandate arguments (and command): stops the agent at once")):
        mp = msub.add_parser(name, help=text)
        mp.add_argument("input")
        mp.add_argument("--tokens")
        mp.add_argument("--contract")
        mp.add_argument("--key-file", help="the owner's key; with --contract, prints the near CLI command")
        mp.add_argument("--network", default="mainnet")
    mc = msub.add_parser("check", help="check an agent's intent against a mandate before quoting")
    mc.add_argument("mandate")
    mc.add_argument("intent")
    mc.add_argument("--tokens")
    mc.add_argument("--guard-contract", help="check against the mandate this x3-guard holds on chain")

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
            try:
                result = quote(args.input, client=client, tokens_path=args.tokens, live=args.live,
                               nonce_store=NonceStore(args.nonce_store), receipt_path=args.receipt, rpc=rpc,
                               near_views=near_views, mandate_path=args.mandate, guard_contract=args.guard_contract)
            except NearIntentsError as exc:
                if args.out:
                    request = (exc.detail or {}).get("quote_request") if isinstance(exc.detail, dict) else None
                    planned = None
                    if request is None:
                        with contextlib.suppress(NearIntentsError, OSError, ValueError, TypeError):
                            planned = plan(args.input, client=client, tokens_path=args.tokens, live=args.live)
                            request = planned["quote_request"]
                    _write_private_json(args.out, {
                        "schema": "x3.near_intents.quote-evidence.v1",
                        "status": "error",
                        "input": args.input,
                        "input_sha256": _sha256_file(args.input),
                        "mode": "live" if args.live else "dry",
                        "authenticated": client.authenticated,
                        "quote_request": request,
                        "error": exc.to_dict(),
                    })
                raise
            if args.out:
                _write_private_json(args.out, {
                    "schema": "x3.near_intents.quote-evidence.v1",
                    "status": result["status"],
                    "input": args.input,
                    "input_sha256": _sha256_file(args.input),
                    "authenticated": client.authenticated,
                    **result,
                })
            return _emit(result)
        if args.command == "submit-deposit":
            # A 1Click notification is an external side effect. Never notify it about a
            # transaction that the live quote did not authorize or that Solana has not
            # finalized to the exact signed deposit address, mint, and amount.
            with _locked_path(args.receipt):
                receipt = _load_receipt(args.receipt)
                signed = _signed_quote(receipt)
                response_data = receipt.get("quote_response")
                valid, reason = check_oneclick_quote_signature(response_data)
                if not valid:
                    raise NearIntentsError("X3_NEAR_INVALID_SIGNATURE",
                                           f"receipt's 1Click signature did not verify: {reason}")
                if (receipt.get("quote_response_sha256") != _sha256_json(response_data)
                        or receipt.get("quote_hash") != oneclick_quote_hash(response_data)
                        or receipt.get("signed_payload") != signed_quote_payload(response_data)):
                    raise NearIntentsError("X3_NEAR_RECEIPT",
                                           "receipt differs from the original signed quote")
                recorded = receipt.get("deposit") or {}
                if (recorded.get("amount") != signed.get("amountIn")
                        or recorded.get("deadline") != signed.get("deadline")
                        or recorded.get("asset") != (receipt.get("quote_request") or {}).get("originAsset")):
                    raise NearIntentsError("X3_NEAR_RECEIPT",
                                           "recorded deposit details differ from the signed quote")
                if (recorded.get("address") != args.deposit_address
                        or signed.get("depositAddress") != args.deposit_address):
                    raise NearIntentsError("X3_NEAR_RECEIPT",
                                           "that deposit address does not match this receipt's signed quote")
                if recorded.get("tx_hash") and recorded["tx_hash"] != args.tx_hash:
                    raise NearIntentsError("X3_NEAR_RECEIPT",
                                           "this receipt already binds to a different deposit transaction")
                evidence = verify_deposit(receipt, tx_signature=args.tx_hash, solana=solana)
                if not evidence.get("verified"):
                    raise NearIntentsError("X3_EVIDENCE_UNVERIFIED",
                                           "a finalized, correct Solana transfer must be proven before notifying 1Click",
                                           "deposit", evidence)
                if recorded.get("tx_hash"):
                    return _emit({"status": "already_tracking", "deposit_verified": True})
                # Save the proven signature before notifying 1Click. A lost response
                # leaves the notification uncertain, but the deposit stays bound.
                receipt["deposit"]["tx_hash"] = args.tx_hash
                write_receipt(args.receipt, receipt)
                response = client.submit_deposit(args.deposit_address, args.tx_hash)
                return _emit({"status": "submitted", "response": response,
                              "deposit_verified": True})
        if args.command in ("verify-deposit", "verify-settlement"):
            with _locked_path(args.receipt):
                receipt = _load_receipt(args.receipt)
                if args.command == "verify-deposit":
                    evidence = verify_deposit(receipt, tx_signature=args.tx, solana=solana,
                                              bsc=bsc, helius=args.helius)
                else:
                    evidence = verify_settlement(receipt, refund_tx=args.refund_tx, near_views=near_views,
                                                 solana=solana, bsc=bsc)
                write_receipt(args.receipt, receipt)
                verification = receipt["verification"]
                if args.command == "verify-deposit":
                    code = 0 if evidence.get("verified") else EXIT_UNVERIFIED
                else:
                    code = {"settled": 0, "refunded": 2}.get(verification["outcome"], EXIT_UNVERIFIED)
                return _emit({"evidence": evidence, "verification": verification}, code)
        if args.command == "prepare":
            receipt = _load_receipt(args.receipt)
            return _emit(prepare_handoff(
                receipt,
                receipt_path=args.receipt,
                program_path=args.program,
                contract=args.contract,
                signer=args.signer,
                key_file=args.key_file,
                network=args.network,
                prepaid_agent_storage=args.prepaid_agent_storage,
                agent_public_key=args.agent_public_key, agent_max_allowance_yocto=args.agent_max_allowance_yocto,
            ))
        if args.command == "guard-args":
            with open(args.receipt, "r", encoding="utf-8") as f:
                receipt = json.load(f)
            call_args = guard_args(receipt, args.step)
            method = guard_method(receipt, args.step)
            out: Dict[str, Any] = {"method": method, "args": call_args,
                                   "attached_deposit": guard_deposit(args.step, method=method,
                                                                     prepaid_agent_storage=args.prepaid_agent_storage),
                                   "gas": GUARD_GAS}
            if args.contract and args.signer and args.key_file:
                import shlex
                out["command"] = shlex.join(guard_command(args.step, call_args, contract=args.contract,
                                                          signer=args.signer, key_file=args.key_file,
                                                          network=args.network, method=method,
                                                          prepaid_agent_storage=args.prepaid_agent_storage,
                                                          agent_public_key=args.agent_public_key,
                                                          agent_max_allowance_yocto=args.agent_max_allowance_yocto))
            return _emit(out)
        if args.command == "mandate":
            if args.mandate_command == "check":
                intent, leg = compile_program(args.intent)
                tokens = load_tokens(client, args.tokens)
                request = build_quote_request(leg, tokens, dry=True)
                with open(args.intent, "rb") as f:
                    intent_sha = hashlib.sha256(f.read()).hexdigest()
                views = (near_views or near_views_for(rpc or NearRpc())) if args.guard_contract else None
                try:
                    checked = check_against_mandate(leg, request, intent_sha, args.mandate, tokens=tokens,
                                                    guard_contract=args.guard_contract, near_views=views)
                except NearIntentsError as exc:
                    if exc.code != "X3_MANDATE_DENIED" or not isinstance(exc.detail, dict):
                        raise
                    return _emit({"status": "denied", "mandate": exc.detail}, EXIT_MANDATE_DENIED)
                return _emit({"status": "allowed", "mandate": checked})
            compiled = mandates().compile_mandate(args.input, load_tokens(client, args.tokens))
            if args.mandate_command == "args":
                step, method, call_args = "mandate", "create_mandate", {"agent": compiled["agent"], "terms": compiled["terms"]}
            else:
                step, method, call_args = "revoke", "revoke_mandate", {"agent": compiled["agent"]}
            out = {"method": method, "args": call_args, "signer": compiled["owner"],
                   "attached_deposit": GUARD_DEPOSITS[step], "gas": GUARD_GAS}
            if step == "mandate":
                out["expires"] = compiled["expires"]
            if args.contract and args.key_file:
                import shlex
                out["command"] = shlex.join(guard_command(step, call_args, contract=args.contract,
                                                          signer=compiled["owner"], key_file=args.key_file,
                                                          network=args.network, method=method))
            return _emit(out)
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
