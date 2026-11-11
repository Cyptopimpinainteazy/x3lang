#!/usr/bin/env python3
"""Ask 1Click for DRY quotes on the candidate mainnet routes and say which ones X3 would accept.

    python3 scripts/hackathon/route_probe.py \
        --near-account x3lang.near \
        --solana-address <your Solana wallet> \
        --base-address 0x<your Base wallet>

QUOTE ONLY. Every request is `"dry": true`: 1Click returns a price and no deposit address, so
nothing can be sent and nothing is signed. The script holds no keys and needs none.

Routes (all native USDC, 6 decimals, exact input):

    sol-near   Solana USDC -> NEAR USDC    refund to your Solana wallet, paid to your NEAR account
    near-sol   NEAR USDC   -> Solana USDC  refund to your NEAR account,  paid to your Solana wallet
    base-near  Base USDC   -> NEAR USDC    refund to your Base wallet,   paid to your NEAR account
    near-base  NEAR USDC   -> Base USDC    refund to your NEAR account,  paid to your Base wallet

Base routes run only when --base-address is given. Each token is matched in 1Click's live token
list by chain and contract address (not by symbol) and must be USDC with 6 decimals, or that
route is refused.

Each quote goes through `near_intents.verify_quote`, the same checks the real trade uses: the
1Click Ed25519 signature first, then freshness, the echoed request, the input amount, the
slippage spread, the effective cost and the refund fee against the fee cap. USABLE means every
check passed at that amount, at that moment. Quotes move: re-check right before trading.

An API key in NEAR_INTENTS_API_KEY (sent as X-API-Key) or a legacy NEAR_INTENTS_JWT (Bearer)
is used if set and never printed. Without one the unauthenticated fee schedule applies.

The full report, including every raw signed response, is written to --out (default
~/.x3/route-probe-<UTC time>.json, mode 0600). It holds public addresses and public quotes only.
"""
from __future__ import annotations

import argparse
import importlib.util
import json
import os
import re
import sys
import time
from datetime import datetime, timedelta, timezone
from decimal import Decimal
from pathlib import Path
from typing import Any, Callable, Dict, List, Optional, Tuple

ROOT = Path(__file__).resolve().parents[1].parent
X3 = ROOT / "x3-lang"


def _load_adapter():
    sys.path.insert(0, str(X3))
    spec = importlib.util.spec_from_file_location("x3_near_intents_probe", X3 / "near_intents.py")
    module = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    spec.loader.exec_module(module)
    return module


ni = _load_adapter()

USDC_DECIMALS = 6
#: Native USDC on each chain, matched against 1Click's own token list by contract address.
USDC = {
    "near": "17208628f84f5d6ad33f0da3bbbeb27ffcb398eac501a31bd6ad2011e36133a1",
    "sol": "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
    "base": "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913",
}
ROUTES = {
    "sol-near": ("sol", "near"),
    "near-sol": ("near", "sol"),
    "base-near": ("base", "near"),
    "near-base": ("near", "base"),
}
CHAIN_NAMES = {"sol": "Solana", "near": "NEAR", "base": "Base"}
DEFAULT_AMOUNTS = "1,25,50"

_NEAR_NAMED = re.compile(r"(?=.{2,64}\Z)([a-z0-9]+([-_][a-z0-9]+)*\.)*[a-z0-9]+([-_][a-z0-9]+)*\.near")
_NEAR_IMPLICIT = re.compile(r"[a-f0-9]{64}")
_BASE58 = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"
_EVM = re.compile(r"0x[0-9a-fA-F]{40}")


class ProbeError(ValueError):
    pass


def check_near_account(value: str) -> str:
    if not (_NEAR_NAMED.fullmatch(value) or _NEAR_IMPLICIT.fullmatch(value)):
        raise ProbeError(f"not a NEAR mainnet account: {value!r}")
    return value


def check_solana_address(value: str) -> str:
    if not 32 <= len(value) <= 44 or any(c not in _BASE58 for c in value):
        raise ProbeError(f"not a Solana address: {value!r}")
    n = 0
    for c in value:
        n = n * 58 + _BASE58.index(c)
    raw = n.to_bytes((n.bit_length() + 7) // 8, "big") if n else b""
    raw = b"\0" * (len(value) - len(value.lstrip("1"))) + raw
    if len(raw) != 32:
        raise ProbeError(f"not a 32-byte Solana address: {value!r}")
    return value


def check_base_address(value: str) -> str:
    if not _EVM.fullmatch(value):
        raise ProbeError(f"not a Base (EVM) address: {value!r}")
    if value.lower() == "0x" + "0" * 40:
        raise ProbeError("refusing the zero address")
    return value


def parse_amounts(text: str) -> List[Decimal]:
    amounts = []
    for part in text.split(","):
        part = part.strip()
        if not re.fullmatch(r"[0-9]+(\.[0-9]{1,6})?", part) or Decimal(part) <= 0:
            raise ProbeError(f"amount must be a positive USDC value with at most 6 decimals: {part!r}")
        amounts.append(Decimal(part))
    if not amounts or len(amounts) > 6:
        raise ProbeError("give between 1 and 6 amounts")
    return amounts


def find_usdc(tokens: List[Dict[str, Any]], chain: str) -> Dict[str, Any]:
    """The one native-USDC entry for `chain` in 1Click's live list, or refuse."""
    want = USDC[chain]
    same = (lambda a: a == want) if chain == "sol" else (lambda a: a.lower() == want.lower())
    matches = [t for t in tokens if t.get("blockchain") == chain and isinstance(t.get("contractAddress"), str)
               and same(t["contractAddress"])]
    if len(matches) != 1:
        raise ProbeError(f"1Click lists {len(matches)} tokens for {CHAIN_NAMES[chain]} USDC {want}; expected exactly one")
    token = matches[0]
    if token.get("symbol") != "USDC" or token.get("decimals") != USDC_DECIMALS or not str(token.get("assetId", "")).startswith("nep141:"):
        raise ProbeError(f"{CHAIN_NAMES[chain]} USDC entry is not 6-decimal USDC: {token!r}")
    return token


def _human(units: Any) -> Optional[str]:
    try:
        return str((Decimal(int(units)) / Decimal(10 ** USDC_DECIMALS)).quantize(Decimal("0.000001")))
    except (TypeError, ValueError):
        return None


def build_request(origin: Dict[str, Any], destination: Dict[str, Any], amount_units: int, *, refund_to: str,
                  recipient: str, slippage_bps: int, now: datetime, deadline_minutes: int = 60) -> Dict[str, Any]:
    return {
        "dry": True,
        "swapType": "EXACT_INPUT",
        "slippageTolerance": slippage_bps,
        "originAsset": origin["assetId"],
        "depositType": "ORIGIN_CHAIN",
        "destinationAsset": destination["assetId"],
        "amount": str(amount_units),
        "refundTo": refund_to,
        "refundType": "ORIGIN_CHAIN",
        "recipient": recipient,
        "recipientType": "DESTINATION_CHAIN",
        "deadline": ni.format_timestamp(now + timedelta(minutes=deadline_minutes)),
    }


def probe_one(client, request: Dict[str, Any], *, slippage_bps: int, fee_cap_bps: int,
              now_fn: Callable[[], datetime]) -> Dict[str, Any]:
    amount_units = int(request["amount"])
    row: Dict[str, Any] = {"amount_usdc": _human(amount_units), "request": request}
    try:
        response = client.quote(request)
    except ni.NearIntentsError as exc:
        detail = exc.detail if isinstance(exc.detail, dict) else {}
        row.update(verdict="NO_QUOTE", http_status=detail.get("status"), provider_message=exc.message,
                   correlation_id=detail.get("correlationId"))
        return row
    row["response"] = response
    leg = {"amount": amount_units, "slippage_bps": slippage_bps, "fee_cap_bps": fee_cap_bps,
           "deadline": request["deadline"], "min_receive": None}
    try:
        checks = ni.verify_quote(leg, request, response, now=now_fn())
    except ni.NearIntentsError as exc:
        checks = exc.detail if isinstance(exc.detail, list) else []
        if not checks:
            row.update(verdict="MALFORMED", provider_message=exc.message)
            return row
    row["checks"] = checks
    by_guard = {c["guard"]: c for c in checks}
    quote = response.get("quote") if isinstance(response.get("quote"), dict) else {}
    row.update(
        signature_ok=by_guard.get("quote_signature", {}).get("ok", False),
        amount_out_usdc=_human(quote.get("amountOut")),
        min_out_usdc=_human(quote.get("minAmountOut")),
        refund_fee_usdc=_human(quote.get("refundFee")),
        refund_fee_bps=by_guard.get("refund_fee", {}).get("refund_fee_bps"),
        cost_bps=by_guard.get("fees", {}).get("effective_cost_bps"),
        spread_bps=by_guard.get("slippage", {}).get("quoted_spread_bps"),
        time_estimate_s=quote.get("timeEstimate"),
    )
    failed = [c["guard"] for c in checks if not c["ok"]]
    row["verdict"] = "USABLE" if not failed else "REFUSED: " + ", ".join(failed)
    return row


def run(args: argparse.Namespace, client, now_fn: Callable[[], datetime] = lambda: datetime.now(timezone.utc),
        sleep: Callable[[float], None] = time.sleep) -> Dict[str, Any]:
    near = check_near_account(args.near_account)
    addresses = {"near": near, "sol": check_solana_address(args.solana_address)}
    if args.base_address:
        addresses["base"] = check_base_address(args.base_address)
    amounts = parse_amounts(args.amounts)
    if not 1 <= args.slippage_bps <= 500 or not 1 <= args.fee_cap_bps <= 500:
        raise ProbeError("slippage and fee cap must be between 1 and 500 bps")
    if not 10 <= args.deadline_minutes <= 1440:
        raise ProbeError("the intent deadline must be 10 to 1440 minutes out")
    wanted = list(ROUTES) if args.routes == "all" else [r.strip() for r in args.routes.split(",")]
    for name in wanted:
        if name not in ROUTES:
            raise ProbeError(f"unknown route {name!r}; choose from {', '.join(ROUTES)}")

    tokens = client.tokens()
    report: Dict[str, Any] = {
        "schema": "x3.oneclick.route-probe.v2",
        "mode": "dry_quotes_only_nothing_signed_nothing_deposited",
        "started_utc": ni.format_timestamp(now_fn()),
        "api": client.base_url,
        "authenticated": bool(getattr(client, "api_key", None) or client.authenticated),
        "slippage_bps": args.slippage_bps, "fee_cap_bps": args.fee_cap_bps,
        "routes": [],
    }
    first = True
    for name in wanted:
        origin_chain, dest_chain = ROUTES[name]
        route: Dict[str, Any] = {"route": name, "from": CHAIN_NAMES[origin_chain], "to": CHAIN_NAMES[dest_chain]}
        report["routes"].append(route)
        if origin_chain not in addresses or dest_chain not in addresses:
            route["skipped"] = "no --base-address given"
            continue
        try:
            origin, destination = find_usdc(tokens, origin_chain), find_usdc(tokens, dest_chain)
        except ProbeError as exc:
            route["skipped"] = str(exc)
            continue
        route.update(origin_asset=origin["assetId"], destination_asset=destination["assetId"],
                     refund_to=addresses[origin_chain], recipient=addresses[dest_chain], quotes=[])
        for amount in amounts:
            if not first:
                sleep(args.delay)
            first = False
            request = build_request(origin, destination, int(amount * 10 ** USDC_DECIMALS),
                                    refund_to=addresses[origin_chain], recipient=addresses[dest_chain],
                                    slippage_bps=args.slippage_bps, now=now_fn(),
                                    deadline_minutes=args.deadline_minutes)
            route["quotes"].append(probe_one(client, request, slippage_bps=args.slippage_bps,
                                             fee_cap_bps=args.fee_cap_bps, now_fn=now_fn))
    report["finished_utc"] = ni.format_timestamp(now_fn())
    usable = [(r["route"], q["amount_usdc"]) for r in report["routes"] for q in r.get("quotes", []) if q["verdict"] == "USABLE"]
    report["usable"] = [{"route": r, "amount_usdc": a} for r, a in usable]
    return report


def render(report: Dict[str, Any]) -> str:
    lines = [f"1Click dry quotes, {report['started_utc']}  (slippage {report['slippage_bps']} bps, fee cap "
             f"{report['fee_cap_bps']} bps, {'API key' if report['authenticated'] else 'no API key: unauthenticated fees'})", ""]
    head = f"  {'route':<10} {'in USDC':>8} {'out':>10} {'min out':>10} {'refund fee':>11} {'cost bps':>8}  verdict"
    lines.append(head)
    lines.append("  " + "-" * (len(head) - 2))
    for route in report["routes"]:
        if route.get("skipped"):
            lines.append(f"  {route['route']:<10} skipped: {route['skipped']}")
            continue
        for q in route["quotes"]:
            if q["verdict"] in ("NO_QUOTE", "MALFORMED"):
                status = f" HTTP {q['http_status']}" if q.get("http_status") else ""
                lines.append(f"  {route['route']:<10} {q['amount_usdc']:>8}  {q['verdict']}{status}: {q.get('provider_message')}")
                continue
            fee = f"{q['refund_fee_usdc']} ({q['refund_fee_bps']}bps)" if q.get("refund_fee_usdc") else "n/a"
            lines.append(f"  {route['route']:<10} {q['amount_usdc']:>8} {q['amount_out_usdc'] or '?':>10} "
                         f"{q['min_out_usdc'] or '?':>10} {fee:>11} {q.get('cost_bps') or '?':>8}  {q['verdict']}")
    lines.append("")
    if report["usable"]:
        lines.append("USABLE at this moment: " + ", ".join(f"{u['route']} @ {u['amount_usdc']} USDC" for u in report["usable"]))
    else:
        lines.append("No route passed every check at these amounts.")
    lines.append("Dry quotes only: nothing was signed or deposited. Quotes move; re-check right before trading.")
    return "\n".join(lines)


class _ApiKeyTransport:
    """Adds 1Click's X-API-Key header; the key never appears in output or the report."""

    def __init__(self, key: str, inner=ni.urllib_transport):
        self._key, self._inner = key, inner

    def __call__(self, method: str, url: str, headers: Dict[str, str], body: Optional[bytes], timeout: float) -> Tuple[int, bytes]:
        return self._inner(method, url, {**headers, "X-API-Key": self._key}, body, timeout)


def make_client():
    key = os.environ.get("NEAR_INTENTS_API_KEY")
    client = ni.OneClickClient(transport=_ApiKeyTransport(key)) if key else ni.OneClickClient()
    client.api_key = bool(key)
    return client


def _write_private(path: Path, data: Dict[str, Any]) -> None:
    path = path.expanduser()
    path.parent.mkdir(parents=True, exist_ok=True)
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(fd, "w", encoding="utf-8") as fh:
        fh.write(json.dumps(data, indent=2, sort_keys=True) + "\n")


def main(argv: Optional[List[str]] = None) -> int:
    p = argparse.ArgumentParser(description="Dry 1Click quotes for the candidate mainnet routes. Signs nothing.")
    p.add_argument("--near-account", required=True, help="your NEAR mainnet account (recipient / refund on NEAR)")
    p.add_argument("--solana-address", required=True, help="your Solana wallet public address")
    p.add_argument("--base-address", help="your Base wallet public address (enables the Base routes)")
    p.add_argument("--amounts", default=DEFAULT_AMOUNTS, help=f"USDC amounts, comma separated (default {DEFAULT_AMOUNTS})")
    p.add_argument("--routes", default="all", help=f"all, or a comma list of: {', '.join(ROUTES)}")
    p.add_argument("--slippage-bps", type=int, default=100)
    p.add_argument("--fee-cap-bps", type=int, default=100)
    p.add_argument("--deadline-minutes", type=int, default=60,
                   help="intent deadline asked for in each quote; refunds start after it (default 60)")
    p.add_argument("--delay", type=float, default=1.5, help="seconds between quote requests")
    p.add_argument("--out", type=Path, help="report path (default ~/.x3/route-probe-<UTC>.json)")
    p.add_argument("--json", action="store_true", help="print the report instead of the table")
    args = p.parse_args(argv)
    try:
        report = run(args, make_client())
    except (ProbeError, ni.NearIntentsError) as exc:
        print(f"route probe stopped: {exc}", file=sys.stderr)
        return 2
    out = args.out or Path(f"~/.x3/route-probe-{datetime.now(timezone.utc):%Y%m%dT%H%M%SZ}.json")
    _write_private(out, report)
    if args.json:
        print(json.dumps(report, indent=2, sort_keys=True))
    else:
        print(render(report))
    print(f"\nFull report (raw signed quotes): {out.expanduser()}")
    return 0 if report["usable"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
