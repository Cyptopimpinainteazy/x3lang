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

**The guard kinds are the compiler's, not a subset of them.** `registry.py::REQUIRE_KINDS`
holds the list and `_parse_require` reads any name in it the same way (a comparison is
`<kind> <op> <value>`, a bare guard is `<kind> <value>`), so a kind this surface does not
*model* is carried rather than refused — `runner.rust_intent_envelope` passes `requires` to
the compiler verbatim. It used to know nine of the eighteen and refuse the rest as
`malformed require`, which refused `require route_score >= 90` — written by three shipped
examples the compiler accepts (TICKET-091). A word the compiler does not know either is
refused by name, with the vocabulary listed.

**Addresses are validated for shape, and only for shape.** Any `0x`-prefixed hex is
accepted, because the language validates none and the repository's own examples write short
placeholders (`0xA1`, `0x1`); what is refused is a string that is not an address shape at
all. Requiring exactly forty hex characters here was drift rather than scope — this surface
refusing files the compiler accepts (TICKET-091).

**Nothing here may raise anything but `X3ParseError`.** A parser with no error surface is
the one part of this boundary that is not a scope decision: callers get a code, a message
and a line, or they get a result. `tests/test_surface_drift.py` asserts it over every
example in the repository.
"""
import argparse
import importlib.util
import json
import os
import re
import sys
from dataclasses import dataclass, asdict
from typing import Any, Dict, List, Optional


def _registry():
    """`registry.py`, loaded by path: this module is not a package."""
    path = os.path.join(os.path.dirname(os.path.abspath(__file__)), "registry.py")
    spec = importlib.util.spec_from_file_location("registry", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


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
    with open(path, "r", encoding="utf-8") as source:
        return _clean_source_lines(source.read())


def _clean_source_lines(source: str) -> List[SourceLine]:
    lines: List[SourceLine] = []
    for no, raw in enumerate(source.split("\n"), 1):
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
        # **Any `0x`-prefixed hex**, not exactly forty digits. The compiler validates no address
        # shape at all, so requiring forty here made this surface refuse files the language
        # accepts: `examples/arb_scope.x3` writes `receiver 0xA1` and `examples/intent_fusion.x3`
        # writes `0x1`, and the compiler checks and builds both (TICKET-091). What is still
        # refused is a string that is not an address shape at all — `not-an-evm-address` has no
        # `0x` and no hex in it — which is the typo this check is for.
        if address.startswith("0x") and not re.fullmatch(r"0x[0-9a-fA-F]+", address):
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


def _parse_app_fee(ln: SourceLine) -> Dict[str, Any]:
    """`fee <rate> to <account>`: the integrator fee this program pays on its route.

    The rate keeps its unit (`25 bps`, `0.25%`) so the adapter that settles the route decides,
    with the same bound parser it uses for `require fees`, whether it is a whole number of
    basis points. Only the shape is read here; a surface that cannot pay the fee refuses it.
    """
    words = ln.text.rstrip(";").split()
    if "to" not in words[1:]:
        raise X3ParseError("X3_PARSE_FEE", "expected `fee <rate> to <account>`, e.g. `fee 25 bps to fees.app.near`",
                           ln.no, "fee")
    split = words.index("to", 1)
    rate = " ".join(words[1:split])
    recipient = words[split + 1:]
    if not rate or len(recipient) != 1:
        raise X3ParseError("X3_PARSE_FEE", "expected `fee <rate> to <account>`, e.g. `fee 25 bps to fees.app.near`",
                           ln.no, "fee")
    return {"value": rate, "recipient": recipient[0].strip('"'), "line": ln.no}


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
        # The whole bound: `require slippage <= 50 bps` is four words after `require`, and
        # keeping only the first one turned it into a bare `50` (issue #241).
        return {"kind": "slippage", "op": tokens[2], "value": " ".join(tokens[3:])}
    if kind == "profit" and len(tokens) >= 4:
        return {"kind": "profit", "op": tokens[2], "value": " ".join(tokens[3:])}
    if kind in ("min_net_bps", "min_profit_bps") and len(tokens) >= 4:
        return {"kind": kind, "op": tokens[2], "value": " ".join(tokens[3:])}
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
    # Every *other* name in the compiler's vocabulary is read the same way: a guard that states
    # a comparison is `<kind> <op> <value>`, and one that does not is `<kind> <value>`. One rule
    # for the list rather than one branch per name — the branches are what drifted. This surface
    # knew nine of the compiler's eighteen kinds and refused the rest as `malformed require`,
    # which is the one thing an unknown *guard* is not: the compiler would have accepted it, and
    # `require route_score >= 90` is written by three shipped examples (TICKET-091). A word the
    # compiler does not know either is still refused, by name.
    if kind in _registry().REQUIRE_KINDS:
        if len(tokens) >= 4:
            return {"kind": kind, "op": tokens[2], "value": " ".join(tokens[3:])}
        if len(tokens) >= 3:
            return {"kind": kind, "value": " ".join(tokens[2:])}
    raise X3ParseError(
        "X3_PARSE_REQUIRE",
        f"malformed require {kind!r}: it is not one of the guard kinds the language defines "
        f"({', '.join(sorted(_registry().REQUIRE_KINDS))})",
        ln.no,
        "require",
    )


def _parse_fallback(lines: List[SourceLine], i: int) -> tuple:
    """A `fallback { … }` block: the venues a route may substitute, and its own guards.

    The compiler carries the approved list in `Operation::RouteFallback`, because a runtime
    can only restrict itself to the compiler's approvals if the approvals are in the
    artifact — so the list travels here too, as a route step. Each `replace with <venue>` is
    one approval, and a `min_output` beside it is the floor that substitution must clear.

    Read as a block rather than a line because that is the grammar; this surface refused
    `examples/route_fallback.x3` as `unsupported route operation 'fallback'` while the
    compiler checks and builds it (TICKET-091).
    """
    if not lines[i].text.startswith("fallback"):
        raise X3ParseError("X3_PARSE_FALLBACK", "expected a fallback block", lines[i].no, "route.fallback")
    i += 1
    replacements: List[Dict[str, Any]] = []
    guards: List[Dict[str, Any]] = []
    while i < len(lines) and lines[i].text != "}":
        text = lines[i].text
        if text.startswith("replace "):
            tokens = text.split()
            # `replace with <venue> [min_output <n>]`
            if len(tokens) < 3 or tokens[1] != "with":
                raise X3ParseError(
                    "X3_PARSE_FALLBACK",
                    f"expected `replace with <venue>` [min_output <n>], got {text!r}",
                    lines[i].no,
                    "route.fallback.replace",
                )
            entry: Dict[str, Any] = {"venue": tokens[2].lower()}
            if "min_output" in tokens:
                entry["min_output"] = tokens[tokens.index("min_output") + 1]
            replacements.append(entry)
        elif text.startswith("require "):
            # The substitution's *own* guards, which the compiler reads as the fallback's
            # conditions rather than as the route's — so they stay in this step and are not
            # added to the intent's `requires`.
            guards.append(_parse_require(lines[i]))
        else:
            raise X3ParseError(
                "X3_PARSE_FALLBACK",
                f"unsupported fallback clause {text!r}: the block takes `replace with <venue>` and "
                "`require <guard>` only",
                lines[i].no,
                "route.fallback",
            )
        i += 1
    if not replacements:
        raise X3ParseError(
            "X3_PARSE_FALLBACK",
            "a fallback block with no `replace with <venue>` approves nothing, which is not a "
            "fallback",
            lines[i].no if i < len(lines) else 0,
            "route.fallback",
        )
    return {"type": "fallback", "replacements": replacements, "requires": guards}, i + 1


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
        # `amount` and `min_receive` are the compiler's bridge-step fields
        # (`parser.rs::parse_bridge_step`). This surface dropped both, so a floor written on a
        # bridge leg never reached anything that could enforce it; `near_intents.py` checks
        # a solver's guaranteed output against `min_receive`.
        for key in ("amount", "min_receive"):
            if key in parts:
                idx = parts.index(key)
                if idx + 1 >= len(parts):
                    raise X3ParseError("X3_PARSE_BRIDGE", f"bridge {key} requires a value", ln.no, f"route.bridge.{key}")
                step[key] = parts[idx + 1]
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
    body — rather than line by line, so a `risk_policy` whose body happens to
    contain the word `intent` does not start an intent early.
    """
    i = 0
    while i < len(lines):
        text = lines[i].text
        if text.startswith("intent "):
            return i
        # Skip a top-level declaration block.
        depth = text.count("{") - text.count("}")
        i += 1
        while i < len(lines) and depth > 0:
            depth += lines[i].text.count("{") - lines[i].text.count("}")
            i += 1
    raise X3ParseError("X3_PARSE_NO_INTENT", "no `intent` declaration found", None, "intent")


def parse_intent(path: str) -> Dict[str, Any]:
    lines = _clean_lines(path)
    if not lines:
        raise X3ParseError("X3_PARSE_EMPTY", "empty intent file")
    start = _intent_line_index(lines)
    head = re.fullmatch(r"intent\s+([A-Za-z_][A-Za-z0-9_-]*)\s*\{", lines[start].text)
    if not head:
        raise X3ParseError("X3_PARSE_INTENT", "expected `intent <name> {`", lines[start].no, "intent")
    intent: Dict[str, Any] = {
        "name": head.group(1),
        "from": None,
        "to": None,
        "route": [],
        "requires": [],
        "timeout": None,
        "on_fail": None,
        "fee": None,
        "under_mandate": None,
    }
    i = start + 1
    while i < len(lines):
        ln = lines[i]
        text = ln.text
        if text == "}":
            break
        if text.startswith("from "):
            intent["from"] = _parse_endpoint(ln, "from")
        elif text.startswith("to "):
            intent["to"] = _parse_endpoint(ln, "to")
        elif text.startswith("route "):
            i += 1
            while i < len(lines) and lines[i].text != "}":
                if lines[i].text.startswith("fallback"):
                    step, i = _parse_fallback(lines, i)
                    intent["route"].append(step)
                    continue
                intent["route"].append(_parse_route_step(lines[i]))
                i += 1
        elif text.startswith("require "):
            intent["requires"].append(_parse_require(ln))
        elif text.startswith("timeout "):
            # timeout 30s refund Solana.USDC to sender
            parts = text.split()
            if len(parts) < 2:
                raise X3ParseError("X3_PARSE_TIMEOUT", "expected timeout <duration>", ln.no, "timeout")
            intent["timeout"] = {"duration": parts[1], "refund": _parse_refund(parts[2:], ln.no) if len(parts) > 2 else None}
        elif text.startswith("on_fail "):
            intent["on_fail"] = _parse_refund(text.split()[1:], ln.no)
        elif text.startswith("fee "):
            if intent["fee"] is not None:
                raise X3ParseError("X3_PARSE_FEE", "an intent states one `fee`; a second fee is ambiguous", ln.no, "fee")
            intent["fee"] = _parse_app_fee(ln)
        elif text.startswith("under mandate "):
            parts = text.split()
            if len(parts) != 3:
                raise X3ParseError("X3_PARSE_MANDATE", "expected `under mandate <owner>`", ln.no, "under_mandate")
            intent["under_mandate"] = parts[2]
        else:
            raise X3ParseError("X3_PARSE_CLAUSE", f"unsupported clause {text!r}", ln.no, "intent")
        i += 1
    if intent["from"] is None or intent["to"] is None:
        raise X3ParseError("X3_PARSE_ENDPOINT", "an intent must state `from` and `to`", None, "intent")
    return intent


def main(argv: Optional[List[str]] = None) -> int:
    parser = argparse.ArgumentParser(description="Parse an X3 intent file to JSON")
    parser.add_argument("path", help="path to a .x3 intent")
    args = parser.parse_args(argv)
    try:
        print(json.dumps(parse_intent(args.path), indent=2))
        return 0
    except X3ParseError as exc:
        print(json.dumps(exc.to_dict(), indent=2))
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
