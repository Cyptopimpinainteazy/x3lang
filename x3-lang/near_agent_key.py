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


def _gas(gas_tgas):
    if type(gas_tgas) is not int or not 1 <= gas_tgas <= 30:
        raise AgentKeyError("X3_AGENT_KEY_GAS")
    return gas_tgas * 10**12


def admission_cost(*, runtime_config, gas_price, gas_tgas, method, arguments, sender_is_receiver):
    """NEAR 2.14 single Ed25519 FunctionCall, zero-deposit upfront purchase.

    This is allowance reservation, NOT the final burnt fee. The receipt price
    uses min_gas_purchase_price, even when the current block price is lower.
    """
    prepaid = _gas(gas_tgas)
    try:
        def number(value):
            if type(value) is int and 0 <= value <= U128_MAX:
                return value
            if isinstance(value, str) and re.fullmatch(r"0|[1-9][0-9]{0,38}", value) and int(value) <= U128_MAX:
                return int(value)
            raise ValueError
        price = number(gas_price)
        floor = number(runtime_config["min_gas_purchase_price"])
        if price <= 0 or not isinstance(method, str) or not isinstance(arguments, str) or type(sender_is_receiver) is not bool:
            raise ValueError
        fees = runtime_config["transaction_costs"]
        receipt = fees["action_receipt_creation_config"]
        call = fees["action_creation_config"]["function_call_cost"]
        byte = fees["action_creation_config"]["function_call_cost_per_byte"]
        size = len(method.encode("utf-8")) + len(arguments.encode("utf-8"))
        send = "send_sir" if sender_is_receiver else "send_not_sir"
        send_gas = number(receipt[send]) + number(call[send]) + size * number(byte[send])
        execution = number(receipt["execution"]) + number(call["execution"]) + size * number(byte["execution"])
        receipt_price = max(price, floor)
        required = send_gas * price + (prepaid + execution) * receipt_price
        if required > U128_MAX or send_gas + prepaid + execution > (1 << 64) - 1:
            raise ValueError
        return {"required_allowance_yocto": str(required), "receipt_gas_price_yocto": str(receipt_price),
                "current_gas_price_yocto": str(price), "send_gas": send_gas,
                "receipt_execution_overhead_gas": execution, "prepaid_gas": prepaid,
                "actual_burnt_fee_yocto": None}
    except (KeyError, TypeError, ValueError, OverflowError):
        raise AgentKeyError("X3_AGENT_KEY_COST_CONFIG") from None


def check_admission_cost(*, checked, network, method, arguments, gas_tgas, transport=urllib_transport):
    """Read fees at the same finalized block as the checked access key; fail closed."""
    def query(method_name):
        try:
            payload = {"jsonrpc": "2.0", "id": "x3-agent-cost", "method": method_name,
                       "params": {"block_id": checked["block_hash"]}}
            status, raw = transport("POST", RPC_URLS[network], {"Content-Type": "application/json"},
                                    json.dumps(payload).encode(), 15.0)
            response = json.loads(raw)
            if status != 200 or response.get("error") is not None or not isinstance(response.get("result"), dict):
                raise ValueError
            return response["result"]
        except Exception:
            raise AgentKeyError("X3_AGENT_KEY_COST_RPC") from None
    config = query("EXPERIMENTAL_protocol_config")
    price = query("gas_price")
    try:
        cost = admission_cost(runtime_config=config["runtime_config"], gas_price=price["gas_price"],
                              gas_tgas=gas_tgas, method=method, arguments=arguments,
                              sender_is_receiver=checked["account"] == checked["receiver_id"])
    except KeyError:
        raise AgentKeyError("X3_AGENT_KEY_COST_CONFIG") from None
    if int(cost["required_allowance_yocto"]) > _positive(checked["allowance"]):
        raise AgentKeyError("X3_AGENT_KEY_INSUFFICIENT_ALLOWANCE")
    return cost


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


def _signed_artifact(output, *, account, public_key, contract, method, arguments, checked, gas_tgas=30):
    """Admit only the exact NEAR V0/single FunctionCall transaction, signed by the pin.

    This compares the installed CLI's Borsh output; it does not implement a signer.
    Unknown transaction/action/signature formats fail closed.
    """
    import base64
    import hashlib
    try:
        matches = re.findall(r"(?m)^Signed transaction \(serialized as base64\):\r?\n([A-Za-z0-9+/]+={0,2})\r?$", output)
        if len(matches) != 1 or len(matches[0]) > 1_400_000:
            raise ValueError
        encoded = matches[0]
        raw = base64.b64decode(encoded, validate=True)
        if base64.b64encode(raw).decode() != encoded:
            raise ValueError
        def blob(value):
            data = value.encode("utf-8")
            return len(data).to_bytes(4, "little") + data
        public = ed25519.b58decode(public_key[8:])
        expected = (blob(account) + b"\0" + public + (checked["nonce"] + 1).to_bytes(8, "little")
                    + blob(contract) + ed25519.b58decode(checked["block_hash"])
                    + (1).to_bytes(4, "little") + b"\x02" + blob(method) + blob(arguments)
                    + (_gas(gas_tgas)).to_bytes(8, "little") + bytes(16))
        if len(raw) != len(expected) + 65 or raw[:len(expected)] != expected or raw[len(expected)] != 0:
            raise ValueError
        digest = hashlib.sha256(expected).digest()
        if not ed25519.verify(public, digest, raw[-64:]):
            raise ValueError
        return encoded, ed25519.b58encode(digest)
    except Exception:
        raise AgentKeyError("X3_AGENT_KEY_SIGNED_TRANSACTION_MISMATCH") from None


def _local_cli_error(output):
    # Fixed categories only. Never include CLI text, argv, paths or payloads.
    for marker, code in (
        ("Access key file not found!", "X3_AGENT_KEY_CLI_CREDENTIAL"),
        ("Error reading data from file:", "X3_AGENT_KEY_CLI_CREDENTIAL"),
        ("unexpected argument", "X3_AGENT_KEY_CLI_ARGUMENT"),
        ("unrecognized subcommand", "X3_AGENT_KEY_CLI_ARGUMENT"),
        ("Cannot send transaction in offline mode", "X3_AGENT_KEY_CLI_OFFLINE_SEND"),
    ):
        if marker in output:
            return code
    return "X3_AGENT_KEY_CLI_SIGNING_FAILED"


def execute(*, account, public_key, contract, method, max_allowance_yocto, key_file, args,
            network="mainnet", attached_deposit="0 NEAR", gas_tgas=30, transport=urllib_transport, runner=None, clock=None):
    """Offline CLI signing, exact artifact admission, then one fixed-RPC send (no retry)."""
    import os
    import base64
    import subprocess
    import tempfile
    import time
    _gas(gas_tgas)
    clock = clock or time.monotonic
    if attached_deposit != "0 NEAR" or not isinstance(args, dict):
        raise AgentKeyError("X3_AGENT_KEY_CALL")
    validate_pin(public_key, max_allowance_yocto)
    credential = _credential_snapshot(key_file, account, public_key)
    started = clock()
    checked = check_access_key(account=account, public_key=public_key, contract=contract, method=method,
                     max_allowance_yocto=max_allowance_yocto, network=network, transport=transport)
    try:
        arguments = json.dumps(args, separators=(",", ":"), sort_keys=True, allow_nan=False)
        if len(arguments.encode()) > 1_000_000:
            raise ValueError
    except (ValueError, TypeError):
        raise AgentKeyError("X3_AGENT_KEY_CALL") from None
    check_admission_cost(checked=checked, network=network, method=method, arguments=arguments,
                         gas_tgas=gas_tgas, transport=transport)
    # A private snapshot binds the validated bytes to the CLI input, not OS isolation.
    with tempfile.TemporaryDirectory(prefix="x3-agent-key-") as directory:
        snapshot = os.path.join(directory, "access-key.json")
        with os.fdopen(os.open(snapshot, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600), "w") as handle:
            json.dump(credential, handle)
        command = ["near", "--offline", "contract", "call-function", "as-transaction", contract, method,
                   "base64-args", base64.b64encode(arguments.encode("utf-8")).decode(), "prepaid-gas", f"{gas_tgas} Tgas", "attached-deposit", "0 NEAR", "sign-as", account,
                   "network-config", network, "sign-with-access-key-file", snapshot,
                   "--nonce", str(checked["nonce"] + 1), "--block-hash", checked["block_hash"],
                   "--block-height", str(checked["block_height"]), "display"]
        if not 0 <= clock() - started <= 60:
            raise AgentKeyError("X3_AGENT_KEY_STALE")
        try:
            result = (runner or subprocess.run)(command, capture_output=True, text=True, timeout=60, check=False)
        except Exception:
            # Offline display cannot broadcast, even if signing timed out.
            raise AgentKeyError("X3_AGENT_KEY_CLI_SIGNING_FAILED") from None
        output = (result.stdout or "") + "\n" + (result.stderr or "")
        if result.returncode:
            raise AgentKeyError(_local_cli_error(output))
        encoded, transaction_hash = _signed_artifact(output, account=account, public_key=public_key,
                     contract=contract, method=method, arguments=arguments, checked=checked, gas_tgas=gas_tgas)
        if not 0 <= clock() - started <= 60:
            raise AgentKeyError("X3_AGENT_KEY_STALE")
        # Never pass signed bytes back through mutable CLI network configuration.
        # An ambiguous response is reconciled externally by hash, never retried here.
        payload = {"jsonrpc": "2.0", "id": "x3-agent-send", "method": "send_tx",
                   "params": {"signed_tx_base64": encoded, "wait_until": "FINAL"}}
        print(json.dumps({"stage": "signed", "transaction_hash_candidate": transaction_hash,
                          "verified": False, "broadcasted": False}, sort_keys=True), flush=True)
        try:
            status, raw = transport("POST", RPC_URLS[network], {"Content-Type": "application/json"},
                                    json.dumps(payload).encode(), 180.0)
            response = json.loads(raw)
            rpc_result = response.get("result") if isinstance(response, dict) else None
            # nearcore RpcTransactionResponse flattens FinalExecutionOutcomeView.
            outcome = rpc_result if isinstance(rpc_result, dict) else None
            outcome_status = outcome.get("status") if isinstance(outcome, dict) else None
            transaction = outcome.get("transaction") if isinstance(outcome, dict) else None
            explicit_success = (isinstance(outcome_status, dict) and len(outcome_status) == 1
                                and any(isinstance(outcome_status.get(key), str)
                                        for key in ("SuccessValue", "SuccessReceiptId")))
            success = (status == 200 and isinstance(response, dict) and response.get("error") is None
                       and isinstance(rpc_result, dict) and rpc_result.get("final_execution_status") == "FINAL"
                       and explicit_success and isinstance(transaction, dict)
                       and transaction.get("hash") == transaction_hash
                       and transaction.get("signer_id") == account and transaction.get("receiver_id") == contract)
            known_codes = ("X3_MANDATE_DENIED", "X3_STORAGE_BUDGET_EXHAUSTED", "X3_STORAGE_WITHDRAWAL_PENDING")
            safe_text = json.dumps(response)
            codes = [code for code in known_codes if re.search(r"(?<![A-Z0-9_])" + code + r"(?![A-Z0-9_])", safe_text)]
        except Exception:
            success, codes = False, []
        print(json.dumps({"cli_exit_code": 0, "transaction_hash_candidate": transaction_hash,
                          "error_code_candidates": codes, "verified": False, "broadcasted": "unknown",
                          "send_status": "rpc_final_response_unverified" if success else "outcome_unknown"}, sort_keys=True))
        return 0 if success else 1


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
    parser.add_argument("--gas-tgas", type=int, choices=range(1, 31), default=30,
                        help="Explicit owner-reviewed prepaid gas, 1..30 Tgas (default: 30); never auto-adjusted")
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
        return execute(**common, key_file=options.key_file, args=args, gas_tgas=options.gas_tgas)
    except AgentKeyError as exc:
        print(json.dumps({"error": str(exc), "broadcasted": "unknown" if "OUTCOME_UNKNOWN" in str(exc) else False}),
              file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
