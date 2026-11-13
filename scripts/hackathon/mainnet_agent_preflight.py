#!/usr/bin/env python3
"""Read-only, dry-only mainnet preparation; never authorizes a wallet."""
from __future__ import annotations

import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import re
import sys
from datetime import datetime, timezone
import urllib.error
import urllib.request

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "x3-lang"))
import near_intents as ni  # noqa: E402

API = "https://1click.chaindefuser.com"
RPC = "https://free.rpc.fastnear.com"
NEAR_GENESIS = "EPnLgE7iEq9s7yTkos96M3cWymH5avBAPm3qx3NXqR8H"
SOL_MINT = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v"
NEAR_USDC = "17208628f84f5d6ad33f0da3bbbeb27ffcb398eac501a31bd6ad2011e36133a1"
SOL_ASSET = "nep141:sol-5ce3bf3a31af18be40ba30f721101b4341690186.omft.near"
PINS = (
    {"assetId": SOL_ASSET, "blockchain": "sol", "symbol": "USDC",
     "decimals": 6, "contractAddress": SOL_MINT},
    {"assetId": "nep141:" + NEAR_USDC, "blockchain": "near", "symbol": "USDC",
     "decimals": 6, "contractAddress": NEAR_USDC},
)
REQUIRED = ("program", "mandate", "max_input_usdc", "guard", "expected_guard_code_hash")


class PreflightError(ValueError):
    pass


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        raise PreflightError("HTTP_REDIRECT_REFUSED")


def readonly_transport(method, url, headers, body, timeout):
    """Allow only token discovery, dry quote, and mainnet account observations."""
    payload = json.loads(body) if body is not None else None
    allowed = method == "GET" and url == API + "/v0/tokens" and body is None
    if method == "POST" and url == API + "/v0/quote":
        allowed = isinstance(payload, dict) and payload.get("dry") is True
    if method == "POST" and url == RPC and isinstance(payload, dict):
        params = payload.get("params")
        allowed = (
            payload.get("method") == "status" and params == []
        ) or (
            payload.get("method") == "query" and isinstance(params, dict)
            and set(params) == {"request_type", "finality", "account_id"}
            and params.get("request_type") == "view_account"
            and params.get("finality") == "final"
        ) or (
            payload.get("method") == "query" and isinstance(params, dict)
            and set(params) == {"request_type", "finality", "account_id", "method_name", "args_base64"}
            and params["request_type"] == "call_function" and params["finality"] == "final"
            and params["account_id"] == NEAR_USDC and params["method_name"] == "storage_balance_of"
        )
    if not allowed:
        raise PreflightError("NETWORK_OPERATION_REFUSED")
    request = urllib.request.Request(url, data=body, headers=headers, method=method)
    opener = urllib.request.build_opener(NoRedirect())
    try:
        with opener.open(request, timeout=timeout) as response:
            return response.status, response.read(8 * 1024 * 1024)
    except urllib.error.HTTPError as exc:
        return exc.code, exc.read(4096)


def pinned_tokens(tokens):
    if not isinstance(tokens, list):
        raise PreflightError("TOKEN_REGISTRY_INVALID")
    selected = []
    for pin in PINS:
        matches = [t for t in tokens if isinstance(t, dict) and t.get("assetId") == pin["assetId"]]
        if len(matches) != 1:
            raise PreflightError("PINNED_ASSET_MISSING_OR_DUPLICATED")
        token = matches[0]
        if any(token.get(k) != v for k, v in pin.items()) or type(token.get("decimals")) is not int:
            raise PreflightError("PINNED_ASSET_METADATA_MISMATCH")
        selected.append(token)
    return selected


def budget_units(value):
    if not isinstance(value, str) or not re.fullmatch(r"[0-9]+(?:\.[0-9]{1,6})?", value):
        raise PreflightError("BUDGET_MUST_BE_POSITIVE_USDC_WITH_AT_MOST_SIX_DECIMALS")
    whole, _, fraction = value.partition(".")
    units = int(whole) * 1000000 + int((fraction + "000000")[:6])
    if units <= 0:
        raise PreflightError("BUDGET_MUST_BE_POSITIVE")
    return units


def rpc_read(method, params, transport):
    status, raw = transport("POST", RPC, {"Content-Type": "application/json"},
                            json.dumps({"jsonrpc": "2.0", "id": "x3-mainnet-preflight",
                                        "method": method, "params": params}).encode(), 15)
    data = json.loads(raw)
    if status != 200 or not isinstance(data, dict) or not isinstance(data.get("result"), dict):
        raise PreflightError("MAINNET_RPC_OBSERVATION_FAILED")
    return data["result"]


def recipient_storage(account, transport):
    """Read the native NEAR USDC contract's NEP-145 registration view."""
    args = base64.b64encode(json.dumps({"account_id": account}).encode()).decode()
    view = rpc_read("query", {"request_type": "call_function", "finality": "final",
                            "account_id": NEAR_USDC, "method_name": "storage_balance_of",
                            "args_base64": args}, transport)
    try:
        raw = view["result"]
        if not isinstance(raw, list) or any(type(n) is not int or not 0 <= n <= 255 for n in raw):
            raise ValueError("invalid byte array")
        balance = json.loads(bytes(raw))
        if balance is not None:
            total, available = balance["total"], balance["available"]
            if (not re.fullmatch(r"[0-9]{1,39}", total)
                    or not re.fullmatch(r"[0-9]{1,39}", available)
                    or not 0 <= int(available) <= int(total) < 2**128
                    or int(total) == 0):
                raise ValueError("invalid balance")
    except (KeyError, TypeError, ValueError) as exc:
        raise PreflightError("NEAR_USDC_STORAGE_RESPONSE_INVALID") from exc
    return {"account": account, "token_contract": NEAR_USDC, "registered": balance is not None,
            "block_hash": view.get("block_hash"), "block_height": view.get("block_height")}


def guard_observation(account, expected, transport):
    if not ni.registry.is_valid_receiver("near", account) or account.endswith(".testnet"):
        raise PreflightError("MAINNET_GUARD_ACCOUNT_INVALID")
    if expected is not None and (
        not re.fullmatch(r"[1-9A-HJ-NP-Za-km-z]{32,44}", expected)
        or expected == "1" * 32
    ):
        raise PreflightError("EXPECTED_CODE_HASH_MUST_BE_DEPLOYED_CODE_BASE58")
    observed = rpc_read("query", {"request_type": "view_account", "finality": "final",
                                  "account_id": account}, transport)
    code_hash = observed.get("code_hash")
    match = bool(expected and code_hash == expected and code_hash != "1" * 32)
    return {"account": account, "rpc": RPC, "finality": "final",
            "code_hash": code_hash, "expected_code_hash": expected, "matches_expected": match,
            "block_hash": observed.get("block_hash"), "block_height": observed.get("block_height"),
            "evidence_kind": "single_rpc_observation_not_authenticated_finality"}


def run(args, *, transport=readonly_transport, now=None):
    now = now or datetime.now(timezone.utc)
    missing = ["--" + key.replace("_", "-") for key in REQUIRED if getattr(args, key) is None]
    report = {
        "kind": "x3-mainnet-agent-preflight-v1", "observed_at": now.isoformat(),
        "mode": "dry_only", "eligible_for_wallet_authorization": False,
        "status": "blocked", "missing_inputs": missing, "checks": {},
        "pending": [
            "Owner confirms exact budget, source/refund wallet, recipient and transaction terms.",
            "Independent Guard review, mainnet deployment provenance and current on-chain mandate/approvals.",
            "Fresh live quote, owner signing review and separate wallet signature.",
            "Finalized source transfer and destination payout verified against the same quote; provider status alone is insufficient.",
        ],
        "limits": [
            "Guard does not custody the owner's Solana wallet; its keyholder can bypass Guard.",
            "Local mandate checks do not establish current chain revocation, spend or approval state.",
            "Dry quote fields cannot authorize a deposit or prove settlement.",
            "This tool does not invoke a model; retain the actual model generation record separately.",
        ],
    }
    client = ni.OneClickClient(base_url=API, transport=transport)
    try:
        tokens = pinned_tokens(client.tokens())
        report["checks"]["assets"] = {"ok": True, "endpoint": API + "/v0/tokens",
                                      "pins": list(PINS)}
        # Token discovery is useful even before public owner parameters are supplied.
        if args.guard or (args.program and args.mandate and args.max_input_usdc):
            identity = rpc_read("status", [], transport)
            if identity.get("chain_id") != "mainnet" or identity.get("genesis_hash") != NEAR_GENESIS:
                raise PreflightError("NEAR_NETWORK_IS_NOT_MAINNET")
            report["checks"]["network"] = {"ok": True, "chain_id": "mainnet", "rpc": RPC}
        if args.guard:
            observation = guard_observation(args.guard, args.expected_guard_code_hash, transport)
            report["checks"]["guard"] = observation
            if args.expected_guard_code_hash and not observation["matches_expected"]:
                raise PreflightError("GUARD_CODE_HASH_MISMATCH")
        if not (args.program and args.mandate and args.max_input_usdc):
            return report
        cap = budget_units(args.max_input_usdc)
        _, leg = ni.compile_program(args.program, now=now)
        if ((leg["origin"]["chain"], leg["origin"]["asset"]) != ("solana", "USDC")
                or (leg["destination"]["chain"], leg["destination"]["asset"]) != ("near", "USDC")):
            raise PreflightError("ONLY_SOLANA_NATIVE_USDC_TO_NEAR_NATIVE_USDC")
        request = ni.build_quote_request(leg, tokens, dry=True)
        if int(request["amount"]) > cap:
            raise PreflightError("PROGRAM_EXCEEDS_OWNER_INPUT_CAP")
        program_hash = hashlib.sha256(Path(args.program).read_bytes()).hexdigest()
        mandate_hash = hashlib.sha256(Path(args.mandate).read_bytes()).hexdigest()
        local_mandate = ni.check_against_mandate(
            leg, request, program_hash, args.mandate, tokens=tokens, now=now)
        report["checks"]["local_program"] = {
            "ok": True, "program_sha256": program_hash, "mandate_sha256": mandate_hash,
            "input_cap_base_units": str(cap), "input_base_units": request["amount"],
            "recipient": request["recipient"], "refund_to": request["refundTo"],
            "mandate_checks": local_mandate["checks"],
            "approvals_needed": local_mandate["approvals_needed"],
        }
        account_checks = ni.preflight_accounts(leg, ni.NearRpc(url=RPC, transport=transport))
        storage = recipient_storage(request["recipient"], transport)
        report["checks"]["recipient_storage"] = storage
        if not storage["registered"]:
            raise PreflightError("NEAR_USDC_RECIPIENT_UNREGISTERED")
        response = client.quote(request)
        guards = ni.verify_quote(leg, request, response, now=now)
        report["checks"]["dry_quote"] = {
            "ok": True, "evidence_kind": "signed_dry_quote_policy_preview",
            "authenticated_api": client.authenticated, "request": request, "response": response,
            "guards": guards, "account_checks": account_checks,
        }
        report["dry_preview_verified"] = True
    except Exception as exc:
        # Do not copy HTTP bodies, environment or arbitrary exception strings into reports.
        report["error"] = {"code": getattr(exc, "code", str(exc) if isinstance(exc, PreflightError)
                                           else type(exc).__name__)}
        if getattr(exc, "code", "") == "X3_NEAR_PROVIDER_MINIMUM":
            minimum = (getattr(exc, "detail", None) or {}).get("providerMinimumUsd")
            if isinstance(minimum, str) and re.fullmatch(r"[0-9]+(?:\.[0-9]+)?", minimum):
                report["error"]["provider_minimum_usd"] = minimum
            report["pending"].append("Provider minimum blocks this budget; do not increase it without owner approval.")
    return report


def save_report(path, report):
    target = Path(path).expanduser()
    if target.resolve().is_relative_to(ROOT):
        raise PreflightError("REPORT_MUST_BE_OUTSIDE_REPOSITORY")
    flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, "O_NOFOLLOW", 0)
    fd = os.open(target, flags, 0o600)
    with os.fdopen(fd, "w", encoding="utf-8") as stream:
        json.dump(report, stream, indent=2)
        stream.write("\n")


def parser():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--program", help="Reviewed public-parameter .x3 payment; never a wallet key file.")
    p.add_argument("--mandate", help="Owner's .x3 mandate containing public parameters only.")
    p.add_argument("--max-input-usdc", help="Explicit owner input cap in decimal USDC, without gas.")
    p.add_argument("--guard", help="Mainnet NEAR Guard account.")
    p.add_argument("--expected-guard-code-hash", help="Reviewed deployment's NEAR base58 code_hash.")
    p.add_argument("--report", required=True, help="New private JSON file outside this repository.")
    return p


def main(argv=None):
    args = parser().parse_args(argv)
    report = run(args)
    try:
        save_report(args.report, report)
    except (OSError, PreflightError):
        print("Report not saved: use a new writable file outside the repository.", file=sys.stderr)
        return 2
    print(json.dumps({"status": report["status"], "eligible_for_wallet_authorization": False,
                      "missing_inputs": report["missing_inputs"], "error": report.get("error"),
                      "dry_preview_verified": report.get("dry_preview_verified", False),
                      "report": str(Path(args.report).expanduser())}, indent=2))
    return 2  # Preparation never opens the financial execution gate.


if __name__ == "__main__":
    raise SystemExit(main())
