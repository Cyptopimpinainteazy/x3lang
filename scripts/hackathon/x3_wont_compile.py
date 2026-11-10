#!/usr/bin/env python3
"""Show which unsafe agent payments X3 refuses to compile, before any check or chain call.

    python3 scripts/hackathon/x3_wont_compile.py
    python3 scripts/hackathon/x3_wont_compile.py --json

Starts from the real agent payment program (`x3-lang/examples/near_intents_agent_payment.x3`),
confirms it compiles, then applies one edit at a time (drop the slippage bound, drop the
timeout, add a second leg, use a look-alike account, ...) and compiles each edited copy with
the same NEAR Intents compiler the agent path uses. Every edit must be refused with its own
error code. A program that does not compile never reaches the mandate check, a quote request
or Guard, so an agent cannot even express these payments.

Offline: no tokens list, RPC, 1Click call, key or chain state is used. Edited copies are
written to a temporary directory; the example program is never modified.
"""
from __future__ import annotations

import argparse
import importlib.util
import json
import sys
import tempfile
from pathlib import Path
from typing import Any, Dict, List, Optional

ROOT = Path(__file__).resolve().parents[2]
X3 = ROOT / "x3-lang"
PROGRAM = X3 / "examples" / "near_intents_agent_payment.x3"


def _load_compiler():
    sys.path.insert(0, str(X3))
    spec = importlib.util.spec_from_file_location("x3_near_intents", X3 / "near_intents.py")
    module = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    spec.loader.exec_module(module)
    return module


ni = _load_compiler()

#: One edit per case: (what the agent tried, exact text to replace, replacement,
#: refusal code, inner type-checker code when the refusal is a type error).
EDITS: List[Dict[str, Any]] = [
    {"case": "no_slippage_bound", "tried": "drop `require slippage`",
     "old": "    require slippage <= 0.5%\n", "new": "",
     "code": "X3_NEAR_SLIPPAGE_REQUIRED"},
    {"case": "no_timeout", "tried": "drop the timeout and refund",
     "old": "    timeout 10m refund Solana.USDC\n", "new": "",
     "code": "X3_NEAR_TIMEOUT_REQUIRED"},
    {"case": "no_nonce", "tried": "drop the one-use nonce",
     "old": "    require nonce unused agent_payment_001\n", "new": "",
     "code": "X3_NEAR_NONCE"},
    {"case": "second_leg", "tried": "add a second swap leg",
     "old": "        bridge NearIntents Solana.USDC -> Near.USDC min_receive 24800000;\n",
     "new": ("        bridge NearIntents Solana.USDC -> Near.USDC min_receive 24800000;\n"
             "        bridge NearIntents Near.USDC -> Near.wNEAR;\n"),
     "code": "X3_NEAR_ROUTE_SHAPE"},
    {"case": "unenforceable_guard", "tried": "add a finality guard 1Click can't enforce",
     "old": "    require nonce unused agent_payment_001\n",
     "new": "    require nonce unused agent_payment_001\n    require finality Solana >= 32\n",
     "code": "X3_NEAR_GUARD_UNENFORCEABLE"},
    {"case": "refund_other_asset", "tried": "refund in a different asset",
     "old": "timeout 10m refund Solana.USDC", "new": "timeout 10m refund Near.USDC",
     "code": "X3_NEAR_REFUND_ASSET"},
    {"case": "lookalike_recipient", "tried": "pay a Cyrillic look-alike of x3-demo.near",
     "old": "receiver x3-demo.near", "new": "receiver x3-demo.neаr",
     "code": "X3_NEAR_TYPECHECK", "inner": "X3_INVALID_RECEIVER"},
    {"case": "wrong_chain_address", "tried": "send from Ethereum with a Solana address",
     "old": "from Solana.USDC", "new": "from Ethereum.USDC",
     "code": "X3_NEAR_TYPECHECK", "inner": "X3_INVALID_RECEIVER"},
    {"case": "zero_amount", "tried": "pay an amount of 0",
     "old": "amount 25000000", "new": "amount 0",
     "code": "X3_NEAR_TYPECHECK", "inner": "X3_INVALID_AMOUNT"},
]


def _compile(path: Path) -> Dict[str, Any]:
    try:
        ni.compile_program(str(path))
    except ni.NearIntentsError as exc:
        detail = exc.detail if isinstance(exc.detail, list) else []
        inner = sorted({d.get("code") for d in detail if isinstance(d, dict) and d.get("code")})
        return {"compiled": False, "code": exc.code, "inner": inner, "message": str(exc)}
    return {"compiled": True, "code": None, "inner": [], "message": None}


def evaluate(work: Path, program: Path = PROGRAM) -> Dict[str, Any]:
    source = program.read_text(encoding="utf-8")
    baseline = _compile(program)
    rows = []
    for edit in EDITS:
        if source.count(edit["old"]) != 1:
            raise SystemExit(f"{edit['case']}: the anchor text no longer appears exactly once in {program.name}")
        edited = work / f"{edit['case']}.x3"
        edited.write_text(source.replace(edit["old"], edit["new"], 1), encoding="utf-8")
        got = _compile(edited)
        ok = (not got["compiled"] and got["code"] == edit["code"]
              and (edit.get("inner") is None or edit["inner"] in got["inner"]))
        rows.append({"case": edit["case"], "tried": edit["tried"], "expected": edit["code"],
                     "expected_inner": edit.get("inner"), **got, "pass": ok})
    return {
        "title": "X3: unsafe agent payments that do not compile",
        "program": str(program.relative_to(ROOT)),
        "baseline_compiles": baseline["compiled"],
        "refused": sum(not r["compiled"] for r in rows),
        "passed": sum(r["pass"] for r in rows),
        "total": len(rows),
        "ok": baseline["compiled"] and all(r["pass"] for r in rows),
        "cases": rows,
    }


def render(report: Dict[str, Any]) -> str:
    out = [
        report["title"],
        f"Program: {report['program']} -> "
        f"{'compiles' if report['baseline_compiles'] else 'DOES NOT COMPILE (FAIL)'}",
        "",
        f"{'agent tried to':<44} {'refused with':<42} result",
        "-" * 94,
    ]
    for r in report["cases"]:
        code = r["code"] or "compiled"
        if r["inner"]:
            code += f" ({', '.join(r['inner'])})"
        out.append(f"{r['tried']:<44} {code:<42} {'PASS' if r['pass'] else 'FAIL'}")
    out += [
        "-" * 94,
        f"{report['refused']}/{report['total']} edits refused at compile time; "
        f"{report['passed']}/{report['total']} with the expected code.",
        "A program that does not compile never reaches the mandate check, a quote or Guard.",
    ]
    return "\n".join(out)


def main(argv: Optional[List[str]] = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--json", action="store_true", help="print the report as JSON")
    args = parser.parse_args(argv)
    with tempfile.TemporaryDirectory(prefix="x3-wont-compile-") as tmp:
        report = evaluate(Path(tmp))
    print(json.dumps(report, indent=2, ensure_ascii=True) if args.json else render(report))
    return 0 if report["ok"] else 1


if __name__ == "__main__":
    sys.exit(main())
