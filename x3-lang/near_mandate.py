"""X3 `mandate`: what an owner lets an AI agent spend, enforced by the x3-guard contract.

An owner writes one `mandate` and an agent writes ordinary NEAR Intents programs that say
`under mandate <owner>`. The mandate compiles to the arguments of x3-guard's `create_mandate`;
from then on the contract refuses any intent of that agent that breaks a term, and any quote
the agent tries to accept once the mandate is revoked, replaced or expired.

    mandate grocery_agent {
        owner alice.near
        agent shopper.alice.near
        spend Solana.USDC
        per_payment 50000000                 // base units: 50 USDC
        per_24h 200000000                    // any rolling 24 hours: 200 USDC
        allow recipients store.near, bob.near
        allow assets Near.USDC
        refund_to 4Nd1mzi8Y1QYxJt9wZWBYZpG7S4pYkZs6YzD3Vt9aBcD
        require slippage <= 1%
        require fees <= 100 bps
        approve above 20000000 by 2 of alice.near, carol.near
        expires 30d
    }

| X3 | x3-guard `MandateTerms` |
|---|---|
| `owner`, `agent` | the caller of `create_mandate`, and its `agent` argument |
| `spend <chain>.<asset>` | `asset` (1Click ID resolved from `/v0/tokens`) |
| `per_payment N`, `per_24h N` | `per_payment_cap`, `daily_cap` (rolling 24 h, at most 64 payments) |
| `allow recipients …`, `allow assets …` | `recipients`, `destination_assets` |
| `refund_to R` | `refund_to`: every refund goes here, whatever the agent writes |
| `require slippage <= x`, `require fees <= y` | `max_slippage_bps`, `max_fee_bps` (both mandatory) |
| `approve above N by K of a, b, …` | `approval_above`, `approval_threshold`, `approvers` |
| `expires D` | `expires_at_ms` = compile time + D |

The parser is strict: a line it does not know is an error, not ignored. Every check here is
the contract's own (`x3-lang/near/x3-guard/src/mandate.rs`, `check_mandate`), in the same
order and with the same names, so an agent learns before it spends gas what the chain will say.
"""

from __future__ import annotations

import hashlib
import re
from datetime import datetime, timedelta, timezone
from typing import Any, Dict, List, Optional

WINDOW_MS = 86_400_000
MAX_PAYMENTS_PER_WINDOW = 64
MAX_LIST = 16
MAX_APPROVERS = 8
BPS = 10_000
U128_MAX = (1 << 128) - 1
# Shared with the browser: the last millisecond of a four-digit UTC year. This
# representability bound is stricter than the contract's u64 expiry storage.
MAX_EXPIRY_MS = 253_402_300_799_999
_EPOCH = datetime(1970, 1, 1, tzinfo=timezone.utc)

# Bound by `near_intents._mandates()`, so errors, units and assets mean exactly what they mean
# in the adapter (one `NearIntentsError` class, one duration and bps parser).
_ni: Any = None


def bind(adapter: Any) -> None:
    global _ni
    _ni = adapter


def _err(code: str, message: str, field: Optional[str] = None, line: Optional[int] = None):
    return _ni.NearIntentsError(code, message, field, {"line": line} if line else None)


def _list(text: str) -> List[str]:
    return [item.strip().strip('"') for item in text.split(",") if item.strip()]


def parse_mandate(path: str) -> Dict[str, Any]:
    """Read one `mandate` declaration. Any line it does not recognise is refused."""
    cli = _ni.cli_module()
    lines = cli._clean_lines(path)
    if not lines:
        raise _err("X3_MANDATE_PARSE", "the file is empty")
    head = re.fullmatch(r"mandate\s+([A-Za-z_][A-Za-z0-9_-]*)\s*\{", lines[0].text)
    if not head:
        raise _err("X3_MANDATE_PARSE", "expected `mandate <name> {` as the first line", "mandate", lines[0].no)
    out: Dict[str, Any] = {"name": head.group(1), "requires": [], "line_of": {}}
    closed = False
    for ln in lines[1:]:
        text = ln.text
        if closed:
            raise _err("X3_MANDATE_PARSE", f"unexpected {text!r} after the mandate's closing brace", "mandate", ln.no)
        if text == "}":
            closed = True
            continue
        words = text.split()
        key = words[0]
        if key == "require":
            try:
                req = cli._parse_require(ln)
            except cli.X3ParseError as exc:
                raise _err("X3_MANDATE_PARSE", exc.message, "require", ln.no) from exc
            if req["kind"] not in ("slippage", "fees"):
                raise _err("X3_MANDATE_PARSE", f"a mandate bounds slippage and fees; `require {req['kind']}` belongs "
                           "in the agent's intent", "require", ln.no)
            out["requires"].append(req)
            continue
        if key == "allow" and len(words) >= 3 and words[1] in ("recipients", "assets"):
            field = f"allow_{words[1]}"
            value: Any = _list(text.split(None, 2)[2])
        elif key == "approve":
            m = re.fullmatch(r"approve\s+above\s+(\S+)\s+by\s+(\d+)\s+of\s+(.+)", text)
            if not m:
                raise _err("X3_MANDATE_PARSE", "expected `approve above <amount> by <n> of <account>, …`", "approve", ln.no)
            field, value = "approve", {"above": m.group(1), "threshold": int(m.group(2)), "approvers": _list(m.group(3))}
        elif key in ("owner", "agent", "spend", "per_payment", "per_24h", "refund_to", "expires") and len(words) == 2:
            field, value = key, words[1].strip('"')
        else:
            raise _err("X3_MANDATE_PARSE", f"{text!r} is not a mandate clause", "mandate", ln.no)
        if field in out:
            raise _err("X3_MANDATE_PARSE", f"`{field.replace('_', ' ', 1) if field.startswith('allow') else field}` "
                       "appears twice", field, ln.no)
        out[field] = value
        out["line_of"][field] = ln.no
    if not closed:
        raise _err("X3_MANDATE_PARSE", "the mandate is never closed", "mandate", lines[-1].no)
    for required in ("owner", "agent", "spend", "per_payment", "per_24h", "allow_recipients", "allow_assets",
                     "refund_to", "expires"):
        if required not in out:
            raise _err("X3_MANDATE_PARSE", f"a mandate must state `{required.replace('_', ' ', 1) if required.startswith('allow') else required}`",
                       required)
    return out


_DURATION_UNITS = {"ms": 1, "s": 1_000, "m": 60_000, "h": 3_600_000, "d": 86_400_000}


def _expiry_range_error(line: Optional[int]):
    return _err("X3_MANDATE_EXPIRES", "expires must fit a UTC timestamp from 1970-01-01T00:00:00.000Z "
                "through 9999-12-31T23:59:59.999Z", "expires", line)


def parse_expiry(text: str, line: Optional[int] = None) -> timedelta:
    """`30d`, `12h`, `90m`: how long the mandate lasts. Days are allowed here, unlike a swap
    timeout; there is no unit for blocks, because the contract compares wall-clock times."""
    m = re.fullmatch(r"(\d+)(ms|s|m|h|d)", text.strip())
    if not m:
        raise _err("X3_MANDATE_EXPIRES", f"expires {text!r} must be a positive <n>ms|s|m|h|d duration", "expires", line)
    try:
        milliseconds = int(m.group(1)) * _DURATION_UNITS[m.group(2)]
        if milliseconds == 0:
            raise _err("X3_MANDATE_EXPIRES", f"expires {text!r} must be a positive <n>ms|s|m|h|d duration", "expires", line)
        if milliseconds > MAX_EXPIRY_MS:
            raise _expiry_range_error(line)
        return timedelta(milliseconds=milliseconds)
    except (OverflowError, ValueError) as exc:
        raise _expiry_range_error(line) from exc


def _chain_asset(ref: str, field: str, line: Optional[int]) -> Dict[str, str]:
    m = re.fullmatch(r"([A-Za-z][A-Za-z0-9_-]*)\.([A-Za-z][A-Za-z0-9_-]*)", ref)
    if not m:
        raise _err("X3_MANDATE_ASSET", f"{ref!r} is not a chain.asset reference", field, line)
    chain = _ni.registry.normalize_chain(m.group(1))
    if chain not in _ni.CHAIN_TO_BLOCKCHAIN:
        raise _err("X3_MANDATE_ASSET", f"{ref!r}: chain has no NEAR Intents mapping", field, line)
    return {"chain": chain, "asset": m.group(2)}


def _near_account(value: str, field: str, line: Optional[int]) -> str:
    if not _ni.registry.is_valid_near_account(value):
        raise _err("X3_MANDATE_ACCOUNT", f"{value!r} is not a NEAR account", field, line)
    return value


def _mandate_amount(value: str, field: str, line: Optional[int], *, code: str = "X3_MANDATE_LIMIT",
                    source_field: Optional[str] = None) -> int:
    # Keep the adapter's base-unit syntax, including underscores and leading zeros,
    # without letting Python's integer digit limit escape as an unstructured error.
    text = str(value).strip().replace("_", "")
    text = re.sub(r"^0+(?=[0-9])", "", text)
    try:
        amount = _ni.base_units(text, field)
    except ValueError as exc:
        raise _err(code, f"{field} must be at most {U128_MAX} (u128)", source_field or field, line) from exc
    if amount > U128_MAX:
        raise _err(code, f"{field} must be at most {U128_MAX} (u128)", source_field or field, line)
    return amount


def lower_mandate(parsed: Dict[str, Any], tokens: List[Dict[str, Any]], *, source_sha256: str,
                  now: Optional[datetime] = None) -> Dict[str, Any]:
    """The `create_mandate` call: the owner who sends it, and its `agent` and `terms` arguments."""
    now = now or datetime.now(timezone.utc)
    at = parsed["line_of"]
    owner = _near_account(parsed["owner"], "owner", at.get("owner"))
    agent = _near_account(parsed["agent"], "agent", at.get("agent"))
    if owner == agent:
        raise _err("X3_MANDATE_ACCOUNT", "the agent must be a different account from the owner", "agent", at.get("agent"))

    spend = _chain_asset(parsed["spend"], "spend", at.get("spend"))
    spend_id = _ni.resolve_asset(tokens, spend["chain"], spend["asset"])["assetId"]
    destinations = [_chain_asset(ref, "allow assets", at.get("allow_assets")) for ref in parsed["allow_assets"]]
    destination_ids = [_ni.resolve_asset(tokens, d["chain"], d["asset"])["assetId"] for d in destinations]

    per_payment = _mandate_amount(parsed["per_payment"], "per_payment", at.get("per_payment"))
    per_24h = _mandate_amount(parsed["per_24h"], "per_24h", at.get("per_24h"))
    if per_payment <= 0:
        raise _err("X3_MANDATE_LIMIT", "per_payment must be positive", "per_payment", at.get("per_payment"))
    if per_payment > per_24h:
        raise _err("X3_MANDATE_LIMIT", "per_payment must not exceed per_24h", "per_payment", at.get("per_payment"))

    recipients = parsed["allow_recipients"]
    chains = sorted({d["chain"] for d in destinations})
    for r in recipients:
        if not any(_ni.registry.is_valid_receiver(c, r) for c in chains) or r == "sender":
            raise _err("X3_MANDATE_RECIPIENT", f"{r!r} is not an address on {', '.join(chains)}", "allow recipients",
                       at.get("allow_recipients"))
    refund_to = parsed["refund_to"]
    if refund_to == "sender" or not _ni.registry.is_valid_receiver(spend["chain"], refund_to):
        raise _err("X3_MANDATE_REFUND_TO", f"{refund_to!r} is not a {spend['chain']} address", "refund_to",
                   at.get("refund_to"))
    for field, items, limit in (("allow recipients", recipients, MAX_LIST), ("allow assets", destination_ids, MAX_LIST)):
        if not items or len(items) > limit or len(set(items)) != len(items):
            raise _err("X3_MANDATE_LIST", f"{field} must list 1..{limit} distinct entries", field)

    bounds = {}
    for kind in ("slippage", "fees"):
        found = [_ni.upper_bound_bps(r, f"require {kind}") for r in parsed["requires"] if r["kind"] == kind]
        if not found:
            raise _err("X3_MANDATE_BOUND", f"a mandate must bound `require {kind} <= …`: the agent would otherwise "
                       "choose it", "require")
        bounds[kind] = min(found)
        if bounds[kind] > BPS:
            raise _err("X3_MANDATE_BOUND", f"require {kind} must bound the compiled value to at most {BPS} bps",
                       f"require {kind}")

    approve = parsed.get("approve")
    approval_above, approvers, threshold = None, [], 0
    if approve:
        approval_above = _mandate_amount(approve["above"], "approve above", at.get("approve"),
                                        code="X3_MANDATE_APPROVAL", source_field="approve")
        approvers = [_near_account(a, "approve", at.get("approve")) for a in approve["approvers"]]
        threshold = approve["threshold"]
        if approval_above >= per_payment:
            raise _err("X3_MANDATE_APPROVAL", "`approve above` must be below per_payment, or no payment could need it",
                       "approve", at.get("approve"))
        if not approvers or len(approvers) > MAX_APPROVERS or len(set(approvers)) != len(approvers):
            raise _err("X3_MANDATE_APPROVAL", f"approvers must be 1..{MAX_APPROVERS} distinct accounts", "approve",
                       at.get("approve"))
        if agent in approvers:
            raise _err("X3_MANDATE_APPROVAL", "the agent cannot approve its own payments", "approve", at.get("approve"))
        if not 1 <= threshold <= len(approvers):
            raise _err("X3_MANDATE_APPROVAL", "the threshold must be 1..the number of approvers", "approve",
                       at.get("approve"))

    try:
        expires = (now + parse_expiry(parsed["expires"], at.get("expires"))).astimezone(timezone.utc)
        since_epoch = expires - _EPOCH
    except (OverflowError, ValueError) as exc:
        raise _expiry_range_error(at.get("expires")) from exc
    # Float timestamp() rounds up near year 9999. Match the ISO string's truncation
    # to milliseconds using integer arithmetic, including at the final millisecond.
    expires_ms = (since_epoch.days * 86_400 + since_epoch.seconds) * 1000 + since_epoch.microseconds // 1000
    if not 0 <= expires_ms <= MAX_EXPIRY_MS:
        raise _expiry_range_error(at.get("expires"))
    terms = {
        "program_sha256": source_sha256,
        "asset": spend_id,
        "per_payment_cap": str(per_payment),
        "daily_cap": str(per_24h),
        "destination_assets": destination_ids,
        "recipients": recipients,
        "refund_to": refund_to,
        "max_slippage_bps": bounds["slippage"],
        "max_fee_bps": bounds["fees"],
        "approval_above": None if approval_above is None else str(approval_above),
        "approvers": approvers,
        "approval_threshold": threshold,
        "expires_at_ms": expires_ms,
    }
    return {"name": parsed["name"], "owner": owner, "agent": agent, "terms": terms,
            "expires": _ni.format_timestamp(expires)}


def compile_mandate(path: str, tokens: List[Dict[str, Any]], *, now: Optional[datetime] = None) -> Dict[str, Any]:
    with open(path, "rb") as f:
        sha = hashlib.sha256(f.read()).hexdigest()
    return lower_mandate(parse_mandate(path), tokens, source_sha256=sha, now=now)


def mandate_from_chain(view: Dict[str, Any]) -> Dict[str, Any]:
    """A `get_mandate` view result, as `check_mandate` reads it."""
    return {"owner": view["owner"], "agent": view["agent"], "terms": view["terms"], "version": view["version"],
            "revoked": bool(view["revoked"]), "spends": view.get("spends") or []}


def spent_in_window(spends: List[Dict[str, Any]], now_ms: int):
    since = max(now_ms - WINDOW_MS, 0)
    inside = [s for s in spends if int(s["at_ms"]) > since]
    return sum(int(s["amount"]) for s in inside), len(inside)


def check_mandate(mandate: Dict[str, Any], policy: Dict[str, Any], now_ms: int) -> List[Dict[str, Any]]:
    """The contract's `check_mandate`, check for check. `mandate` is a compiled mandate (no
    spends, never revoked) or a chain view from `mandate_from_chain`."""
    t = mandate["terms"]
    out: List[Dict[str, Any]] = []

    def record(guard: str, ok: bool, detail: str) -> None:
        out.append({"guard": guard, "ok": bool(ok), "detail": detail})

    revoked = bool(mandate.get("revoked"))
    record("mandate_active", not revoked and now_ms < int(t["expires_at_ms"]),
           f"version {mandate.get('version')}, revoked {revoked}, expires {t['expires_at_ms']}, now {now_ms}")
    record("mandate_asset", policy["origin_asset"] == t["asset"],
           f"spends {policy['origin_asset']}, mandate allows {t['asset']}")
    record("mandate_destination_asset", policy["destination_asset"] in t["destination_assets"],
           f"{policy['destination_asset']}, allowed {t['destination_assets']}")
    record("mandate_recipient", policy["recipient"] in t["recipients"], f"{policy['recipient']}, allowed {t['recipients']}")
    record("mandate_refund_to", policy["refund_to"] == t["refund_to"],
           f"refunds to {policy['refund_to']}, mandate pins {t['refund_to']}")
    amount = int(policy["amount"])
    record("mandate_per_payment", amount <= int(t["per_payment_cap"]), f"{amount} of at most {t['per_payment_cap']}")
    spent, count = spent_in_window(mandate.get("spends") or [], now_ms)
    record("mandate_daily", spent + amount <= int(t["daily_cap"]) and count < MAX_PAYMENTS_PER_WINDOW,
           f"{spent} spent in {count} payments over 24h, plus {amount}, cap {t['daily_cap']}")
    record("mandate_slippage", int(policy["slippage_bps"]) <= int(t["max_slippage_bps"]),
           f"{policy['slippage_bps']} bps of at most {t['max_slippage_bps']}")
    fee_cap = policy.get("fee_cap_bps")
    record("mandate_fees", fee_cap is not None and int(fee_cap) <= int(t["max_fee_bps"]),
           f"fee cap {fee_cap} bps, mandate requires at most {t['max_fee_bps']}")
    record("mandate_deadline", int(policy["deadline_ms"]) <= int(t["expires_at_ms"]),
           f"intent deadline {policy['deadline_ms']}, mandate expires {t['expires_at_ms']}")
    return out


def approvals_needed(mandate: Dict[str, Any], amount: int) -> int:
    above = mandate["terms"].get("approval_above")
    return int(mandate["terms"]["approval_threshold"]) if above is not None and amount > int(above) else 0
