#!/usr/bin/env python3
"""Parser boundary for the production X3 intent language.

The Python surface intentionally emits a stable JSON AST used by the legacy
planner/runner and by integration tests.  It recognizes the production intent
shape rather than the old path/constraints-only MVP subset:

intent name {
  from Solana.USDC amount 10 receiver <sol-address>
  to Ethereum.USDC receiver <0x-address>
  route { swap ...; bridge ...; lock ...; mint ...; burn ...; release ... }
  require finality Ethereum >= 64
  timeout 30s refund Solana.USDC to sender
  on_fail refund Solana.USDC to sender
}
"""
import argparse
import json
import re
import sys
from dataclasses import dataclass, asdict
from typing import Any, Dict, List, Optional


class X3ParseError(Exception):
    def __init__(self, code: str, message: str, line: Optional[int] = None, field: Optional[str] = None):
        super().__init__(message)
        self.code = code
        self.message = message
        self.line = line
        self.field = field

    def to_dict(self) -> Dict[str, Any]:
        data = {"code": self.code, "message": self.message}
        if self.line is not None:
            data["line"] = self.line
        if self.field:
            data["field"] = self.field
        return data


@dataclass
class SourceLine:
    no: int
    text: str


def _clean_lines(path: str) -> List[SourceLine]:
    lines: List[SourceLine] = []
    with open(path, "r", encoding="utf-8") as f:
        for no, raw in enumerate(f.readlines(), 1):
            stripped = raw.strip()
            if not stripped or stripped.startswith("//") or stripped.startswith("#"):
                continue
            # remove trailing comments outside strings (grammar does not use quoted // in commands)
            stripped = re.sub(r"\s+(//|#).*$", "", stripped).strip()
            if stripped:
                lines.append(SourceLine(no, stripped.rstrip(";")))
    return lines


def _asset_ref(value: str, line: int, field: str) -> Dict[str, str]:
    m = re.fullmatch(r"([A-Za-z][A-Za-z0-9_-]*)\.([A-Za-z][A-Za-z0-9_-]*)", value)
    if not m:
        raise X3ParseError("X3_PARSE_ASSET_REF", f"expected chain.asset reference for {field}", line, field)
    return {"chain": m.group(1).lower(), "asset": m.group(2)}


def _parse_receiver(tokens: List[str], line: int) -> Optional[str]:
    if "receiver" in tokens:
        idx = tokens.index("receiver")
        if idx + 1 >= len(tokens):
            raise X3ParseError("X3_PARSE_RECEIVER", "receiver requires an address", line, "receiver")
        address = tokens[idx + 1].strip('"')
        # Basic validation: Ethereum hex address should start with 0x and be 42 chars (0x + 40 hex)
        if address.startswith("0x"):
            if not re.fullmatch(r"0x[0-9a-fA-F]{40}", address):
                raise X3ParseError("X3_PARSE_RECEIVER", f"invalid Ethereum address '{address}'", line, "receiver")
        # Additional address formats can be added here (e.g., base58 for Solana)
        return address
    return None


def _parse_endpoint(ln: SourceLine, keyword: str) -> Dict[str, Any]:
    parts = ln.text.split()
    if len(parts) < 2 or parts[0] != keyword:
        raise X3ParseError("X3_PARSE_ENDPOINT", f"expected {keyword} chain.asset", ln.no, keyword)
    endpoint = _asset_ref(parts[1], ln.no, keyword)
    if "amount" in parts:
        idx = parts.index("amount")
        if idx + 1 >= len(parts):
            raise X3ParseError("X3_PARSE_AMOUNT", f"{keyword}.amount requires a value", ln.no, f"{keyword}.amount")
        endpoint["amount"] = parts[idx + 1]
    else:
        endpoint["amount"] = None
    receiver = _parse_receiver(parts, ln.no)
    if receiver:
        endpoint["receiver"] = receiver
    return endpoint


def _parse_refund(tokens: List[str], line: int) -> Dict[str, Any]:
    if not tokens or tokens[0] != "refund" or len(tokens) < 4 or tokens[2] != "to":
        raise X3ParseError("X3_PARSE_REFUND", "expected refund <chain.asset> to <receiver>", line, "refund")
    asset = _asset_ref(tokens[1], line, "refund.asset")
    return {"type": "refund", "chain": asset["chain"], "asset": asset["asset"], "to": tokens[3].strip('"')}


def _parse_require(ln: SourceLine) -> Dict[str, Any]:
    tokens = ln.text.split()
    if len(tokens) < 2 or tokens[0] != "require":
        raise X3ParseError("X3_PARSE_REQUIRE", "expected require clause", ln.no, "require")
    kind = tokens[1].lower()
    if kind == "finality" and len(tokens) >= 5:
        return {"kind": "finality", "chain": tokens[2].lower(), "op": tokens[3], "value": tokens[4]}
    if kind == "slippage" and len(tokens) >= 4:
        return {"kind": "slippage", "op": tokens[2], "value": tokens[3]}
    if kind == "profit" and len(tokens) >= 4:
        return {"kind": "profit", "op": tokens[2], "value": " ".join(tokens[3:])}
    if kind == "nonce" and len(tokens) >= 3:
        return {"kind": "nonce", "value": " ".join(tokens[2:])}
    if kind == "proof" and len(tokens) >= 3:
        return {"kind": "proof", "value": " ".join(tokens[2:])}
    if kind == "bridge_liquidity" and len(tokens) >= 4:
        return {"kind": "bridge_liquidity", "op": tokens[2], "value": " ".join(tokens[3:])}
    if kind in {"canonical_supply", "invariant"} and len(tokens) >= 3:
        return {"kind": kind, "value": " ".join(tokens[2:])}
    raise X3ParseError("X3_PARSE_REQUIRE", f"malformed require {kind!r}", ln.no, "require")


def _parse_route_step(ln: SourceLine) -> Dict[str, Any]:
    parts = ln.text.split()
    if not parts:
        raise X3ParseError("X3_PARSE_ROUTE", "empty route operation", ln.no, "route")
    op = parts[0].lower()
    if op == "swap":
        # swap Raydium Solana.USDC -> Solana.SOL amount 10 min_output 0.09
        if len(parts) < 5 or parts[3] != "->":
            raise X3ParseError("X3_PARSE_SWAP", "expected swap <dex> <from> -> <to>", ln.no, "route.swap")
        step: Dict[str, Any] = {"type": "swap", "dex": parts[1].lower(), "from_ref": _asset_ref(parts[2], ln.no, "swap.from"), "to_ref": _asset_ref(parts[4], ln.no, "swap.to")}
        step["from"] = step["from_ref"]["asset"]
        step["to"] = step["to_ref"]["asset"]
        if "amount" in parts:
            step["amount"] = parts[parts.index("amount") + 1]
        if "min_output" in parts:
            step["min_output"] = parts[parts.index("min_output") + 1]
        return step
    if op == "bridge":
        # bridge X3 Solana.SOL -> Ethereum.WSOL receiver 0x...
        if len(parts) < 5 or parts[3] != "->":
            raise X3ParseError("X3_PARSE_BRIDGE", "expected bridge <via> <from> -> <to>", ln.no, "route.bridge")
        source = _asset_ref(parts[2], ln.no, "bridge.from")
        dest = _asset_ref(parts[4], ln.no, "bridge.to")
        step = {"type": "bridge", "via": parts[1].lower(), "from_ref": source, "to_ref": dest, "asset": source["asset"]}
        receiver = _parse_receiver(parts, ln.no)
        if receiver:
            step["receiver"] = receiver
        return step
    if op in {"lock", "mint", "burn", "release"}:
        if len(parts) < 2:
            raise X3ParseError("X3_PARSE_OPERATION", f"{op} requires chain.asset", ln.no, f"route.{op}")
        step = {"type": op, **_asset_ref(parts[1], ln.no, f"{op}.asset")}
        if "amount" in parts:
            step["amount"] = parts[parts.index("amount") + 1]
        for key in ("from", "to"):
            if key in parts:
                step[key] = parts[parts.index(key) + 1].strip('"')
        return step
    raise X3ParseError("X3_PARSE_OPERATION", f"unsupported route operation {op!r}", ln.no, "route")


def parse_file(path):
    lines = _clean_lines(path)
    if not lines:
        raise X3ParseError("X3_PARSE_EMPTY", "input file is empty")
    first = lines[0].text
    m = re.match(r"intent\s+([A-Za-z_][A-Za-z0-9_-]*)\s*\{?", first)
    if not m:
        raise X3ParseError("X3_PARSE_INTENT", "expected intent <name> {", lines[0].no, "intent")
    result: Dict[str, Any] = {"intent": m.group(1), "from": {}, "to": {}, "route": [], "path": [], "requires": [], "constraints": {}, "policies": {}}

    i = 1
    while i < len(lines):
        ln = lines[i]
        text = ln.text
        if text == "}":
            i += 1
            continue
        if text.startswith("from "):
            result["from"] = _parse_endpoint(ln, "from")
        elif text.startswith("to "):
            result["to"] = _parse_endpoint(ln, "to")
        elif (text.startswith("route") or text.startswith("path")) and "{" in text:
            i += 1
            while i < len(lines) and lines[i].text != "}":
                step = _parse_route_step(lines[i])
                result["route"].append(step)
                result["path"].append(step)  # compatibility with planner/schema
                i += 1
        elif text.startswith("require "):
            req = _parse_require(ln)
            result["requires"].append(req)
            # keep common MVP constraints populated for existing planner/simulator
            if req["kind"] == "slippage":
                result["constraints"]["max_slippage"] = req["value"]
            if req["kind"] == "profit":
                result["constraints"]["min_profit"] = req["value"]
        elif text.startswith("constraints") and "{" in text:
            i += 1
            while i < len(lines) and lines[i].text != "}":
                c = lines[i].text
                if c.startswith("min_profit"):
                    result["constraints"]["min_profit"] = c.replace("min_profit", "", 1).strip()
                elif c.startswith("max_slippage"):
                    result["constraints"]["max_slippage"] = c.replace("max_slippage", "", 1).strip()
                elif c.startswith("timeout"):
                    result["constraints"]["timeout"] = c.replace("timeout", "", 1).strip()
                elif c.startswith("atomic"):
                    result["constraints"]["atomic"] = c.split()[-1].lower() == "true"
                i += 1
        elif text.startswith("timeout "):
            parts = text.split()
            if len(parts) < 2:
                raise X3ParseError("X3_PARSE_TIMEOUT", "timeout requires a duration", ln.no, "timeout")
            timeout = {"duration": parts[1]}
            if len(parts) > 2:
                timeout["action"] = _parse_refund(parts[2:], ln.no)
            result["constraints"]["timeout"] = parts[1]
            result["policies"]["timeout"] = timeout
        elif text.startswith("on_fail "):
            parts = text.split()[1:]
            if parts and parts[0] == "rollback":
                result["policies"]["on_fail"] = {"type": "rollback"}
            elif parts and parts[0] == "halt":
                result["policies"]["on_fail"] = {"type": "halt"}
            elif parts and parts[0] == "quarantine":
                result["policies"]["on_fail"] = {"type": "quarantine"}
            else:
                result["policies"]["on_fail"] = _parse_refund(parts, ln.no)
        i += 1

    return result


def main():
    p = argparse.ArgumentParser(description='X3 production intent parser -> stable JSON')
    p.add_argument('input', help='input .x3 intent file')
    p.add_argument('-o', '--output', help='output json file (stdout if omitted)')
    args = p.parse_args()
    try:
        out = parse_file(args.input)
        dumped = json.dumps(out, indent=2)
    except X3ParseError as exc:
        dumped = json.dumps({"status": "error", "errors": [exc.to_dict()]}, indent=2)
        print(dumped, file=sys.stderr)
        sys.exit(1)
    if args.output:
        with open(args.output, 'w', encoding='utf-8') as f:
            f.write(dumped)
    else:
        print(dumped)


if __name__ == '__main__':
    main()
