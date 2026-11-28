"""X3CCP — X3 Contract Control Policy: an X3Lang `control` block that a Solidity contract enforces.

An owner writes one `control` block for one deployed contract that inherits `X3Controlled`
(x3-lang/x3ccp-evm/src/X3Controlled.sol). The block compiles to canonical X3CCP v1 policy bytes;
the owner installs them with `installPolicy(bytes)`, and from then on every function marked
`x3Controlled` refuses any call that breaks a clause, on chain, in the same transaction.

    control treasury_guard {
        target base-sepolia 0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f
        owner 0x7FA9385bE102ac3EAc297483Dd6233D62b3e1496
        allow callers 0x1111111111111111111111111111111111111111
        allow fn withdraw(address to, uint256 amount)
        limit withdraw.amount <= 50_000_000
        limit withdraw.to in 0x2222222222222222222222222222222222222222
        rate withdraw.amount per 24h <= 200_000_000
        rate withdraw calls per 1h <= 10
        expires 30d
    }

Authority boundary: a policy controls only a contract whose owner integrated `X3Controlled` and
installed it. X3CCP cannot take control of an arbitrary immutable third-party contract.

Every refusal is `X3Refused(uint16 clause, uint8 code)`: `clause` is the 1-based source line of
the clause that refused, so a revert on a block explorer maps straight back to the `.x3` file.
`predict()` here evaluates the same checks, in the same order, with the same codes as the
contract, so an agent learns before it spends gas what the chain will say.

Rate windows are fixed, aligned to the Unix epoch: window = block.timestamp // seconds. They are
NOT rolling windows. Installing a new policy starts a new epoch and resets rate usage.
"""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import os
import re
import sys
from dataclasses import dataclass, field
from datetime import datetime, timedelta, timezone
from typing import Any, Dict, List, Optional, Tuple

try:
    from Crypto.Hash import keccak as _pycryptodome_keccak

    def keccak256(data: bytes) -> bytes:
        return _pycryptodome_keccak.new(digest_bits=256, data=data).digest()
except ImportError:  # pragma: no cover - exercised only without pycryptodome
    _spec = importlib.util.spec_from_file_location(
        "_x3ccp_keccak", os.path.join(os.path.dirname(os.path.abspath(__file__)), "emitter", "_keccak.py")
    )
    _mod = importlib.util.module_from_spec(_spec)
    _spec.loader.exec_module(_mod)
    keccak256 = _mod.keccak_256

if keccak256(b"").hex() != "c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470":
    raise RuntimeError("keccak-256 self-test failed; refusing to compile policies with a wrong hash")

MAGIC = b"X3CP"
VERSION = 1

CHAINS = {"base-sepolia": 84532, "local": 31337}
MAINNET_NAMES = {"base": 8453, "ethereum": 1, "mainnet": 1, "bsc": 56, "optimism": 10, "arbitrum": 42161}

MAX_CALLERS = 16
MAX_FUNCTIONS = 32
MAX_ARGS = 16
MAX_CHECKS = 16
MAX_SET = 16
MIN_WINDOW_S = 1
MAX_WINDOW_S = 365 * 86_400
U64_MAX = (1 << 64) - 1
U256_MAX = (1 << 256) - 1
MAX_LINE = 65_535
MAX_SOURCE_BYTES = 65_536  # bounds compile and grammar work for untrusted (model-written) input

# Type codes in policy bytes. uintN is 0x10 + (N/8 - 1): uint8 = 0x10 ... uint256 = 0x2F.
T_ADDRESS = 0x01
T_BOOL = 0x02
T_BYTES32 = 0x03
T_UINT_BASE = 0x10

# Check kinds in policy bytes.
K_MAX, K_MIN, K_EQ, K_IN, K_RATE_SUM, K_RATE_CALLS = 1, 2, 3, 4, 5, 6
KIND_NAMES = {K_MAX: "max", K_MIN: "min", K_EQ: "eq", K_IN: "in", K_RATE_SUM: "rate_sum", K_RATE_CALLS: "rate_calls"}
NO_ARG = 0xFF

# Refusal codes — identical to X3Controlled.sol.
REFUSAL_CODES = {
    1: "NO_POLICY",
    2: "WRONG_CHAIN",
    3: "EXPIRED",
    4: "CALLER_NOT_ALLOWED",
    5: "FUNCTION_NOT_ALLOWED",
    6: "BAD_CALLDATA",
    7: "ABOVE_MAX",
    8: "BELOW_MIN",
    9: "NOT_EQUAL",
    10: "NOT_IN_SET",
    11: "RATE_SUM_EXCEEDED",
    12: "RATE_CALLS_EXCEEDED",
}
CODE = {name: num for num, name in REFUSAL_CODES.items()}

# keccak256("X3Refused(uint16,uint8)")[:4]
REFUSED_SELECTOR = keccak256(b"X3Refused(uint16,uint8)")[:4]

_IDENT = r"[A-Za-z_][A-Za-z0-9_]*"
_DURATION_UNITS = {"s": 1, "m": 60, "h": 3_600, "d": 86_400}


class X3CCPError(Exception):
    """A compile error with a stable code, the source line, and a concrete fix for a human or a model."""

    def __init__(self, code: str, message: str, line: Optional[int] = None, field: Optional[str] = None,
                 hint: Optional[str] = None):
        super().__init__(message)
        self.code = code
        self.message = message
        self.line = line
        self.field = field
        self.hint = hint

    def to_dict(self) -> Dict[str, Any]:
        out: Dict[str, Any] = {"code": self.code, "message": self.message}
        if self.line is not None:
            out["line"] = self.line
        if self.field:
            out["field"] = self.field
        if self.hint:
            out["hint"] = self.hint
        return out


@dataclass
class _Line:
    no: int
    text: str


def _clean(source: str) -> List[_Line]:
    out: List[_Line] = []
    for no, raw in enumerate(source.split("\n"), 1):
        text = re.sub(r"//.*$", "", raw).strip().rstrip(";").strip()
        if text:
            out.append(_Line(no, text))
    return out


# ---------------------------------------------------------------------------- values


def _address(text: str, line: int, field: str) -> str:
    if not re.fullmatch(r"0x[0-9a-fA-F]{40}", text):
        raise X3CCPError("X3CCP_ADDRESS", f"{text!r} is not a 20-byte hex address", line, field,
                         "write an address as 0x followed by exactly 40 hex digits")
    body = text[2:]
    if body != body.lower() and body != body.upper() and to_checksum(text) != text:
        raise X3CCPError("X3CCP_CHECKSUM", f"{text} has mixed case but a wrong EIP-55 checksum", line, field,
                         f"did you mean {to_checksum(text)}? Check the address against its source")
    if int(body, 16) == 0:
        raise X3CCPError("X3CCP_ADDRESS", "the zero address is not allowed here", line, field,
                         "use the real account or contract address")
    return to_checksum(text)


def to_checksum(addr: str) -> str:
    body = addr[2:].lower()
    digest = keccak256(body.encode()).hex()
    return "0x" + "".join(c.upper() if c.isalpha() and int(digest[i], 16) >= 8 else c for i, c in enumerate(body))


def _uint(text: str, bits: int, line: int, field: str) -> int:
    clean = text.replace("_", "")
    if re.fullmatch(r"0x[0-9a-fA-F]+", clean):
        value = int(clean, 16)
    elif re.fullmatch(r"[0-9]+", clean):
        value = int(clean, 10)
    else:
        raise X3CCPError("X3CCP_NUMBER", f"{text!r} is not an unsigned integer", line, field,
                         "write base units as a whole number, e.g. 50_000_000 for 50 USDC at 6 decimals")
    if value > (1 << bits) - 1:
        raise X3CCPError("X3CCP_NUMBER", f"{text} does not fit in uint{bits}", line, field,
                         f"the largest uint{bits} is {(1 << bits) - 1}")
    return value


def _duration_s(text: str, line: int, field: str) -> int:
    m = re.fullmatch(r"([0-9][0-9_]*)\s*([smhd])", text)
    if not m:
        raise X3CCPError("X3CCP_DURATION", f"{text!r} is not a duration", line, field,
                         "write a duration as a number and a unit: 30s, 10m, 1h or 7d")
    seconds = int(m.group(1).replace("_", "")) * _DURATION_UNITS[m.group(2)]
    return seconds


def _type_code(type_name: str, line: int) -> Tuple[int, int]:
    """Return (type code, bits) for a static ABI type the enforcer can read from one calldata word."""
    if type_name == "address":
        return T_ADDRESS, 160
    if type_name == "bool":
        return T_BOOL, 1
    if type_name == "bytes32":
        return T_BYTES32, 256
    m = re.fullmatch(r"uint([0-9]*)", type_name)
    if m:
        bits = int(m.group(1)) if m.group(1) else 256
        if bits % 8 == 0 and 8 <= bits <= 256:
            return T_UINT_BASE + bits // 8 - 1, bits
    raise X3CCPError("X3CCP_TYPE", f"`{type_name}` is not a supported parameter type", line, "allow fn",
                     "X3CCP v1 reads static one-word ABI types only: address, bool, bytes32, uint8..uint256")


def _canonical_type(type_name: str) -> str:
    return "uint256" if type_name == "uint" else type_name


def type_name_of(code: int) -> str:
    if code == T_ADDRESS:
        return "address"
    if code == T_BOOL:
        return "bool"
    if code == T_BYTES32:
        return "bytes32"
    if T_UINT_BASE <= code <= T_UINT_BASE + 31:
        return f"uint{(code - T_UINT_BASE + 1) * 8}"
    raise ValueError(f"unknown type code {code}")


def _value_word(text: str, type_code: int, bits: int, line: int, field: str) -> int:
    if type_code == T_ADDRESS:
        return int(_address(text, line, field), 16)
    if type_code == T_BOOL:
        if text not in ("true", "false"):
            raise X3CCPError("X3CCP_VALUE", f"{text!r} is not a bool", line, field, "write true or false")
        return 1 if text == "true" else 0
    if type_code == T_BYTES32:
        if not re.fullmatch(r"0x[0-9a-fA-F]{64}", text):
            raise X3CCPError("X3CCP_VALUE", f"{text!r} is not a bytes32 value", line, field,
                             "write bytes32 as 0x followed by exactly 64 hex digits")
        return int(text, 16)
    return _uint(text, bits, line, field)


def _split_list(text: str) -> List[str]:
    return [item.strip() for item in text.split(",") if item.strip()]


# ---------------------------------------------------------------------------- parse + compile


def parse(source: str) -> Dict[str, Any]:
    lines = _clean(source)
    if not lines:
        raise X3CCPError("X3CCP_PARSE", "the file is empty", None, "control",
                         "start with `control <name> {`")
    head = re.fullmatch(rf"control\s+({_IDENT})\s*\{{", lines[0].text)
    if not head:
        raise X3CCPError("X3CCP_PARSE", "expected `control <name> {` as the first line", lines[0].no, "control",
                         "a control policy starts with `control my_policy {`")
    out: Dict[str, Any] = {"name": head.group(1), "header_line": lines[0].no, "functions": [], "limits": []}
    closed = False
    seen: Dict[str, int] = {}

    def once(key: str, ln: _Line) -> None:
        if key in seen:
            raise X3CCPError("X3CCP_DUPLICATE", f"`{key}` appears twice (first on line {seen[key]})", ln.no, key,
                             f"keep one `{key}` clause")
        seen[key] = ln.no

    for ln in lines[1:]:
        text = ln.text
        if closed:
            raise X3CCPError("X3CCP_PARSE", f"unexpected {text!r} after the closing brace", ln.no, "control",
                             "put every clause inside the braces; one control block per file")
        if text == "}":
            closed = True
            continue
        key = text.split()[0]
        if key == "target":
            once("target", ln)
            m = re.fullmatch(r"target\s+(\S+)\s+(\S+)", text)
            if not m:
                raise X3CCPError("X3CCP_PARSE", "expected `target <chain> <contract address>`", ln.no, "target",
                                 "e.g. `target base-sepolia 0x…`")
            chain = m.group(1)
            if chain in MAINNET_NAMES:
                raise X3CCPError("X3CCP_MAINNET_REFUSED", f"`{chain}` is a mainnet; X3CCP v1 is testnet-only",
                                 ln.no, "target", "use `base-sepolia` or `local` until the enforcer is audited")
            if chain not in CHAINS:
                raise X3CCPError("X3CCP_CHAIN", f"unknown chain `{chain}`", ln.no, "target",
                                 f"supported chains: {', '.join(sorted(CHAINS))}")
            out["chain"] = chain
            out["chain_id"] = CHAINS[chain]
            out["target"] = _address(m.group(2), ln.no, "target")
            out["target_line"] = ln.no
        elif key == "owner":
            once("owner", ln)
            parts = text.split()
            if len(parts) != 2:
                raise X3CCPError("X3CCP_PARSE", "expected `owner <address>`", ln.no, "owner",
                                 "the owner is the account that deployed the contract and installs this policy")
            out["owner"] = _address(parts[1], ln.no, "owner")
            out["owner_line"] = ln.no
        elif text.startswith("allow callers"):
            once("allow callers", ln)
            items = _split_list(text[len("allow callers"):])
            if not items:
                raise X3CCPError("X3CCP_PARSE", "`allow callers` lists no address", ln.no, "allow callers",
                                 "list at least one caller: `allow callers 0x…, 0x…`")
            if len(items) > MAX_CALLERS:
                raise X3CCPError("X3CCP_LIMIT", f"at most {MAX_CALLERS} callers", ln.no, "allow callers",
                                 "split duties across separate contracts or reduce the list")
            callers = [_address(item, ln.no, "allow callers") for item in items]
            if len(set(callers)) != len(callers):
                raise X3CCPError("X3CCP_DUPLICATE", "a caller is listed twice", ln.no, "allow callers",
                                 "list each caller once")
            out["callers"] = callers
            out["callers_line"] = ln.no
        elif text.startswith("allow fn"):
            m = re.fullmatch(rf"allow\s+fn\s+({_IDENT})\s*\((.*)\)", text)
            if not m:
                raise X3CCPError("X3CCP_PARSE", "expected `allow fn <name>(<type> <param>, …)`", ln.no,
                                 "allow fn", "e.g. `allow fn withdraw(address to, uint256 amount)`")
            params = []
            for raw in _split_list(m.group(2)):
                pm = re.fullmatch(rf"({_IDENT})\s+({_IDENT})", raw)
                if not pm:
                    raise X3CCPError("X3CCP_PARSE", f"parameter {raw!r} needs a type and a name", ln.no,
                                     "allow fn", "write each parameter as `<type> <name>`, e.g. `uint256 amount`")
                type_name = _canonical_type(pm.group(1))
                code, bits = _type_code(type_name, ln.no)
                params.append({"name": pm.group(2), "type": type_name, "type_code": code, "bits": bits})
            if len(params) > MAX_ARGS:
                raise X3CCPError("X3CCP_LIMIT", f"at most {MAX_ARGS} parameters", ln.no, "allow fn")
            names = [p["name"] for p in params]
            if len(set(names)) != len(names):
                raise X3CCPError("X3CCP_DUPLICATE", "two parameters share a name", ln.no, "allow fn",
                                 "give each parameter a distinct name")
            out["functions"].append({"name": m.group(1), "params": params, "line": ln.no})
        elif key == "limit":
            m = re.fullmatch(rf"limit\s+({_IDENT})\.({_IDENT})\s*(<=|>=|==|in)\s*(.+)", text)
            if not m:
                raise X3CCPError("X3CCP_PARSE", "expected `limit <fn>.<param> <=|>=|==|in <value>`", ln.no, "limit",
                                 "e.g. `limit withdraw.amount <= 50_000_000` or `limit withdraw.to in 0x…, 0x…`")
            out["limits"].append({"kind": "limit", "fn": m.group(1), "param": m.group(2), "op": m.group(3),
                                  "value": m.group(4).strip(), "line": ln.no})
        elif key == "rate":
            m = re.fullmatch(rf"rate\s+({_IDENT})\.({_IDENT})\s+per\s+(\S+)\s*<=\s*(\S+)", text)
            c = re.fullmatch(rf"rate\s+({_IDENT})\s+calls\s+per\s+(\S+)\s*<=\s*(\S+)", text)
            if m:
                out["limits"].append({"kind": "rate_sum", "fn": m.group(1), "param": m.group(2),
                                      "window": m.group(3), "value": m.group(4), "line": ln.no})
            elif c:
                out["limits"].append({"kind": "rate_calls", "fn": c.group(1), "window": c.group(2),
                                      "value": c.group(3), "line": ln.no})
            else:
                raise X3CCPError("X3CCP_PARSE", "expected `rate <fn>.<param> per <duration> <= N` or "
                                 "`rate <fn> calls per <duration> <= N`", ln.no, "rate",
                                 "e.g. `rate withdraw.amount per 24h <= 200_000_000`")
        elif key == "expires":
            once("expires", ln)
            at = re.fullmatch(r"expires\s+at\s+(\S+)", text)
            rel = re.fullmatch(r"expires\s+(\S+)", text)
            if at:
                out["expires"] = {"at": at.group(1)}
            elif rel:
                out["expires"] = {"after": rel.group(1)}
            else:
                raise X3CCPError("X3CCP_PARSE", "expected `expires <duration>` or `expires at <UTC time>`", ln.no,
                                 "expires", "e.g. `expires 30d` or `expires at 2027-01-01T00:00:00Z`")
            out["expires_line"] = ln.no
        else:
            raise X3CCPError("X3CCP_PARSE", f"{text!r} is not a control clause", ln.no, "control",
                             "clauses: target, owner, allow callers, allow fn, limit, rate, expires")
    if not closed:
        raise X3CCPError("X3CCP_PARSE", "the control block is never closed", lines[-1].no, "control",
                         "end the block with `}`")
    for required, hint in (("target", "`target base-sepolia 0x…` names the contract this policy controls"),
                           ("owner", "`owner 0x…` must equal the contract's owner"),
                           ("callers", "`allow callers 0x…` lists who may call controlled functions"),
                           ("expires", "`expires 30d`: every policy ends; renew it by installing a new one")):
        if required not in out:
            raise X3CCPError("X3CCP_MISSING", f"a control policy must state `{required}`", None, required, hint)
    if not out["functions"]:
        raise X3CCPError("X3CCP_MISSING", "a control policy must allow at least one function", None, "allow fn",
                         "add `allow fn <name>(<type> <param>, …)`; every unlisted function is refused")
    return out


def _resolve_expiry(spec: Dict[str, str], line: int, now: datetime) -> int:
    if "at" in spec:
        try:
            when = datetime.fromisoformat(spec["at"].replace("Z", "+00:00"))
        except ValueError as exc:
            raise X3CCPError("X3CCP_EXPIRY", f"{spec['at']!r} is not an ISO-8601 time", line, "expires",
                             "write UTC time like 2027-01-01T00:00:00Z") from exc
        if when.tzinfo is None:
            raise X3CCPError("X3CCP_EXPIRY", "the expiry time has no time zone", line, "expires",
                             "end it with Z for UTC")
        expires_at = int(when.timestamp())
    else:
        seconds = _duration_s(spec["after"], line, "expires")
        if seconds <= 0:
            raise X3CCPError("X3CCP_EXPIRY", "the policy must live longer than zero seconds", line, "expires")
        expires_at = int(now.timestamp()) + seconds
    if expires_at <= int(now.timestamp()):
        raise X3CCPError("X3CCP_EXPIRY", "the policy would already be expired", line, "expires",
                         "choose a time in the future")
    if expires_at > U64_MAX:
        raise X3CCPError("X3CCP_EXPIRY", "the expiry does not fit in uint64", line, "expires")
    return expires_at


def compile_source(source: str, *, now: Optional[datetime] = None) -> Dict[str, Any]:
    """Compile a `control` block to X3CCP v1 policy bytes. Raises X3CCPError on any problem."""
    now = now or datetime.now(timezone.utc)
    if len(source.encode("utf-8")) > MAX_SOURCE_BYTES:
        raise X3CCPError("X3CCP_LIMIT", f"the source is larger than {MAX_SOURCE_BYTES} bytes", None, "control",
                         "one control block is a few dozen lines; split unrelated policies")
    parsed = parse(source)
    for key in ("header_line", "target_line", "owner_line", "callers_line", "expires_line"):
        if parsed.get(key, 0) > MAX_LINE:
            raise X3CCPError("X3CCP_LIMIT", f"source lines above {MAX_LINE} cannot be clause ids", parsed[key])
    fns: Dict[str, Dict[str, Any]] = {}
    selectors: Dict[bytes, str] = {}
    for fn in parsed["functions"]:
        if fn["name"] in fns:
            raise X3CCPError("X3CCP_DUPLICATE", f"function `{fn['name']}` is allowed twice", fn["line"], "allow fn",
                             "X3CCP refers to functions by name; overloads are not supported in v1")
        signature = f"{fn['name']}({','.join(p['type'] for p in fn['params'])})"
        selector = keccak256(signature.encode())[:4]
        if selector in selectors:
            raise X3CCPError("X3CCP_SELECTOR", f"`{signature}` collides with `{selectors[selector]}`", fn["line"],
                             "allow fn", "rename one of the functions")
        selectors[selector] = signature
        fns[fn["name"]] = {**fn, "signature": signature, "selector": "0x" + selector.hex(), "checks": []}
    if len(fns) > MAX_FUNCTIONS:
        raise X3CCPError("X3CCP_LIMIT", f"at most {MAX_FUNCTIONS} functions", None, "allow fn")
    warnings: List[Dict[str, Any]] = []
    for lim in parsed["limits"]:
        line = lim["line"]
        fn = fns.get(lim["fn"])
        if fn is None:
            known = ", ".join(sorted(fns)) or "none"
            raise X3CCPError("X3CCP_UNKNOWN_FN", f"`{lim['fn']}` is not an allowed function", line, lim["kind"],
                             f"allow it first with `allow fn {lim['fn']}(…)`; allowed: {known}")
        arg_index = NO_ARG
        param = None
        if lim["kind"] != "rate_calls":
            index = next((i for i, p in enumerate(fn["params"]) if p["name"] == lim["param"]), None)
            if index is None:
                names = ", ".join(p["name"] for p in fn["params"]) or "none"
                raise X3CCPError("X3CCP_UNKNOWN_PARAM", f"`{lim['fn']}` has no parameter `{lim['param']}`", line,
                                 lim["kind"], f"parameters of {fn['signature']}: {names}")
            arg_index, param = index, fn["params"][index]
        check: Dict[str, Any] = {"line": line, "arg": arg_index, "window": 0, "value": 0, "set": []}
        is_uint = param is not None and param["type_code"] >= T_UINT_BASE
        if lim["kind"] == "limit":
            op = lim["op"]
            if op in ("<=", ">=") and not is_uint:
                raise X3CCPError("X3CCP_OPERATOR", f"`{op}` needs an unsigned integer; `{lim['param']}` is "
                                 f"{param['type']}", line, "limit", "use `==` or `in` for address, bool and bytes32")
            if op == "in":
                if param["type_code"] == T_BOOL:
                    raise X3CCPError("X3CCP_OPERATOR", "`in` on a bool is always `==`", line, "limit",
                                     f"write `limit {lim['fn']}.{lim['param']} == true` (or false)")
                items = _split_list(lim["value"])
                if not items or len(items) > MAX_SET:
                    raise X3CCPError("X3CCP_LIMIT", f"`in` needs 1..{MAX_SET} values", line, "limit")
                words = [_value_word(v, param["type_code"], param["bits"], line, "limit") for v in items]
                if len(set(words)) != len(words):
                    raise X3CCPError("X3CCP_DUPLICATE", "a value is listed twice", line, "limit")
                check.update(kind=K_IN, set=words)
            else:
                word = _value_word(lim["value"], param["type_code"], param["bits"], line, "limit")
                check.update(kind={"<=": K_MAX, ">=": K_MIN, "==": K_EQ}[op], value=word)
        else:
            seconds = _duration_s(lim["window"], line, "rate")
            if not MIN_WINDOW_S <= seconds <= MAX_WINDOW_S:
                raise X3CCPError("X3CCP_DURATION", "a rate window must be 1s..365d", line, "rate")
            if lim["kind"] == "rate_sum":
                if not is_uint:
                    raise X3CCPError("X3CCP_OPERATOR", f"a rate sums an unsigned integer; `{lim['param']}` is "
                                     f"{param['type']}", line, "rate",
                                     f"count calls instead: `rate {lim['fn']} calls per {lim['window']} <= N`")
                value = _uint(lim["value"], 256, line, "rate")
                kind = K_RATE_SUM
            else:
                value = _uint(lim["value"], 32, line, "rate")
                kind = K_RATE_CALLS
            if value == 0:
                raise X3CCPError("X3CCP_CONFLICT", "a rate of 0 refuses every call", line, "rate",
                                 f"to forbid `{lim['fn']}`, remove its `allow fn` line instead")
            check.update(kind=kind, window=seconds, value=value)
        fn["checks"].append(check)
    for fn in fns.values():
        fn["checks"].sort(key=lambda c: c["line"])
        if len(fn["checks"]) > MAX_CHECKS:
            raise X3CCPError("X3CCP_LIMIT", f"at most {MAX_CHECKS} checks per function", fn["line"], "limit")
        _lint_function(fn, warnings)
    expires_at = _resolve_expiry(parsed["expires"], parsed["expires_line"], now)
    compiled = {
        "format": "x3ccp-v1",
        "name": parsed["name"],
        "chain": parsed["chain"],
        "chain_id": parsed["chain_id"],
        "target": parsed["target"],
        "owner": parsed["owner"],
        "callers": parsed["callers"],
        "expires_at": expires_at,
        "lines": {k: parsed[k] for k in ("header_line", "target_line", "owner_line", "callers_line", "expires_line")},
        "functions": [_public_fn(fn) for fn in fns.values()],
        "warnings": warnings,
        "source_sha256": hashlib.sha256(source.encode()).hexdigest(),
    }
    policy = encode_policy(compiled)
    compiled["policy_hex"] = "0x" + policy.hex()
    compiled["policy_hash"] = "0x" + keccak256(policy).hex()
    if decode_policy(policy) != _decodable_view(compiled):
        raise RuntimeError("X3CCP encoder/decoder round trip mismatch")
    return compiled


def _public_fn(fn: Dict[str, Any]) -> Dict[str, Any]:
    return {
        "name": fn["name"], "signature": fn["signature"], "selector": fn["selector"], "line": fn["line"],
        "params": [{"name": p["name"], "type": p["type"], "type_code": p["type_code"]} for p in fn["params"]],
        "checks": [{"kind": KIND_NAMES[c["kind"]], "kind_code": c["kind"], "line": c["line"], "arg": c["arg"],
                    "window": c["window"], "value": c["value"], "set": c["set"]} for c in fn["checks"]],
    }


def _lint_function(fn: Dict[str, Any], warnings: List[Dict[str, Any]]) -> None:
    by_arg: Dict[int, Dict[str, Any]] = {}
    for c in fn["checks"]:
        slot = by_arg.setdefault(c["arg"], {})
        key = (c["kind"], c["window"])
        if key in slot and c["kind"] != K_IN:
            raise X3CCPError("X3CCP_DUPLICATE", f"line {slot[key]['line']} already bounds this the same way",
                             c["line"], KIND_NAMES[c["kind"]], "keep the one bound you mean")
        slot[key] = c
    for arg, slot in by_arg.items():
        mx = slot.get((K_MAX, 0))
        mn = slot.get((K_MIN, 0))
        if mx and mn and mn["value"] > mx["value"]:
            raise X3CCPError("X3CCP_CONFLICT", f"minimum {mn['value']} is above maximum {mx['value']}; every call "
                             "would be refused", mn["line"], "limit", f"see line {mx['line']}")
        eq = slot.get((K_EQ, 0))
        if eq and ((mx and eq["value"] > mx["value"]) or (mn and eq["value"] < mn["value"])):
            raise X3CCPError("X3CCP_CONFLICT", "the required value is outside the other bounds", eq["line"], "limit")
        for (kind, _window), rate in slot.items():
            if kind == K_RATE_SUM and mx and mx["value"] > rate["value"]:
                warnings.append({"code": "X3CCP_CAP_SHADOWED", "line": mx["line"],
                                 "message": f"the per-call maximum {mx['value']} is above the window budget "
                                            f"{rate['value']} on line {rate['line']}; the effective per-call cap is "
                                            f"{rate['value']}"})


# ---------------------------------------------------------------------------- binary format


def encode_policy(compiled: Dict[str, Any]) -> bytes:
    lines = compiled["lines"]
    out = bytearray(MAGIC)
    out += bytes([VERSION])
    out += compiled["chain_id"].to_bytes(8, "big")
    out += bytes.fromhex(compiled["target"][2:])
    out += bytes.fromhex(compiled["owner"][2:])
    out += compiled["expires_at"].to_bytes(8, "big")
    out += lines["header_line"].to_bytes(2, "big")
    out += lines["expires_line"].to_bytes(2, "big")
    out += lines["callers_line"].to_bytes(2, "big")
    out += bytes([len(compiled["callers"])])
    for caller in compiled["callers"]:
        out += bytes.fromhex(caller[2:])
    out += bytes([len(compiled["functions"])])
    for fn in compiled["functions"]:
        out += bytes.fromhex(fn["selector"][2:])
        out += fn["line"].to_bytes(2, "big")
        out += bytes([len(fn["params"])])
        out += bytes(p["type_code"] for p in fn["params"])
        out += bytes([len(fn["checks"])])
        for c in fn["checks"]:
            out += bytes([c["kind_code"]])
            out += c["line"].to_bytes(2, "big")
            out += bytes([c["arg"]])
            if c["kind_code"] in (K_MAX, K_MIN, K_EQ):
                out += c["value"].to_bytes(32, "big")
            elif c["kind_code"] == K_IN:
                out += bytes([len(c["set"])])
                for word in c["set"]:
                    out += word.to_bytes(32, "big")
            else:
                out += c["window"].to_bytes(4, "big")
                out += c["value"].to_bytes(32, "big")
    return bytes(out)


class _Reader:
    def __init__(self, data: bytes):
        self.data = data
        self.pos = 0

    def take(self, n: int) -> bytes:
        if self.pos + n > len(self.data):
            raise ValueError("policy bytes end early")
        chunk = self.data[self.pos:self.pos + n]
        self.pos += n
        return chunk

    def uint(self, n: int) -> int:
        return int.from_bytes(self.take(n), "big")


def decode_policy(data: bytes) -> Dict[str, Any]:
    """Strictly decode X3CCP v1 bytes with the same rules as X3Controlled.installPolicy."""
    r = _Reader(data)
    if r.take(4) != MAGIC:
        raise ValueError("bad magic")
    if r.uint(1) != VERSION:
        raise ValueError("unsupported version")
    out: Dict[str, Any] = {"chain_id": r.uint(8), "target": to_checksum("0x" + r.take(20).hex()),
                           "owner": to_checksum("0x" + r.take(20).hex()), "expires_at": r.uint(8),
                           "header_line": r.uint(2), "expires_line": r.uint(2), "callers_line": r.uint(2)}
    n_callers = r.uint(1)
    if not 1 <= n_callers <= MAX_CALLERS:
        raise ValueError("caller count out of range")
    out["callers"] = [to_checksum("0x" + r.take(20).hex()) for _ in range(n_callers)]
    if len(set(out["callers"])) != n_callers:
        raise ValueError("duplicate caller")
    n_fns = r.uint(1)
    if not 1 <= n_fns <= MAX_FUNCTIONS:
        raise ValueError("function count out of range")
    fns = []
    for _ in range(n_fns):
        fn: Dict[str, Any] = {"selector": "0x" + r.take(4).hex(), "line": r.uint(2)}
        n_args = r.uint(1)
        if n_args > MAX_ARGS:
            raise ValueError("argument count out of range")
        fn["types"] = list(r.take(n_args))
        for t in fn["types"]:
            type_name_of(t)
        n_checks = r.uint(1)
        if n_checks > MAX_CHECKS:
            raise ValueError("check count out of range")
        checks = []
        for _ in range(n_checks):
            kind = r.uint(1)
            c: Dict[str, Any] = {"kind": kind, "line": r.uint(2), "arg": r.uint(1), "window": 0, "value": 0, "set": []}
            if kind == K_RATE_CALLS:
                if c["arg"] != NO_ARG:
                    raise ValueError("call-rate check names an argument")
            elif c["arg"] >= n_args:
                raise ValueError("check argument out of range")
            if kind in (K_MAX, K_MIN, K_EQ):
                c["value"] = r.uint(32)
            elif kind == K_IN:
                n = r.uint(1)
                if not 1 <= n <= MAX_SET:
                    raise ValueError("set size out of range")
                c["set"] = [r.uint(32) for _ in range(n)]
                if len(set(c["set"])) != n:
                    raise ValueError("duplicate set value")
            elif kind in (K_RATE_SUM, K_RATE_CALLS):
                c["window"] = r.uint(4)
                c["value"] = r.uint(32)
                if not MIN_WINDOW_S <= c["window"] <= MAX_WINDOW_S or c["value"] == 0:
                    raise ValueError("rate out of range")
            else:
                raise ValueError("unknown check kind")
            if kind in (K_MAX, K_MIN, K_RATE_SUM) and fn["types"][c["arg"]] < T_UINT_BASE:
                raise ValueError("numeric check on a non-integer argument")
            checks.append(c)
        fn["checks"] = checks
        fns.append(fn)
    if len({f["selector"] for f in fns}) != n_fns:
        raise ValueError("duplicate selector")
    if r.pos != len(data):
        raise ValueError("trailing bytes after policy")
    out["functions"] = fns
    return out


def _decodable_view(compiled: Dict[str, Any]) -> Dict[str, Any]:
    lines = compiled["lines"]
    return {
        "chain_id": compiled["chain_id"], "target": compiled["target"], "owner": compiled["owner"],
        "expires_at": compiled["expires_at"], "header_line": lines["header_line"],
        "expires_line": lines["expires_line"], "callers_line": lines["callers_line"],
        "callers": compiled["callers"],
        "functions": [{"selector": f["selector"], "line": f["line"], "types": [p["type_code"] for p in f["params"]],
                       "checks": [{"kind": c["kind_code"], "line": c["line"], "arg": c["arg"], "window": c["window"],
                                   "value": c["value"], "set": c["set"]} for c in f["checks"]]}
                      for f in compiled["functions"]],
    }


# ---------------------------------------------------------------------------- prediction


@dataclass
class EnforcerState:
    """Mirror of one X3Controlled contract's enforcement state for one installed policy epoch."""

    active: bool = True
    usage: Dict[Tuple[str, int, int], int] = field(default_factory=dict)


def _word_ok(word: int, type_code: int) -> bool:
    if type_code == T_ADDRESS:
        return word >> 160 == 0
    if type_code == T_BOOL:
        return word <= 1
    if type_code == T_BYTES32:
        return True
    bits = (type_code - T_UINT_BASE + 1) * 8
    return word >> bits == 0


def _refuse(line: int, code: str) -> Dict[str, Any]:
    return {"allowed": False, "clause": line, "code": CODE[code], "reason": code}


def predict(compiled: Dict[str, Any], state: EnforcerState, *, sender: str, calldata: bytes, timestamp: int,
            chain_id: int, commit: bool = True) -> Dict[str, Any]:
    """Evaluate one call exactly as the chain does. Commits rate usage only when allowed.

    Order: Solidity's ABI decoder for the matched function runs first (a short call or a dirty
    word reverts with empty data, whatever the policy state), then X3Controlled._x3Enforce.
    Assumes the call sends no ETH to a non-payable function (that also reverts before decoding).
    """
    view = _decodable_view(compiled)
    if len(calldata) >= 4:
        early = next((f for f in view["functions"] if f["selector"] == "0x" + calldata[:4].hex()), None)
        if early is not None:
            n = len(early["types"])
            if len(calldata) < 4 + 32 * n:
                return {**_refuse(early["line"], "BAD_CALLDATA"), "abi_decoder_first": True}
            for i, type_code in enumerate(early["types"]):
                if not _word_ok(int.from_bytes(calldata[4 + 32 * i:36 + 32 * i], "big"), type_code):
                    return {**_refuse(early["line"], "BAD_CALLDATA"), "abi_decoder_first": True}
    if not state.active:
        return _refuse(0, "NO_POLICY")
    if chain_id != view["chain_id"]:
        return _refuse(view["header_line"], "WRONG_CHAIN")
    if timestamp >= view["expires_at"]:
        return _refuse(view["expires_line"], "EXPIRED")
    if to_checksum(sender) not in view["callers"]:
        return _refuse(view["callers_line"], "CALLER_NOT_ALLOWED")
    if len(calldata) < 4:
        return _refuse(view["header_line"], "FUNCTION_NOT_ALLOWED")
    selector = "0x" + calldata[:4].hex()
    fn = next((f for f in view["functions"] if f["selector"] == selector), None)
    if fn is None:
        return _refuse(view["header_line"], "FUNCTION_NOT_ALLOWED")
    if len(calldata) != 4 + 32 * len(fn["types"]):
        return _refuse(fn["line"], "BAD_CALLDATA")
    # Words were validated above, as the ABI decoder does; X3Controlled re-checks them on chain as
    # defense in depth for contracts that read calldata without Solidity's decoder.
    words = [int.from_bytes(calldata[4 + 32 * i:36 + 32 * i], "big") for i in range(len(fn["types"]))]
    pending: Dict[Tuple[str, int, int], int] = {}
    for index, c in enumerate(fn["checks"]):
        kind = c["kind"]
        word = words[c["arg"]] if c["arg"] != NO_ARG else 0
        if kind == K_MAX and word > c["value"]:
            return _refuse(c["line"], "ABOVE_MAX")
        if kind == K_MIN and word < c["value"]:
            return _refuse(c["line"], "BELOW_MIN")
        if kind == K_EQ and word != c["value"]:
            return _refuse(c["line"], "NOT_EQUAL")
        if kind == K_IN and word not in c["set"]:
            return _refuse(c["line"], "NOT_IN_SET")
        if kind in (K_RATE_SUM, K_RATE_CALLS):
            key = (selector, index, timestamp // c["window"])
            add = word if kind == K_RATE_SUM else 1
            new = state.usage.get(key, 0) + add
            if new > c["value"]:
                return _refuse(c["line"], "RATE_SUM_EXCEEDED" if kind == K_RATE_SUM else "RATE_CALLS_EXCEEDED")
            pending[key] = new
    if commit:
        state.usage.update(pending)
    return {"allowed": True, "clause": None, "code": 0, "reason": "ALLOWED"}


def encode_call(compiled: Dict[str, Any], fn_name: str, args: List[Any]) -> bytes:
    """ABI-encode a call to an allowed function from human values (no checks are applied here)."""
    fn = next((f for f in compiled["functions"] if f["name"] == fn_name), None)
    if fn is None:
        raise X3CCPError("X3CCP_UNKNOWN_FN", f"`{fn_name}` is not allowed by this policy", None, "call",
                         "allowed: " + ", ".join(f["name"] for f in compiled["functions"]))
    if len(args) != len(fn["params"]):
        raise X3CCPError("X3CCP_CALL", f"{fn['signature']} takes {len(fn['params'])} arguments, got {len(args)}",
                         None, "call")
    out = bytearray(bytes.fromhex(fn["selector"][2:]))
    for value, p in zip(args, fn["params"]):
        code = p["type_code"]
        if code == T_ADDRESS:
            word = int(str(value), 16)
        elif code == T_BOOL:
            word = 1 if value in (True, "true", 1) else 0
        elif isinstance(value, int):
            word = value
        else:
            text = str(value).replace("_", "")
            word = int(text, 16) if text.startswith("0x") else int(text)
        if not 0 <= word <= U256_MAX:
            raise X3CCPError("X3CCP_CALL", f"argument {p['name']} is out of range", None, "call")
        out += word.to_bytes(32, "big")
    return bytes(out)


def refusal_revert_data(clause: int, code: int) -> bytes:
    return REFUSED_SELECTOR + clause.to_bytes(32, "big") + code.to_bytes(32, "big")


def decode_refusal(revert_data: bytes) -> Optional[Dict[str, Any]]:
    if len(revert_data) != 68 or revert_data[:4] != REFUSED_SELECTOR:
        return None
    clause = int.from_bytes(revert_data[4:36], "big")
    code = int.from_bytes(revert_data[36:68], "big")
    return {"clause": clause, "code": code, "reason": REFUSAL_CODES.get(code, "UNKNOWN")}


# ---------------------------------------------------------------------------- plain English


def _fmt_duration(seconds: int) -> str:
    if seconds % 86_400 == 0 and seconds >= 2 * 86_400:
        return f"{seconds // 86_400}-day"
    for unit, size in (("hour", 3_600), ("minute", 60)):
        if seconds % size == 0:
            return f"{seconds // size}-{unit}"
    return f"{seconds}-second"


def _fmt_word(word: int, type_code: int) -> str:
    if type_code == T_ADDRESS:
        return to_checksum("0x" + word.to_bytes(20, "big").hex())
    if type_code == T_BOOL:
        return "true" if word else "false"
    if type_code == T_BYTES32:
        return "0x" + word.to_bytes(32, "big").hex()
    return f"{word:,}"


def explain(compiled: Dict[str, Any]) -> List[Dict[str, Any]]:
    """One plain-English sentence per clause, keyed by source line, for the owner to read before signing."""
    lines = compiled["lines"]
    expires = datetime.fromtimestamp(compiled["expires_at"], timezone.utc).strftime("%Y-%m-%d %H:%M UTC")
    out = [
        {"line": lines["target_line"], "text": f"This policy controls contract {compiled['target']} on "
                                               f"{compiled['chain']} (chain {compiled['chain_id']}) and nothing else."},
        {"line": lines["owner_line"], "text": f"Only {compiled['owner']}, the contract's owner, can install or "
                                              "revoke it."},
        {"line": lines["callers_line"], "text": "Only these callers may use controlled functions: "
                                                + ", ".join(compiled["callers"]) + "."},
    ]
    for fn in compiled["functions"]:
        out.append({"line": fn["line"], "text": f"{fn['signature']} may be called. Every controlled function not "
                                                "listed is refused."})
        for c in fn["checks"]:
            p = fn["params"][c["arg"]] if c["arg"] != NO_ARG else None
            subject = f"`{p['name']}` in {fn['name']}" if p else fn["name"]
            if c["kind"] == "max":
                text = f"{subject} must be at most {_fmt_word(c['value'], p['type_code'])} per call."
            elif c["kind"] == "min":
                text = f"{subject} must be at least {_fmt_word(c['value'], p['type_code'])} per call."
            elif c["kind"] == "eq":
                text = f"{subject} must equal {_fmt_word(c['value'], p['type_code'])}."
            elif c["kind"] == "in":
                text = f"{subject} must be one of: " + ", ".join(_fmt_word(w, p["type_code"]) for w in c["set"]) + "."
            elif c["kind"] == "rate_sum":
                text = (f"The total of {subject} may not exceed {c['value']:,} in any fixed "
                        f"{_fmt_duration(c['window'])} window.")
            else:
                text = f"{fn['name']} may be called at most {c['value']:,} times in any fixed " \
                       f"{_fmt_duration(c['window'])} window."
            out.append({"line": c["line"], "text": text})
    out.append({"line": lines["expires_line"], "text": f"The policy stops allowing anything at {expires}."})
    return out


def simulate(compiled: Dict[str, Any], calls: List[Dict[str, Any]], *, active: bool = True) -> List[Dict[str, Any]]:
    """Run a sequence of calls through the predictor, carrying rate usage forward like the chain does."""
    state = EnforcerState(active=active)
    results = []
    for call in calls:
        if "calldata" in call:
            calldata = bytes.fromhex(str(call["calldata"]).removeprefix("0x"))
        else:
            calldata = encode_call(compiled, call["fn"], call.get("args", []))
        decision = predict(compiled, state, sender=call["sender"], calldata=calldata,
                           timestamp=int(call["timestamp"]), chain_id=int(call.get("chain_id", compiled["chain_id"])))
        decision["calldata"] = "0x" + calldata.hex()
        if not decision["allowed"]:
            decision["revert_data"] = "0x" if decision.get("abi_decoder_first") else \
                "0x" + refusal_revert_data(decision["clause"], decision["code"]).hex()
        results.append(decision)
    return results


# ---------------------------------------------------------------------------- CLI


def _now_arg(text: Optional[str]) -> Optional[datetime]:
    if not text:
        return None
    return datetime.fromisoformat(text.replace("Z", "+00:00"))


def main(argv: Optional[List[str]] = None) -> int:
    ap = argparse.ArgumentParser(prog="x3ccp", description="Compile and explain X3CCP control policies")
    sub = ap.add_subparsers(dest="cmd", required=True)
    for name in ("compile", "explain"):
        p = sub.add_parser(name)
        p.add_argument("file")
        p.add_argument("--now", help="compile time as ISO-8601 UTC (for reproducible relative expiry)")
    sim = sub.add_parser("simulate")
    sim.add_argument("file")
    sim.add_argument("calls", help="JSON file: list of {sender, fn, args, timestamp}")
    sim.add_argument("--now")
    dec = sub.add_parser("decode-revert")
    dec.add_argument("hex")
    args = ap.parse_args(argv)
    try:
        if args.cmd == "decode-revert":
            print(json.dumps(decode_refusal(bytes.fromhex(args.hex.removeprefix("0x")))))
            return 0
        with open(args.file, encoding="utf-8") as fh:
            compiled = compile_source(fh.read(), now=_now_arg(args.now))
        if args.cmd == "compile":
            print(json.dumps(compiled, indent=2))
        elif args.cmd == "explain":
            for item in explain(compiled):
                print(f"line {item['line']:>3}: {item['text']}")
        else:
            with open(args.calls, encoding="utf-8") as fh:
                print(json.dumps(simulate(compiled, json.load(fh)), indent=2))
        return 0
    except X3CCPError as exc:
        print(json.dumps({"error": exc.to_dict()}), file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
