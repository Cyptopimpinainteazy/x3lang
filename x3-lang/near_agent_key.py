#!/usr/bin/env python3
"""Pinned function-call key boundary for a trusted operator's NEAR CLI harness.

This is not OS credential isolation. Owner recovery keys may remain on the account.
Only the supplied, explicitly pinned signing key is admitted for agent calls.
"""
from __future__ import annotations

import json
import re

import ed25519
from near_intents import urllib_transport

METHODS = ("agent_register_intent", "agent_accept_quote", "agent_record_outcome")
RPC_URLS = {"mainnet": "https://free.rpc.fastnear.com", "testnet": "https://rpc.testnet.near.org"}
U128_MAX = (1 << 128) - 1


class AgentKeyError(ValueError):
    """Fixed, non-secret failure codes safe for command output."""


def _positive(value):
    if not isinstance(value, str) or not re.fullmatch(r"[1-9][0-9]{0,38}", value) or int(value) > U128_MAX:
        raise AgentKeyError("X3_AGENT_KEY_ALLOWANCE")
    return int(value)


def validate_pin(public_key, max_allowance_yocto):
    try:
        if not isinstance(public_key, str) or not public_key.startswith("ed25519:") or len(public_key) > 52:
            raise ValueError
        raw = ed25519.b58decode(public_key[8:])
        if len(raw) != 32 or ed25519.b58encode(raw) != public_key[8:]:
            raise ValueError
    except (ValueError, TypeError):
        raise AgentKeyError("X3_AGENT_KEY_PIN_REQUIRED") from None
    _positive(max_allowance_yocto)


def _account(value):
    if not isinstance(value, str) or not 2 <= len(value) <= 64 or not re.fullmatch(r"[a-z0-9]+(?:[-_.][a-z0-9]+)*", value):
        raise AgentKeyError("X3_AGENT_KEY_ACCOUNT")


def check_access_key(*, account, public_key, contract, method, max_allowance_yocto,
                     network="mainnet", transport=urllib_transport):
    """Fresh read-only check of one key. Never enumerate, remove or require removing other keys."""
    validate_pin(public_key, max_allowance_yocto)
    _account(account)
    _account(contract)
    if method not in METHODS:
        raise AgentKeyError("X3_AGENT_KEY_METHOD")
    if network not in RPC_URLS:
        raise AgentKeyError("X3_AGENT_KEY_NETWORK")

    def query(method_name, params):
        payload = {"jsonrpc": "2.0", "id": "x3-agent-key", "method": method_name, "params": params}
        try:
            status, raw = transport("POST", RPC_URLS[network], {"Content-Type": "application/json"},
                                    json.dumps(payload).encode(), 15.0)
            result = json.loads(raw)
            if status != 200 or not isinstance(result, dict) or result.get("error") is not None:
                raise ValueError
            result = result["result"]
            if not isinstance(result, dict) or result.get("error") is not None:
                raise ValueError
            return result
        except Exception:
            raise AgentKeyError("X3_AGENT_KEY_RPC") from None

    if query("status", []).get("chain_id") != network:
        raise AgentKeyError("X3_AGENT_KEY_NETWORK")
    result = query("query", {"request_type": "view_access_key", "finality": "final",
                             "account_id": account, "public_key": public_key})
    permission = result.get("permission")
    if not isinstance(permission, dict) or set(permission) != {"FunctionCall"}:
        raise AgentKeyError("X3_AGENT_KEY_PERMISSION")
    call = permission["FunctionCall"]
    if not isinstance(call, dict) or call.get("receiver_id") != contract:
        raise AgentKeyError("X3_AGENT_KEY_RECEIVER")
    methods = call.get("method_names")
    if (not isinstance(methods, list) or not methods or any(m not in METHODS for m in methods)
            or len(set(methods)) != len(methods) or method not in methods):
        raise AgentKeyError("X3_AGENT_KEY_METHOD")
    allowance = _positive(call.get("allowance"))
    if allowance > _positive(max_allowance_yocto):
        raise AgentKeyError("X3_AGENT_KEY_ALLOWANCE")
    if type(result.get("block_height")) is not int or not 0 < result["block_height"] <= (1 << 64) - 1 or not result.get("block_hash"):
        raise AgentKeyError("X3_AGENT_KEY_RPC")
    if type(result.get("nonce")) is not int or not 0 <= result["nonce"] < (1 << 64) - 1:
        raise AgentKeyError("X3_AGENT_KEY_RPC")
    try:
        if not isinstance(result["block_hash"], str) or not 32 <= len(result["block_hash"]) <= 44:
            raise ValueError
        raw_hash = ed25519.b58decode(result["block_hash"])
        if len(raw_hash) != 32 or ed25519.b58encode(raw_hash) != result["block_hash"]:
            raise ValueError
    except (ValueError, TypeError):
        raise AgentKeyError("X3_AGENT_KEY_RPC") from None
    return {"nonce": result["nonce"], "account": account, "public_key": public_key, "permission": "FunctionCall",
            "receiver_id": contract, "method": method, "allowance": str(allowance),
            "block_height": result["block_height"], "block_hash": result["block_hash"],
            "account_other_keys": "not_checked; owner recovery keys may remain", "broadcasted": False}


def _credential_snapshot(key_file, account, public_key):
    import os
    import stat
    from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
    try:
        fd = os.open(key_file, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
        with os.fdopen(fd, "rb") as handle:
            info = os.fstat(handle.fileno())
            if not stat.S_ISREG(info.st_mode) or info.st_uid != os.getuid() or info.st_mode & 0o077:
                raise ValueError
            raw = handle.read(16385)
        if len(raw) > 16384:
            raise ValueError
        value = json.loads(raw)
        if value.get("account_id") != account or value.get("public_key") != public_key:
            raise ValueError
        secret = value.get("private_key")
        if not isinstance(secret, str) or not secret.startswith("ed25519:") or len(secret) > 96:
            raise ValueError
        decoded = ed25519.b58decode(secret[8:])
        if len(decoded) != 64 or ed25519.b58encode(decoded) != secret[8:]:
            raise ValueError
        derived = Ed25519PrivateKey.from_private_bytes(decoded[:32]).public_key().public_bytes_raw()
        if decoded[32:] != derived or "ed25519:" + ed25519.b58encode(derived) != public_key:
            raise ValueError
        return {"account_id": account, "public_key": public_key, "private_key": secret}
    except Exception:
        raise AgentKeyError("X3_AGENT_KEY_CREDENTIAL") from None


def execute(*, account, public_key, contract, method, max_allowance_yocto, key_file, args,
            network="mainnet", attached_deposit="0 NEAR", transport=urllib_transport, runner=None, clock=None):
    """Validate once immediately before a single CLI send, using only that credential snapshot."""
    import os
    import subprocess
    import tempfile
    import time
    clock = clock or time.monotonic
    if attached_deposit != "0 NEAR" or not isinstance(args, dict):
        raise AgentKeyError("X3_AGENT_KEY_CALL")
    validate_pin(public_key, max_allowance_yocto)
    credential = _credential_snapshot(key_file, account, public_key)
    started = clock()
    checked = check_access_key(account=account, public_key=public_key, contract=contract, method=method,
                     max_allowance_yocto=max_allowance_yocto, network=network, transport=transport)
    # A private, short-lived copy binds the validated bytes to the CLI input. This is
    # protection against replacing the original path, not against a compromised OS/user.
    with tempfile.TemporaryDirectory(prefix="x3-agent-key-") as directory:
        snapshot = os.path.join(directory, "access-key.json")
        with os.fdopen(os.open(snapshot, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600), "w") as handle:
            json.dump(credential, handle)
        command = ["near", "contract", "call-function", "as-transaction", contract, method,
                   "json-args", json.dumps(args, separators=(",", ":"), sort_keys=True),
                   "prepaid-gas", "30 Tgas", "attached-deposit", "0 NEAR", "sign-as", account,
                   "network-config", network, "sign-with-access-key-file", snapshot,
                   "--nonce", str(checked["nonce"] + 1), "--block-hash", checked["block_hash"],
                   "--block-height", str(checked["block_height"]), "send"]
        if not 0 <= clock() - started <= 60:
            raise AgentKeyError("X3_AGENT_KEY_STALE")
        try:
            result = (runner or subprocess.run)(command, capture_output=True, text=True, timeout=180, check=False)
        except Exception:
            raise AgentKeyError("X3_AGENT_KEY_CLI_FAILED_OUTCOME_UNKNOWN") from None
        # CLI output can contain the full private quote/receipt and arguments. Emit
        # only a fixed metadata schema; candidates are never settlement/finality proof.
        output = (result.stdout or "") + "\n" + (result.stderr or "")
        candidates = set()
        for candidate in re.findall(r"(?m)^\s*Transaction ID: ([1-9A-HJ-NP-Za-km-z]{32,44})\s*$", output):
            decoded = ed25519.b58decode(candidate)
            if len(decoded) == 32 and ed25519.b58encode(decoded) == candidate:
                candidates.add(candidate)
        known_codes = ("X3_MANDATE_DENIED", "X3_STORAGE_BUDGET_EXHAUSTED", "X3_STORAGE_WITHDRAWAL_PENDING")
        codes = [code for code in known_codes if re.search(r"(?<![A-Z0-9_])" + code + r"(?![A-Z0-9_])", output)]
        print(json.dumps({"cli_exit_code": result.returncode,
                          "transaction_hash_candidate": next(iter(candidates)) if len(candidates) == 1 else None,
                          "error_code_candidates": codes, "verified": False, "broadcasted": "unknown"}, sort_keys=True))
        return result.returncode


def main(argv=None):
    import argparse
    import sys
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("check", "send"))
    parser.add_argument("--account", required=True)
    parser.add_argument("--public-key", required=True)
    parser.add_argument("--contract", required=True)
    parser.add_argument("--method", required=True, choices=METHODS)
    parser.add_argument("--max-allowance-yocto", required=True)
    parser.add_argument("--network", choices=tuple(RPC_URLS), default="mainnet")
    parser.add_argument("--key-file")
    parser.add_argument("--args-json")
    options = parser.parse_args(argv)
    common = dict(account=options.account, public_key=options.public_key, contract=options.contract,
                  method=options.method, max_allowance_yocto=options.max_allowance_yocto, network=options.network)
    try:
        if options.action == "check":
            if options.key_file or options.args_json:
                raise AgentKeyError("X3_AGENT_KEY_CHECK_IS_READ_ONLY")
            print(json.dumps(check_access_key(**common), sort_keys=True))
            return 0
        if not options.key_file or not options.args_json:
            raise AgentKeyError("X3_AGENT_KEY_CALL")
        try:
            args = json.loads(options.args_json)
        except ValueError:
            raise AgentKeyError("X3_AGENT_KEY_CALL") from None
        return execute(**common, key_file=options.key_file, args=args)
    except AgentKeyError as exc:
        print(json.dumps({"error": str(exc), "broadcasted": "unknown" if "OUTCOME_UNKNOWN" in str(exc) else False}),
              file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
