#!/usr/bin/env python3
"""Offline .x3 -> native-NEAR vault constructor configuration.

Reuses near_mandate's parser with an explicit native-vault profile. This module
never contacts a node, signs, deploys, or authorizes funds. A self-contained hash
is not an owner signature: verification requires independently trusted source
bytes, deployment context, code identity, policy version and current time.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import re
import sys
import tempfile
import time
from typing import Any

PROFILE = "near_native_vault_v1"
SCHEMA = "x3.native-vault.manifest.v1"
MAX_SOURCE_BYTES = 65_536
MAX_MANIFEST_BYTES = 65_536
MAX_MANIFEST_AGE_MS = 300_000
MAX_TIME_MS = 253_402_300_799_999
COMPILER_INPUTS = ("near_native_vault.py", "near_mandate.py", "near_intents.py", "cli.py", "registry.py")


class NativeVaultPolicyError(ValueError):
    """The policy cannot be represented exactly or the manifest does not match."""


def _require(condition: bool, message: str) -> None:
    if not condition:
        raise NativeVaultPolicyError(message)


def _time(value: Any) -> int:
    _require(type(value) is int and 0 <= value <= MAX_TIME_MS, "invalid integer millisecond timestamp")
    return value


def _canonical(value: Any) -> bytes:
    try:
        return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=True, allow_nan=False).encode("ascii")
    except (TypeError, ValueError, RecursionError) as exc:
        raise NativeVaultPolicyError("manifest is not canonical JSON data") from exc


def manifest_digest(manifest: dict[str, Any]) -> str:
    _require(type(manifest) is dict, "manifest must be an object")
    return hashlib.sha256(_canonical({k: v for k, v in manifest.items() if k != "manifest_sha256"})).hexdigest()


def _compiler_sources() -> dict[str, str]:
    root = Path(__file__).resolve().parent
    return {name: hashlib.sha256((root / name).read_bytes()).hexdigest() for name in COMPILER_INPUTS}


def compile_policy(source: bytes, *, network: str, vault_account: str, wasm_sha256: str,
                   policy_version: int, issued_at_ms: int) -> dict[str, Any]:
    """Compile one bounded source snapshot. All amounts are integer yoctonear.

    The five source-file digests identify the relevant compiler sources, not a
    reproducible binary toolchain or a complete transitive dependency attestation.
    Network/code/version are offline deployment context, not on-chain storage.
    """
    _require(type(source) is bytes and 0 < len(source) <= MAX_SOURCE_BYTES, "empty or oversized policy source")
    _time(issued_at_ms)
    _require(type(network) is str and network in ("sandbox", "testnet"), "only sandbox/testnet context is supported")
    _require(type(policy_version) is int and 0 < policy_version < (1 << 64), "invalid positive u64 policy version")
    _require(type(wasm_sha256) is str and re.fullmatch(r"[0-9a-f]{64}", wasm_sha256) is not None,
             "contract WASM identity must be lowercase SHA-256")
    try:
        source.decode("utf-8")
    except UnicodeError as exc:
        raise NativeVaultPolicyError("policy must be UTF-8") from exc

    import near_intents as ni
    mandate = ni.mandates()
    try:
        # Parse exactly the hashed bytes once through the existing parser. A
        # private temporary snapshot avoids a second read of a mutable input.
        with tempfile.TemporaryDirectory(prefix="x3-native-policy-") as directory:
            snapshot = Path(directory) / "policy.x3"
            snapshot.write_bytes(source)
            parsed = mandate.parse_mandate(str(snapshot), profile=PROFILE)
        _require(parsed["spend"] == "Near.NEAR", "only native Near.NEAR is enforced; no token or swap adapter")
        _require(len(parsed["allow_recipients"]) == 1, "native vault enforces exactly one recipient")
        recipient = parsed["allow_recipients"][0]
        principals = {"owner": parsed["owner"], "agent": parsed["agent"],
                      "recipient": recipient, "vault": vault_account}
        for role, account in principals.items():
            _require(type(account) is str and account.endswith(".testnet")
                     and ni.registry.is_valid_near_account(account), f"{role} must be a valid .testnet account")
        _require(len(set(principals.values())) == 4, "owner, agent, recipient and vault must be distinct")
        at = parsed["line_of"]
        cap = mandate._mandate_amount(parsed["per_payment"], "per_payment", at.get("per_payment"))
        budget = mandate._mandate_amount(parsed["lifetime_budget"], "lifetime_budget", at.get("lifetime_budget"))
        _require(0 < cap <= budget, "positive per-payment cap must not exceed lifetime budget")
        duration = mandate.parse_expiry(parsed["expires"], at.get("expires"))
        duration_ms = (duration.days * 86_400 + duration.seconds) * 1000 + duration.microseconds // 1000
        deadline_ms = _time(issued_at_ms + duration_ms)
        _require(deadline_ms > issued_at_ms, "policy expiry must be after compilation")
    except ni.NearIntentsError as exc:
        raise NativeVaultPolicyError(f"{exc.code}: {exc.message}") from exc

    manifest = {
        "schema": SCHEMA,
        "profile": PROFILE,
        "name": parsed["name"],
        "source_sha256": hashlib.sha256(source).hexdigest(),
        "compiler_sources_sha256": _compiler_sources(),
        "network": network,
        "vault_account": vault_account,
        "contract_wasm_sha256": wasm_sha256,
        "policy_version": policy_version,
        "compiled_at_ms": issued_at_ms,
        "asset": "native:NEAR",
        "units": "yoctonear",
        "budget_semantics": "lifetime_spent_plus_reserved",
        "max_payment_records": 64,
        "constructor": {"owner": parsed["owner"], "agent": parsed["agent"], "recipient": recipient,
                        "cap": str(cap), "budget": str(budget), "deadline_ms": deadline_ms},
        "owner_authorized": False,
        "deployment_verified": False,
    }
    manifest["manifest_sha256"] = manifest_digest(manifest)
    return manifest


def verify_manifest(manifest: dict[str, Any], source: bytes, *, network: str, vault_account: str,
                    wasm_sha256: str, policy_version: int, now_ms: int) -> dict[str, Any]:
    """Recompile from trusted inputs; never trust a self-recomputed manifest hash.

    This checks offline source/configuration consistency only. Policy-version and
    code-context matching do not prove that a deployed contract stores them.
    """
    _require(type(manifest) is dict, "manifest must be an object")
    now_ms = _time(now_ms)
    issued_at = _time(manifest.get("compiled_at_ms"))
    _require(0 <= now_ms - issued_at <= MAX_MANIFEST_AGE_MS, "future or stale manifest; recompile before deployment review")
    expected = compile_policy(source, network=network, vault_account=vault_account, wasm_sha256=wasm_sha256,
                              policy_version=policy_version, issued_at_ms=issued_at)
    _require(_canonical(manifest) == _canonical(expected), "manifest does not exactly match trusted source and context")
    _require(now_ms < expected["constructor"]["deadline_ms"], "policy expired")
    return dict(expected["constructor"])


def _unique_object(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    out = {}
    for key, value in pairs:
        _require(key not in out, "duplicate manifest JSON key")
        out[key] = value
    return out


def _reject_number(value: str) -> None:
    raise NativeVaultPolicyError("manifest does not permit floating-point or non-finite numbers")


def verify_manifest_bytes(raw: bytes, source: bytes, **context: Any) -> dict[str, Any]:
    _require(type(raw) is bytes and 0 < len(raw) <= MAX_MANIFEST_BYTES, "empty or oversized manifest")
    try:
        value = json.loads(raw.decode("utf-8"), object_pairs_hook=_unique_object,
                           parse_float=_reject_number, parse_constant=_reject_number)
    except (ValueError, UnicodeError, RecursionError) as exc:
        raise NativeVaultPolicyError("malformed or ambiguous manifest JSON") from exc
    return verify_manifest(value, source, **context)


def _read(path: Path, maximum: int) -> bytes:
    with path.open("rb") as handle:
        raw = handle.read(maximum + 1)
    _require(len(raw) <= maximum, "input exceeds size limit")
    return raw


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    for name in ("compile", "verify"):
        command = commands.add_parser(name)
        command.add_argument("source", type=Path)
        command.add_argument("--network", choices=("sandbox", "testnet"), required=True)
        command.add_argument("--vault-account", required=True)
        command.add_argument("--wasm-sha256", required=True)
        command.add_argument("--policy-version", type=int, required=True)
        command.add_argument("--at-ms", type=int, help="explicit compile/replay time; defaults to current time")
        if name == "verify":
            command.add_argument("--manifest", type=Path, required=True)
    args = parser.parse_args(argv)
    try:
        source = _read(args.source, MAX_SOURCE_BYTES)
        context = {"network": args.network, "vault_account": args.vault_account,
                   "wasm_sha256": args.wasm_sha256, "policy_version": args.policy_version}
        now = time.time_ns() // 1_000_000 if args.at_ms is None else args.at_ms
        if args.command == "compile":
            result = compile_policy(source, issued_at_ms=now, **context)
        else:
            constructor = verify_manifest_bytes(_read(args.manifest, MAX_MANIFEST_BYTES), source, now_ms=now, **context)
            result = {"offline_configuration_verified": True, "constructor": constructor,
                      "owner_authorized": False, "deployment_verified": False}
        output = _canonical(result).decode("ascii")
    except (OSError, NativeVaultPolicyError) as exc:
        print(f"X3_NATIVE_VAULT_REFUSED: {exc}", file=sys.stderr)
        return 2
    print(output)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
