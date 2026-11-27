"""X3CCP AI layer: GBNF grammar vs the real compiler, diagnostics, and the stdio MCP server."""

from __future__ import annotations

import json
import os
import subprocess
import sys
from datetime import datetime, timezone

import pytest

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, ROOT)

import x3ccp  # noqa: E402
import x3ccp_ai  # noqa: E402
from tools import x3ccp_fixtures  # noqa: E402

NOW = datetime(2026, 11, 26, tzinfo=timezone.utc)
EXAMPLE = open(os.path.join(ROOT, "examples", "x3ccp_treasury_guard.x3"), encoding="utf-8").read()
AGENT = "0x1111111111111111111111111111111111111111"
STORE = "0x2222222222222222222222222222222222222222"


def _valid_sources():
    yield EXAMPLE
    yield from x3ccp_fixtures.sources().values()
    yield ("control minimal {\n  target base-sepolia 0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f\n"
           "  owner 0x7FA9385bE102ac3EAc297483Dd6233D62b3e1496\n  allow callers " + AGENT + "\n"
           "  allow fn ping()\n  rate ping calls per 30s <= 2;  // trailing semicolon and comment\n"
           "  expires 7d\n}")


@pytest.mark.parametrize("source", list(_valid_sources()))
def test_grammar_accepts_every_compilable_source(source):
    x3ccp.compile_source(source, now=NOW)
    assert x3ccp_ai.grammar_accepts(source)


@pytest.mark.parametrize("broken", [
    EXAMPLE.replace("allow fn withdraw", "alow fn withdraw"),
    EXAMPLE.replace("control treasury_guard {", "control treasury guard {"),
    EXAMPLE.replace("limit withdraw.amount <= 50_000_000", "limit withdraw.amount < 50_000_000"),
    EXAMPLE.replace("rate withdraw calls per 1h <= 3", "rate withdraw calls per hour <= 3"),
    EXAMPLE.replace("0x1111111111111111111111111111111111111111", "0x11111"),
    EXAMPLE.replace("target local", "target ethereum"),
    EXAMPLE.replace("allow fn setMemo(bytes32 memo, bool urgent)", "allow fn setMemo(string memo)"),
    EXAMPLE.rstrip().rstrip("}"),
    EXAMPLE.replace("expires at 2030-01-01T00:00:00Z", "expires at 2030-01-01"),
])
def test_grammar_rejects_syntax_the_compiler_rejects(broken):
    assert not x3ccp_ai.grammar_accepts(broken)
    with pytest.raises(x3ccp.X3CCPError):
        x3ccp.compile_source(broken, now=NOW)


def test_grammar_is_syntax_only_semantics_stay_with_the_compiler():
    # Syntactically fine, semantically wrong: unknown parameter.
    bad = EXAMPLE.replace("limit withdraw.amount <= 50_000_000", "limit withdraw.amt <= 50_000_000")
    report = x3ccp_ai.diagnose(bad, now=NOW)
    assert report["grammar_ok"] and not report["ok"]
    assert report["errors"][0]["code"] == "X3CCP_UNKNOWN_PARAM"
    assert "to, amount" in report["errors"][0]["hint"]


def test_diagnose_success_carries_explanation_and_owner_step():
    report = x3ccp_ai.diagnose(EXAMPLE, now=NOW)
    assert report["ok"] and report["grammar_ok"]
    assert report["policy_hash"] == x3ccp.compile_source(EXAMPLE, now=NOW)["policy_hash"]
    assert "owner" in report["next_step"] and report["explain"]


def test_gbnf_recognizer_basics():
    g = 'root ::= "a" [0-9]{2} ("x" | "yz")* [^q]?\n'
    assert x3ccp_ai.grammar_accepts("a12", g)
    assert x3ccp_ai.grammar_accepts("a12xyzx!", g)
    assert not x3ccp_ai.grammar_accepts("a1", g)
    assert not x3ccp_ai.grammar_accepts("a12q", g)


# ------------------------------------------------------------------------ MCP over stdio


def _session(messages):
    proc = subprocess.run([sys.executable, os.path.join(ROOT, "x3ccp_mcp.py")],
                          input="".join(json.dumps(m) + "\n" for m in messages), capture_output=True, text=True,
                          timeout=60)
    assert proc.returncode == 0, proc.stderr
    return [json.loads(line) for line in proc.stdout.splitlines() if line.strip()]


def _call(i, name, arguments):
    return {"jsonrpc": "2.0", "id": i, "method": "tools/call", "params": {"name": name, "arguments": arguments}}


def test_mcp_session_end_to_end():
    out = _session([
        {"jsonrpc": "2.0", "id": 1, "method": "initialize",
         "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "t", "version": "0"}}},
        {"jsonrpc": "2.0", "method": "notifications/initialized"},
        {"jsonrpc": "2.0", "id": 2, "method": "tools/list"},
        _call(3, "x3ccp_language", {}),
        _call(4, "x3ccp_compile", {"source": EXAMPLE, "now": "2026-11-26T00:00:00Z"}),
        _call(5, "x3ccp_compile", {"source": EXAMPLE.replace("withdraw.to in", "withdraw.recipient in")}),
        _call(6, "x3ccp_simulate", {"source": EXAMPLE, "calls": [
            {"sender": AGENT, "fn": "withdraw", "args": [STORE, 5_000], "timestamp": 1_800_000_000},
            {"sender": AGENT, "fn": "withdraw", "args": [STORE, 60_000_000], "timestamp": 1_800_000_001},
        ]}),
        _call(7, "x3ccp_decode_revert", {"revert_data": "0x" + x3ccp.refusal_revert_data(19, 7).hex()}),
        _call(8, "x3ccp_explain", {"source": EXAMPLE}),
        {"jsonrpc": "2.0", "id": 9, "method": "nope"},
        _call(10, "x3ccp_unknown", {}),
    ])
    by_id = {m["id"]: m for m in out}
    assert len(out) == 10  # the notification gets no reply
    assert by_id[1]["result"]["protocolVersion"] == "2025-06-18"
    assert {t["name"] for t in by_id[2]["result"]["tools"]} == {
        "x3ccp_language", "x3ccp_compile", "x3ccp_explain", "x3ccp_simulate", "x3ccp_decode_revert"}
    assert all(t["annotations"]["readOnlyHint"] for t in by_id[2]["result"]["tools"])
    assert "control <name>" in by_id[3]["result"]["structuredContent"]["language_card"]
    ok = by_id[4]["result"]
    assert not ok["isError"] and ok["structuredContent"]["policy_hash"].startswith("0x")
    err = by_id[5]["result"]
    assert err["isError"] and err["structuredContent"]["errors"][0]["code"] == "X3CCP_UNKNOWN_PARAM"
    sim = by_id[6]["result"]["structuredContent"]["results"]
    assert sim[0]["allowed"] and not sim[1]["allowed"] and sim[1]["clause"] == 19
    assert sim[1]["revert_data"] == "0x" + x3ccp.refusal_revert_data(19, 7).hex()
    assert by_id[7]["result"]["structuredContent"]["reason"] == "ABOVE_MAX"
    assert by_id[8]["result"]["structuredContent"]["explain"][0]["line"] == 14
    assert by_id[9]["error"]["code"] == -32601
    assert by_id[10]["error"]["code"] == -32602


def test_mcp_survives_garbage_input():
    proc = subprocess.run([sys.executable, os.path.join(ROOT, "x3ccp_mcp.py")],
                          input="not json\n" + json.dumps({"jsonrpc": "2.0", "id": 1, "method": "ping"}) + "\n",
                          capture_output=True, text=True, timeout=60)
    lines = [json.loads(x) for x in proc.stdout.splitlines()]
    assert lines[0]["error"]["code"] == -32700 and lines[1]["result"] == {}


def test_oversized_input_is_refused_before_any_work():
    huge = EXAMPLE + "// pad\n" * 20_000
    report = x3ccp_ai.diagnose(huge, now=NOW)
    assert not report["ok"] and report["errors"][0]["code"] == "X3CCP_LIMIT"
    with pytest.raises(x3ccp.X3CCPError):
        x3ccp.compile_source(huge, now=NOW)
