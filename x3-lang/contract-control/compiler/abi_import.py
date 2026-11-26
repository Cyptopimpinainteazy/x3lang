"""Read-only, bounded ABI inventory for the experimental X3CCP profile."""
import hashlib
import json
import re
from collections import Counter

from compile_action import keccak, PolicyError


class AbiError(ValueError):
    pass


def unique(pairs):
    out = {}
    for k, v in pairs:
        if k in out:
            raise AbiError("duplicate JSON key")
        out[k] = v
    return out


def static_word(t):
    if t in ("address", "bool"):
        return True
    m = re.fullmatch(r"(?:uint|int)([0-9]{1,3})", t)
    if m:
        n = int(m[1])
        return str(n) == m[1] and 8 <= n <= 256 and n % 8 == 0
    m = re.fullmatch(r"bytes([0-9]{1,2})", t)
    return bool(m and str(int(m[1])) == m[1] and 1 <= int(m[1]) <= 32)


def inspect_abi(raw):
    if type(raw) is not bytes or not 0 < len(raw) <= 262144:
        raise AbiError("invalid ABI size")
    try:
        entries = json.loads(raw.decode("utf-8"), object_pairs_hook=unique)
    except (ValueError, UnicodeError, RecursionError) as exc:
        raise AbiError("invalid ABI JSON") from exc
    if not isinstance(entries, list) or len(entries) > 512:
        raise AbiError("expected bounded ABI array")
    rows, nonfunctions, signatures = [], [], set()
    for entry in entries:
        if not isinstance(entry, dict):
            raise AbiError("invalid ABI entry")
        kind = entry.get("type")
        if kind in ("constructor", "receive", "fallback", "event", "error"):
            nonfunctions.append(kind)
            continue
        if kind != "function":
            raise AbiError("unknown ABI entry type")
        name, inputs, mut = (entry.get(k) for k in ("name", "inputs", "stateMutability"))
        if not isinstance(name, str) or not re.fullmatch(r"[A-Za-z_$][A-Za-z0-9_$]{0,127}", name):
            raise AbiError("invalid function name")
        if mut not in ("pure", "view", "nonpayable", "payable"):
            raise AbiError("invalid mutability")
        if not isinstance(inputs, list) or len(inputs) > 128:
            raise AbiError("invalid inputs")
        types = []
        for arg in inputs:
            if not isinstance(arg, dict) or not isinstance(arg.get("type"), str):
                raise AbiError("invalid input type")
            t = arg["type"]
            if not re.fullmatch(r"[A-Za-z0-9]+(?:\[[0-9]*\]){0,16}", t) or len(t) > 128:
                raise AbiError("malformed type")
            types.append(t)
        supported = all(static_word(t) for t in types)
        signature = name + "(" + ",".join(types) + ")" if supported else None
        selector = None
        if signature is not None:
            if signature in signatures:
                raise AbiError("duplicate function signature")
            signatures.add(signature)
            try:
                selector = "0x" + keccak(signature.encode("ascii"))[:4].hex()
            except PolicyError as exc:
                raise AbiError("selector hashing failed") from exc
        reason = ("unsupported input types" if not supported else
                  "too many ABI words" if len(types) > 16 else
                  "read-only function" if mut in ("view", "pure") else None)
        rows.append(dict(name=name, input_types=types, signature=signature,
                         selector=selector, mutability=mut,
                         profile_candidate=reason is None, reason=reason))
    counts = Counter(r["selector"] for r in rows if r["selector"] is not None)
    incomplete = any(r["selector"] is None for r in rows)
    for row in rows:
        if row["selector"] is not None and counts[row["selector"]] > 1:
            row.update(profile_candidate=False, reason="selector collision")
        elif incomplete and row["profile_candidate"]:
            row.update(profile_candidate=False, reason="incomplete selector inventory")
    return dict(schema="x3.abi-inventory.v1", abi_sha256=hashlib.sha256(raw).hexdigest(),
                authority_verified=False, deployment_verified=False,
                incomplete_inventory=incomplete, functions=rows, nonfunctions=nonfunctions)
