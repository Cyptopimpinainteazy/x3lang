#!/usr/bin/env python3
"""Read-only bridge between the .x3 compiler and disposable Sandbox observations.

This verifies consistency with independently supplied source/context, not owner
consent, node authentication, or public-chain finality. No wallet or network I/O.
"""
from __future__ import annotations
import argparse
import base64
import binascii
import json
from pathlib import Path
import re
import sys
from typing import Any

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "x3-lang"))
import near_native_vault as backend

MAX_INPUT = 131_072
CONTEXT_KEYS = {"network", "vault_account", "wasm_sha256", "policy_version", "compiled_at_ms"}
OBSERVATION_KEYS = {"vault_account", "block_hash", "block_height", "code_hash", "configuration"}
B58 = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def exact_keys(value: Any, expected: set[str], label: str) -> None:
    require(type(value) is dict and set(value) == expected, f"invalid {label} fields")


def decode_source(value: Any) -> bytes:
    require(type(value) is str and 0 < len(value) <= 87_384, "invalid source encoding size")
    try:
        raw = base64.b64decode(value, validate=True)
    except (ValueError, binascii.Error) as exc:
        raise ValueError("invalid source base64") from exc
    require(base64.b64encode(raw).decode("ascii") == value, "noncanonical source base64")
    require(0 < len(raw) <= backend.MAX_SOURCE_BYTES, "empty or oversized source")
    return raw


def code_hash(wasm_sha256: str) -> str:
    require(type(wasm_sha256) is str and re.fullmatch(r"[0-9a-f]{64}", wasm_sha256) is not None,
            "invalid WASM digest")
    raw = bytes.fromhex(wasm_sha256)
    number, result = int.from_bytes(raw, "big"), ""
    while number:
        number, digit = divmod(number, 58)
        result = B58[digit] + result
    return "1" * (len(raw) - len(raw.lstrip(b"\x00"))) + result


def compile_binding(request: dict[str, Any]) -> dict[str, Any]:
    exact_keys(request, {"source_base64", "context"}, "compile input")
    context = request["context"]
    exact_keys(context, CONTEXT_KEYS, "context")
    require(context["network"] == "sandbox", "this bridge is disposable Sandbox only")
    source = decode_source(request["source_base64"])
    args = {key: context[key] for key in ("network", "vault_account", "wasm_sha256", "policy_version")}
    manifest = backend.compile_policy(source, issued_at_ms=context["compiled_at_ms"], **args)
    constructor = backend.verify_manifest(manifest, source, now_ms=context["compiled_at_ms"],
                                          expected_compiled_at_ms=context["compiled_at_ms"], **args)
    return {"source_base64": request["source_base64"], "context": dict(context),
            "manifest": manifest, "constructor": constructor}


def verify_readback(binding: dict[str, Any], observation: dict[str, Any], *, now_ms: int) -> dict[str, Any]:
    exact_keys(binding, {"source_base64", "context", "manifest", "constructor"}, "binding")
    # Source and context must come from the verifier's reviewed inputs, not from
    # the untrusted observation. A completely rewritten bundle is not a signature.
    expected = compile_binding({k: binding[k] for k in ("source_base64", "context")})
    require(backend._canonical(binding) == backend._canonical(expected), "binding differs from compiled source/context")
    context = binding["context"]
    constructor = backend.verify_manifest(binding["manifest"], decode_source(binding["source_base64"]),
                                          now_ms=now_ms, expected_compiled_at_ms=context["compiled_at_ms"],
                                          **{k: context[k] for k in ("network", "vault_account", "wasm_sha256", "policy_version")})
    exact_keys(observation, OBSERVATION_KEYS, "observation")
    require(observation["vault_account"] == context["vault_account"], "readback targets a different vault")
    require(observation["code_hash"] == code_hash(context["wasm_sha256"]), "deployed code identity mismatch")
    block_hash = observation["block_hash"]
    require(type(block_hash) is str and 32 <= len(block_hash) <= 44
            and all(c in B58 for c in block_hash), "invalid observed block hash")
    require(type(observation["block_height"]) is int and 0 <= observation["block_height"] < 1 << 64,
            "invalid observed block height")
    require(backend._canonical(observation["configuration"]) == backend._canonical(constructor),
            "observed vault configuration differs from generated constructor")
    return {"configuration_matches": True, "code_hash_matches": True,
            "source_sha256": binding["manifest"]["source_sha256"],
            "manifest_sha256": binding["manifest"]["manifest_sha256"],
            "vault_account": context["vault_account"], "block_hash": block_hash,
            "block_height": observation["block_height"],
            "evidence_kind": "local_sandbox_rpc_observation", "cryptographic_proof": False,
            "owner_authorized": False, "public_chain": False}


def unique(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for key, value in pairs:
        require(key not in result, "duplicate JSON key")
        result[key] = value
    return result


def reject_number(value: str) -> None:
    raise ValueError("floating-point and non-finite numbers are unsupported")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=("compile", "verify"))
    args = parser.parse_args()
    try:
        raw = sys.stdin.buffer.read(MAX_INPUT + 1)
        require(0 < len(raw) <= MAX_INPUT, "empty or oversized input")
        value = json.loads(raw.decode("utf-8"), object_pairs_hook=unique,
                           parse_float=reject_number, parse_constant=reject_number)
        if args.mode == "compile":
            result = compile_binding(value)
        else:
            exact_keys(value, {"binding", "observation", "now_ms"}, "verification input")
            result = verify_readback(value["binding"], value["observation"], now_ms=value["now_ms"])
        print(backend._canonical(result).decode("ascii"))
    except (ValueError, OSError, UnicodeError, RecursionError) as exc:
        print(f"X3_NATIVE_BINDING_REFUSED: {exc}", file=sys.stderr)
        return 2
    return 0

if __name__ == "__main__":
    raise SystemExit(main())
