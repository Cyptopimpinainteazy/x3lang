#!/usr/bin/env python3
"""NEAR Testnet Guard preflight: read-only public RPC, no signing or deploying.

Checks minimum account balance required by the WASM *code bytes alone*. A
positive balance check is never a complete authorization to deploy, since
contract storage, gas, code audit, owner approval and signing are distinct.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import re
import subprocess
import sys
from datetime import datetime, timezone
from decimal import Decimal
from pathlib import Path
from urllib.error import URLError
from urllib.request import Request, urlopen

PROVIDERS = ("https://rpc.testnet.near.org", "https://test.rpc.fastnear.com")
EXPECTED_GENESIS = "FWJ9kR6KFWoyMoNjpLXXGHeuiy7tEY6GmoFeCA5yuc6b"
EXPECTED_EMPTY_CODE_HASH = "11111111111111111111111111111111"
ROOT = Path(__file__).resolve().parents[2]


class PreflightFailure(RuntimeError):
    pass


def minimum_wasm_stake(artifact_bytes: int, yocto_per_byte: int) -> int:
    if artifact_bytes <= 0 or artifact_bytes > 4 * 1024 * 1024:
        raise PreflightFailure("invalid code length (0 < size <= 4 MiB required)")
    if yocto_per_byte <= 0:
        raise PreflightFailure("invalid protocol storage price")
    return artifact_bytes * yocto_per_byte


def _rpc(provider: str, method: str, params: object) -> dict:
    if provider not in PROVIDERS:
        raise PreflightFailure("provider is not on the explicit testnet allowlist")
    request = {"jsonrpc": "2.0", "id": "x3-guard-readonly-preflight", "method": method, "params": params}
    body = json.dumps(request, separators=(",", ":"), sort_keys=True).encode()
    http_request = Request(provider, data=body, headers={
        "Content-Type": "application/json", "User-Agent": "X3GuardReadOnlyDeploymentPreflight/1.0",
    })
    try:
        with urlopen(http_request, timeout=15) as response:
            raw = response.read(2_000_001)
    except (URLError, OSError, TimeoutError) as exc:
        raise PreflightFailure(f"read-only public RPC unavailable: {provider} {method}") from exc
    if len(raw) > 2_000_000:
        raise PreflightFailure("oversized public RPC response")
    try:
        data = json.loads(raw)
    except ValueError as exc:
        raise PreflightFailure("malformed JSON-RPC response") from exc
    if not isinstance(data, dict) or data.get("error") is not None or not isinstance(data.get("result"), dict):
        raise PreflightFailure(f"public RPC declined {method}: {provider}")
    return data["result"]


def observe(provider: str, account: str) -> dict:
    status = _rpc(provider, "status", [])
    if status.get("chain_id") != "testnet" or status.get("genesis_hash") != EXPECTED_GENESIS:
        raise PreflightFailure("RPC network/genesis is not expected NEAR Testnet")
    head_height = status.get("sync_info", {}).get("latest_block_height")
    if not isinstance(head_height, int) or head_height < 1:
        raise PreflightFailure("RPC has no usable reported block height")
    config = _rpc(provider, "EXPERIMENTAL_protocol_config", {"block_id": head_height})
    if config.get("chain_id") != "testnet":
        raise PreflightFailure("protocol config is not Testnet")
    try:
        price = int(config["runtime_config"]["storage_amount_per_byte"])
        version = int(config["protocol_version"])
    except (KeyError, TypeError, ValueError) as exc:
        raise PreflightFailure("missing or invalid storage staking configuration") from exc
    if price <= 0:
        raise PreflightFailure("invalid storage staking price")
    view = _rpc(provider, "query", {"request_type": "view_account", "account_id": account, "finality": "final"})
    try:
        amount = int(view["amount"])
        stored = int(view["storage_usage"])
        code_hash = view["code_hash"]
    except (KeyError, TypeError, ValueError) as exc:
        raise PreflightFailure("incomplete account view") from exc
    if amount < 0 or stored < 0 or not isinstance(code_hash, str):
        raise PreflightFailure("invalid account balance or storage view")
    return {"provider": provider, "head_height_reported": head_height, "protocol_version": version,
            "storage_yocto_per_byte": str(price), "account_balance_yocto": str(amount),
            "existing_account_storage_bytes": stored, "code_hash": code_hash}


def run(account: str, wasm: Path) -> tuple[dict, bool]:
    if not re.fullmatch(r"[a-z0-9][a-z0-9._-]{1,50}\.testnet", account):
        raise PreflightFailure("account must be explicit and end in .testnet; no mainnet account allowed")
    if not wasm.is_file():
        raise PreflightFailure("local Guard .wasm is missing; build the release artifact first")
    raw = wasm.read_bytes()
    if not raw.startswith(b"\x00asm"):
        raise PreflightFailure("artifact is not a WebAssembly binary")
    size = len(raw)
    if not 0 < size <= 4 * 1024 * 1024:
        raise PreflightFailure("WASM artifact outside accepted size")
    observations = [observe(provider, account) for provider in PROVIDERS]
    agreed = [(x["storage_yocto_per_byte"], x["account_balance_yocto"],
               x["existing_account_storage_bytes"], x["code_hash"]) for x in observations]
    if agreed[0] != agreed[1]:
        raise PreflightFailure("independent public RPC observations disagree; no deployment readiness")
    price, amount, stored, code_hash = agreed[0]
    if code_hash != EXPECTED_EMPTY_CODE_HASH:
        raise PreflightFailure("account already contains code; this first-deployment check does not approve upgrades or migrations")
    minimum = minimum_wasm_stake(size, int(price))
    shortfall = max(0, minimum - int(amount))
    commit = subprocess.run(["git", "rev-parse", "HEAD"], cwd=ROOT, capture_output=True,
                            text=True, check=True, timeout=8).stdout.strip()
    insufficient = shortfall > 0
    report = {
        "schema": "x3.guard.testnet.readonly-deployment-preflight.v1",
        "observed_at_utc": datetime.now(timezone.utc).isoformat(timespec="seconds"),
        "source_commit": commit,
        "network": "testnet", "genesis_hash_rpc_reported": EXPECTED_GENESIS,
        "account": account,
        "wasm_sha256": hashlib.sha256(raw).hexdigest(), "wasm_bytes": size,
        "reported_current_code_hash": code_hash,
        "existing_code_absent": code_hash == EXPECTED_EMPTY_CODE_HASH,
        "balance_yocto": amount,
        "storage_price_yocto_per_byte": price,
        "minimum_code_stake_yocto": str(minimum),
        "minimum_shortfall_yocto": str(shortfall),
        "existing_account_storage_bytes": stored,
        "rpc_observations": observations,
        "account_has_enough_for_code_bytes_alone": not insufficient,
        "status": "BLOCKED_INSUFFICIENT_STORAGE_STAKE" if insufficient else "UNAPPROVED_BALANCE_ONLY_PREFLIGHT",
        "independently_authenticated_chain": False,
        "keys_loaded": False, "deploy_authorized": False, "contract_deployed": False,
        "signed_quote_testnet_refusal_proven": False,
        "funds_moved": False,
        "limitations": [
            "Both providers return public RPC assertions, not independent consensus proofs.",
            "WASM-code storage cost is a lower bound; existing state/storage, gas and init can require more.",
            "Account ownership and signing-key availability are intentionally not inspected.",
            "Deployment and attached funds require separate explicit owner authorization.",
            "Official 1Click manager signatures are not equivalent to third-party testnet signer keys.",
        ],
    }
    return report, insufficient


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--account", required=True, help="explicit account ID, ending .testnet")
    parser.add_argument("--wasm", type=Path, required=True, help="locally built WASM file (read only)")
    parser.add_argument("--report", type=Path, required=True, help="output JSON path (private 0600)")
    args = parser.parse_args()
    try:
        report, insufficient = run(args.account, args.wasm)
        args.report.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
        import os
        fd = os.open(args.report, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(fd, "w", encoding="utf8") as file:
            json.dump(report, file, indent=2, sort_keys=True)
            file.write("\n")
        print("ACCOUNT", args.account, "WASM_BYTES", report["wasm_bytes"])
        print("ACCOUNT_BALANCE_NEAR", Decimal(report["balance_yocto"]) / Decimal(10**24))
        print("MINIMUM_CODE_STORAGE_NEAR", Decimal(report["minimum_code_stake_yocto"]) / Decimal(10**24))
        print("MINIMUM_SHORTFALL_NEAR", Decimal(report["minimum_shortfall_yocto"]) / Decimal(10**24))
        print("STATUS", report["status"])
        print("AUTHORIZATION false | DEPLOYMENT false | KEYS false | FUNDS_MOVED false")
        print("REPORT", args.report)
        return 3 if insufficient else 0
    except (PreflightFailure, OSError, ValueError, subprocess.SubprocessError) as exc:
        print("PREFLIGHT REFUSED", str(exc), file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
