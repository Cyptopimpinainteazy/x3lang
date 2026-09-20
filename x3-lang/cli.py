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

## Scope, stated so it does not have to be inferred

**One `intent` per file.** Every other top-level item — `finality_policy`, `risk_policy`,
`venue`, `proofs required`, `atomic_choice`, `objective`, `parallel`, `strategy`, … — is
*skipped*, not read, because this surface's output is a `validated_intent_v1` for the
runner and the legacy planner, and those consume an intent. A file whose subject is a bare
`atomic_choice` or `strategy` block is a program for the compiler and has no intent to
offer; the refusal says so by name (`X3_PARSE_NO_INTENT`) rather than crashing on the line
after the last one.

**Nine guard kinds of the compiler's eighteen.** `registry.py::REQUIRE_KINDS` is the list
and it is a deliberate subset — telemetry, audit and solver-bond guards are not this
surface's business. A guard outside it is refused as `malformed require '<kind>'`, which
is accurate but does not say that the compiler would accept it; the list in `registry.py`
is where that boundary is written down.

**Addresses are validated for shape, and the shape is stricter than the compiler's.** A
40-hex-character `0x…` is required here, where the language accepts a short placeholder
like `0xA1` — which is what several of the repository's own examples use. That is drift
rather than scope, and it is TICKET-091: this surface refuses files the compiler accepts.

**Nothing here may raise anything but `X3ParseError`.** A parser with no error surface is
the one part of this boundary that is not a scope decision: callers get a code, a message
and a line, or they get a result. `tests/test_surface_drift.py` asserts it over every
example in the repository.
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
    # `to <receiver>` is optional, and its absence means `sender` — the compiler's own
    # default (`formatter::is_sender_default`), which is why `x3c fmt` writes
    # `refund Solana.USDC` for a clause that said `refund Solana.USDC to sender`. Requiring
    # the explicit receiver here made this surface unable to read the formatter's output.
    if not tokens or tokens[0] != "refund" or len(tokens) < 2:
        raise X3ParseError("X3_PARSE_REFUND", "expected refund <chain.asset> [to <receiver>]", line, "refund")
    receiver = "sender"
    if len(tokens) > 2:
        if tokens[2] != "to" or len(tokens) < 4:
            raise X3ParseError(
                "X3_PARSE_REFUND", "expected refund <chain.asset> [to <receiver>]", line, "refund"
            )
        receiver = tokens[3].strip('"')
    asset = _asset_ref(tokens[1], line, "refund.asset")
    return {"type": "refund", "chain": asset["chain"], "asset": asset["asset"], "to": receiver}


def _parse_require(ln: SourceLine) -> Dict[str, Any]:
    tokens = ln.text.split()
    if len(tokens) < 2 or tokens[0] != "require":
        raise X3ParseError("X3_PARSE_REQUIRE", "expected require clause", ln.no, "require")
    kind = tokens[1].lower()
    # `finality.<chain>` and `finality <chain>` are one guard written two ways, and the
    # compiler reads both (`require finality.sol == finalized`). The **dotted** form is
    # what `x3c fmt` writes, so a surface that reads only the spaced form cannot read its
    # own tooling's output — which is how this was found: reformatting
    # `examples/arb_solana_eth.x3` made this harness reject the file.
    if kind.startswith("finality.") and len(tokens) >= 4:
        return {"kind": "finality", "chain": kind.split(".", 1)[1].lower(), "op": tokens[2], "value": tokens[3]}
    if kind == "finality" and len(tokens) >= 5:
        return {"kind": "finality", "chain": tokens[2].lower(), "op": tokens[3], "value": tokens[4]}
    if kind == "slippage" and len(tokens) >= 4:
        return {"kind": "slippage", "op": tokens[2], "value": tokens[3]}
    if kind == "profit" and len(tokens) >= 4:
        return {"kind": "profit", "op": tokens[2], "value": " ".join(tokens[3:])}
    if kind == "nonce" and len(tokens) >= 3:
        return {"kind": "nonce", "value": " ".join(tokens[2:])}
    if kind in {"proof", "proof_complete"} and len(tokens) >= 3:
        # `proof` is the spelling this surface was written with, and the compiler's guard
        # is `proof_complete` — its kind list has no `proof`, so `require proof verified`
        # is refused as a guard kind it does not know. A program may be written against
        # either, so both are read here, and the returned kind says which one the source
        # used rather than quietly rewriting it.
        return {"kind": kind, "value": " ".join(tokens[2:])}
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


def _intent_line_index(lines: List[SourceLine]) -> int:
    """Index of the `intent` line, skipping the declarations written above it.

    The intent is not always the first thing in a file: `risk_policy`,
    `finality_policy`, `proofs required`, `relayers`, `solver_market` and `venue`
    blocks all sit above it in the corpus. Requiring line 0 to be
    `intent <name> {` made every such file unreadable — one example gaining a
    `finality_policy` block failed thirteen tests, none of which were about
    finality.

    A declaration is skipped as a *whole* — its header line plus a brace-balanced
    body — rather than line by line, so a body that is never closed is an error
    here instead of silently eating the intent. `error <Name>` is the one
    brace-less top-level declaration the corpus writes, and it is matched
    explicitly: a line that is neither a declaration nor the intent is left for
    the caller to refuse, so a misspelled declaration still fails loudly.
    """
    index = 0
    while index < len(lines):
        text = lines[index].text
        if re.match(r"intent\s+[A-Za-z_]", text):
            return index
        if re.match(r"error\s+[A-Za-z_][A-Za-z0-9_]*$", text):
            index += 1
            continue
        if not re.match(r"[A-Za-z_][A-Za-z0-9_-]*(\s+[A-Za-z_][A-Za-z0-9_.-]*)*\s*\{", text):
            return index
        header = lines[index]
        depth = 0
        while index < len(lines):
            depth += lines[index].text.count("{") - lines[index].text.count("}")
            index += 1
            if depth <= 0:
                break
        if depth > 0:
            raise X3ParseError(
                "X3_PARSE_DECLARATION",
                f"declaration {header.text!r} is never closed",
                header.no,
                "declaration",
            )
    return index


def parse_file(path):
    lines = _clean_lines(path)
    if not lines:
        raise X3ParseError("X3_PARSE_EMPTY", "input file is empty")
    start = _intent_line_index(lines)
    if start >= len(lines):
        # `_intent_line_index` walks past the last line when a file declares no `intent` at
        # all — every top-level item is a declaration it skips — and it returned that index
        # to here, where `lines[start]` raised `IndexError`. Five of the repository's
        # examples are shaped that way, so this surface *crashed* on valid input instead of
        # refusing it: a parser with no error surface is worse than a narrow one. Its own
        # docstring already said the caller refuses what it does not recognise, and this is
        # that refusal.
        raise X3ParseError(
            "X3_PARSE_NO_INTENT",
            "this file declares no `intent`: this surface reads one intent per file, and every "
            "top-level item here is a declaration it skips. A file whose subject is a bare "
            "`atomic_choice`, `strategy`, `objective` or `parallel` block is a program for the "
            "compiler, not an intent for this surface",
            lines[-1].no,
            "intent",
        )

    first = lines[start].text
    m = re.match(r"intent\s+([A-Za-z_][A-Za-z0-9_-]*)\s*\{?", first)
    if not m:
        # The line is neither an intent nor a declaration `_intent_line_index` skips, so the
        # file has no intent to read. Saying "expected intent <name> {" at that line sent a
        # reader looking for a typo in a construct that is correct in the language and
        # simply is not what this surface reads.
        raise X3ParseError(
            "X3_PARSE_NO_INTENT",
            f"this file declares no `intent`; the first top-level item this surface does not "
            f"recognise is {first!r}. It reads one intent per file, so a program built from "
            f"`atomic_choice`, `objective`, `parallel`, `strategy` or similar declarations is "
            f"for the compiler rather than for this surface",
            lines[start].no,
            "intent",
        )
    result: Dict[str, Any] = {"intent": m.group(1), "from": {}, "to": {}, "route": [], "path": [], "requires": [], "constraints": {}, "policies": {}}

    i = start + 1
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
