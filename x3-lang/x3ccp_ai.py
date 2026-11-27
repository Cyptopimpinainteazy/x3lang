"""X3CCP for AI agents: grammar-constrained generation, model-readable diagnostics, a language card.

The rule this module keeps: a model may WRITE a `control` policy, but nothing a model writes can
authorize anything. The compiler checks it, `explain()` turns it back into sentences the owner
reads, `simulate()` shows what the chain would do, and only the owner's own `installPolicy`
transaction gives it force. Nothing here signs, sends or installs.

- `GRAMMAR` is x3ccp.gbnf, for llama.cpp-compatible constrained decoding (`--grammar-file`).
- `grammar_accepts()` is a small exact GBNF recognizer used to test that grammar against every
  committed source, so the grammar a model samples from cannot drift from the real language.
- `diagnose()` returns the compiler's verdict as JSON a model can repair from: a stable code, the
  line, the field and a concrete hint.
- `LANGUAGE_CARD` is the reference to put in a model's context.
"""

from __future__ import annotations

import os
import re
import sys
from datetime import datetime
from functools import lru_cache
from typing import Any, Dict, List, Optional, Tuple

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import x3ccp  # noqa: E402

GRAMMAR_PATH = os.path.join(os.path.dirname(os.path.abspath(__file__)), "x3ccp.gbnf")
with open(GRAMMAR_PATH, encoding="utf-8") as _fh:
    GRAMMAR = _fh.read()

LANGUAGE_CARD = """X3CCP `control` — an X3Lang policy that a Solidity contract enforces on chain.

One block per file. Every clause is one line. `//` starts a comment.

control <name> {
    target <base-sepolia|local> <contract address>      // the X3Controlled contract, nothing else
    owner <address>                                     // must equal the contract's owner
    allow callers <address>, <address>                  // who may call controlled functions
    allow fn <name>(<type> <param>, ...)                // types: address bool bytes32 uint8..uint256
    limit <fn>.<param> <= <uint>                        // per-call maximum (uint only)
    limit <fn>.<param> >= <uint>                        // per-call minimum (uint only)
    limit <fn>.<param> == <value>                       // exact value (any type)
    limit <fn>.<param> in <value>, <value>              // allowlist, 1..16 values
    rate <fn>.<param> per <duration> <= <uint>          // sum per fixed window (uint only)
    rate <fn> calls per <duration> <= <uint>            // calls per fixed window
    expires <duration> | expires at <YYYY-MM-DDTHH:MM:SSZ>
}

Durations: 30s, 10m, 1h, 7d (rate windows 1s..365d, fixed and epoch-aligned, not rolling).
Numbers are base units: 50 USDC at 6 decimals is 50_000_000. Underscores are allowed.
Every controlled function not listed in `allow fn` is refused. With no policy, everything is refused.
A refusal reverts with X3Refused(<line of the clause>, <reason code>).

Rules for writing policies:
- Never invent an address. Use only addresses the owner gave you; checksum or all-lowercase.
- Mainnets are refused in v1. Use base-sepolia or local.
- State the tightest bounds the owner asked for; do not add permissions they did not ask for.
- After compiling, show the owner explain() output. The owner signs, not you.
"""


# ---------------------------------------------------------------------------- GBNF recognizer


class GrammarError(ValueError):
    pass


def _parse_gbnf(text: str) -> Dict[str, Any]:
    rules: Dict[str, Any] = {}
    body = "\n".join(re.sub(r"(^|\s)#.*$", r"\1", line) for line in text.split("\n"))
    # A rule starts at `name ::=`; continuation lines are indented.
    chunks = re.split(r"\n(?=[A-Za-z][A-Za-z0-9_-]*\s*::=)", "\n" + body)
    for chunk in chunks:
        chunk = chunk.strip()
        if not chunk:
            continue
        m = re.match(r"([A-Za-z][A-Za-z0-9_-]*)\s*::=(.*)", chunk, re.S)
        if not m:
            raise GrammarError(f"bad rule: {chunk[:40]!r}")
        toks = _tokenize(m.group(2))
        node, pos = _alt(toks, 0)
        if pos != len(toks):
            raise GrammarError(f"rule {m.group(1)}: unexpected {toks[pos]!r}")
        rules[m.group(1)] = node
    return rules


def _tokenize(src: str) -> List[Tuple[str, Any]]:
    toks: List[Tuple[str, Any]] = []
    i = 0
    while i < len(src):
        c = src[i]
        if c.isspace():
            i += 1
        elif c == '"':
            j, out = i + 1, []
            while src[j] != '"':
                if src[j] == "\\":
                    out.append({"n": "\n", "r": "\r", "t": "\t"}.get(src[j + 1], src[j + 1]))
                    j += 2
                else:
                    out.append(src[j])
                    j += 1
            toks.append(("lit", "".join(out)))
            i = j + 1
        elif c == "[":
            j = i + 1
            negate = src[j] == "^"
            if negate:
                j += 1
            items: List[Tuple[str, str]] = []
            while src[j] != "]":
                a, j = _class_char(src, j)
                if src[j] == "-" and src[j + 1] != "]":
                    b, j = _class_char(src, j + 1)
                    items.append((a, b))
                else:
                    items.append((a, a))
            toks.append(("cls", (negate, tuple(items))))
            i = j + 1
        elif c == "{":
            j = src.index("}", i)
            parts = src[i + 1:j].split(",")
            lo = int(parts[0])
            hi = lo if len(parts) == 1 else (int(parts[1]) if parts[1].strip() else None)
            toks.append(("rep", (lo, hi)))
            i = j + 1
        elif c in "()|*+?":
            toks.append((c, c))
            i += 1
        else:
            m = re.match(r"[A-Za-z][A-Za-z0-9_-]*", src[i:])
            if not m:
                raise GrammarError(f"unexpected character {c!r}")
            toks.append(("ref", m.group(0)))
            i += len(m.group(0))
    return toks


def _class_char(src: str, j: int) -> Tuple[str, int]:
    if src[j] == "\\":
        return {"n": "\n", "r": "\r", "t": "\t"}.get(src[j + 1], src[j + 1]), j + 2
    return src[j], j + 1


def _alt(toks, pos):
    options = []
    seq, pos = _seq(toks, pos)
    options.append(seq)
    while pos < len(toks) and toks[pos][0] == "|":
        seq, pos = _seq(toks, pos + 1)
        options.append(seq)
    return ("alt", tuple(options)), pos


def _seq(toks, pos):
    items = []
    while pos < len(toks) and toks[pos][0] not in ("|", ")"):
        kind, val = toks[pos]
        if kind == "(":
            node, pos = _alt(toks, pos + 1)
            if pos >= len(toks) or toks[pos][0] != ")":
                raise GrammarError("unclosed group")
            pos += 1
        elif kind in ("lit", "cls", "ref"):
            node, pos = (kind, val), pos + 1
        else:
            raise GrammarError(f"unexpected {val!r}")
        while pos < len(toks) and toks[pos][0] in ("*", "+", "?", "rep"):
            k, v = toks[pos]
            lo, hi = {"*": (0, None), "+": (1, None), "?": (0, 1)}.get(k, v if k == "rep" else (0, None))
            node = ("repeat", node, lo, hi)
            pos += 1
        items.append(node)
    return ("seq", tuple(items)), pos


@lru_cache(maxsize=4)
def _compiled_rules(text: str) -> Dict[str, Any]:
    return _parse_gbnf(text)


def grammar_accepts(source: str, grammar: str = GRAMMAR, start: str = "root") -> bool:
    """True if `source` is in the language of the GBNF grammar (exact, full-string match)."""
    rules = _compiled_rules(grammar)
    memo: Dict[Tuple[int, int], frozenset] = {}

    def match(node, pos: int) -> frozenset:
        kind = node[0]
        if kind == "lit":
            return frozenset([pos + len(node[1])]) if source.startswith(node[1], pos) else frozenset()
        if kind == "cls":
            if pos >= len(source):
                return frozenset()
            negate, items = node[1]
            hit = any(a <= source[pos] <= b for a, b in items)
            return frozenset([pos + 1]) if hit != negate else frozenset()
        if kind == "ref":
            key = (id(rules[node[1]]), pos)
            if key not in memo:
                memo[key] = frozenset()  # no left recursion in this grammar; guard anyway
                memo[key] = match(rules[node[1]], pos)
            return memo[key]
        if kind == "alt":
            out = set()
            for option in node[1]:
                out |= match(option, pos)
            return frozenset(out)
        if kind == "seq":
            current = {pos}
            for item in node[1]:
                nxt = set()
                for p in current:
                    nxt |= match(item, p)
                current = nxt
                if not current:
                    break
            return frozenset(current)
        if kind == "repeat":
            _, inner, lo, hi = node
            results = set()
            frontier = {pos}
            count = 0
            seen = set()
            if lo == 0:
                results |= frontier
            while frontier and (hi is None or count < hi):
                nxt = set()
                for p in frontier:
                    nxt |= match(inner, p)
                count += 1
                if hi is None and count > lo:
                    nxt -= seen  # unbounded repeat: stop once no new positions appear
                seen |= nxt
                if count >= lo:
                    results |= nxt
                frontier = nxt
            return frozenset(results)
        raise GrammarError(f"unknown node {kind}")

    return len(source) in match(rules[start], 0)


# ---------------------------------------------------------------------------- diagnostics


def diagnose(source: str, *, now: Optional[datetime] = None) -> Dict[str, Any]:
    """Compile and report in a shape a model can repair from. Never raises on bad input."""
    if len(source.encode("utf-8")) > x3ccp.MAX_SOURCE_BYTES:
        return {"ok": False, "grammar_ok": False, "warnings": [],
                "errors": [{"code": "X3CCP_LIMIT", "message": f"source larger than {x3ccp.MAX_SOURCE_BYTES} bytes",
                            "hint": "one control block is a few dozen lines"}]}
    syntax_ok = grammar_accepts(source if source.endswith("\n") else source + "\n")
    try:
        compiled = x3ccp.compile_source(source, now=now)
    except x3ccp.X3CCPError as exc:
        return {"ok": False, "grammar_ok": syntax_ok, "errors": [exc.to_dict()], "warnings": []}
    return {
        "ok": True,
        "grammar_ok": syntax_ok,
        "errors": [],
        "warnings": compiled["warnings"],
        "policy_hash": compiled["policy_hash"],
        "policy_hex": compiled["policy_hex"],
        "explain": x3ccp.explain(compiled),
        "next_step": "Show the explanation to the contract owner. Only the owner's own "
                     "installPolicy(bytes) transaction gives this policy force.",
    }
