"""X3CCP MCP server: lets any MCP client (Claude, Cursor, Agent Forge agents) write X3 policies.

Run over stdio (newline-delimited JSON-RPC 2.0, MCP protocol):

    python3 x3-lang/x3ccp_mcp.py

Client config example (Claude Desktop / Claude Code `mcpServers`):

    {"x3ccp": {"command": "python3", "args": ["/path/to/x3lang/x3-lang/x3ccp_mcp.py"]}}

Tools: x3ccp_language, x3ccp_compile, x3ccp_explain, x3ccp_simulate, x3ccp_decode_revert.
Every tool is pure: no network, no keys, no signing, no transactions. A model can draft and test
a policy; only the contract owner's own installPolicy transaction gives it force.
"""

from __future__ import annotations

import json
import os
import sys
import traceback
from datetime import datetime
from typing import Any, Dict, Optional

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import x3ccp  # noqa: E402
import x3ccp_ai  # noqa: E402

SERVER_INFO = {"name": "x3ccp", "version": "1.0.0"}
PROTOCOL_VERSION = "2025-06-18"
SUPPORTED_PROTOCOLS = ("2025-06-18", "2025-03-26", "2024-11-05")

_SOURCE = {"type": "string", "description": "Full text of one `control { ... }` block"}
_NOW = {"type": "string", "description": "Optional compile time, ISO-8601 UTC, for reproducible relative expiry"}

TOOLS = [
    {
        "name": "x3ccp_language",
        "description": "Return the X3CCP `control` language card and its GBNF grammar. Read this before "
                       "writing a policy.",
        "inputSchema": {"type": "object", "properties": {}, "additionalProperties": False},
        "annotations": {"readOnlyHint": True, "openWorldHint": False},
    },
    {
        "name": "x3ccp_compile",
        "description": "Compile a `control` block. On error returns a stable code, the line and a hint to "
                       "repair from. On success returns the policy bytes, their keccak256 hash, warnings and a "
                       "plain-English explanation for the owner.",
        "inputSchema": {"type": "object", "properties": {"source": _SOURCE, "now": _NOW}, "required": ["source"],
                        "additionalProperties": False},
        "annotations": {"readOnlyHint": True, "openWorldHint": False},
    },
    {
        "name": "x3ccp_explain",
        "description": "Turn a `control` block into one plain-English sentence per clause, keyed by line.",
        "inputSchema": {"type": "object", "properties": {"source": _SOURCE, "now": _NOW}, "required": ["source"],
                        "additionalProperties": False},
        "annotations": {"readOnlyHint": True, "openWorldHint": False},
    },
    {
        "name": "x3ccp_simulate",
        "description": "Predict exactly what the X3Controlled contract would do with a sequence of calls, "
                       "carrying rate usage forward. Each call: {sender, fn, args, timestamp} or "
                       "{sender, calldata, timestamp}. Returns allowed/refused, the clause line, the reason "
                       "and the exact revert data.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "source": _SOURCE,
                "now": _NOW,
                "calls": {"type": "array", "items": {"type": "object", "properties": {
                    "sender": {"type": "string"},
                    "fn": {"type": "string"},
                    "args": {"type": "array"},
                    "calldata": {"type": "string"},
                    "timestamp": {"type": "integer"},
                    "chain_id": {"type": "integer"},
                }, "required": ["sender", "timestamp"]}},
            },
            "required": ["source", "calls"],
            "additionalProperties": False,
        },
        "annotations": {"readOnlyHint": True, "openWorldHint": False},
    },
    {
        "name": "x3ccp_decode_revert",
        "description": "Decode X3Refused(clause, code) revert data from a failed transaction.",
        "inputSchema": {"type": "object", "properties": {"revert_data": {"type": "string"}},
                        "required": ["revert_data"], "additionalProperties": False},
        "annotations": {"readOnlyHint": True, "openWorldHint": False},
    },
]


def _now(args: Dict[str, Any]) -> Optional[datetime]:
    text = args.get("now")
    return datetime.fromisoformat(text.replace("Z", "+00:00")) if text else None


def _result(payload: Any, is_error: bool = False) -> Dict[str, Any]:
    return {"content": [{"type": "text", "text": json.dumps(payload, indent=2)}],
            "structuredContent": payload if isinstance(payload, dict) else {"result": payload},
            "isError": is_error}


def call_tool(name: str, args: Dict[str, Any]) -> Dict[str, Any]:
    if name == "x3ccp_language":
        return _result({"language_card": x3ccp_ai.LANGUAGE_CARD, "gbnf": x3ccp_ai.GRAMMAR,
                        "refusal_codes": x3ccp.REFUSAL_CODES})
    if name == "x3ccp_compile":
        report = x3ccp_ai.diagnose(args["source"], now=_now(args))
        return _result(report, is_error=not report["ok"])
    if name in ("x3ccp_explain", "x3ccp_simulate"):
        try:
            compiled = x3ccp.compile_source(args["source"], now=_now(args))
            if name == "x3ccp_explain":
                return _result({"policy_hash": compiled["policy_hash"], "explain": x3ccp.explain(compiled)})
            return _result({"policy_hash": compiled["policy_hash"],
                            "results": x3ccp.simulate(compiled, args["calls"])})
        except x3ccp.X3CCPError as exc:
            return _result({"ok": False, "errors": [exc.to_dict()]}, is_error=True)
    if name == "x3ccp_decode_revert":
        text = str(args["revert_data"]).removeprefix("0x")
        try:
            decoded = x3ccp.decode_refusal(bytes.fromhex(text))
        except ValueError:
            decoded = None
        if decoded is None:
            return _result({"ok": False, "message": "not X3Refused(uint16,uint8) revert data; an empty revert "
                                                    "usually means Solidity's ABI decoder rejected the call"},
                           is_error=True)
        return _result({"ok": True, **decoded})
    raise KeyError(name)


def handle(msg: Dict[str, Any]) -> Optional[Dict[str, Any]]:
    """Handle one JSON-RPC message; return the response, or None for notifications."""
    method = msg.get("method")
    msg_id = msg.get("id")
    if msg_id is None:
        return None  # notification (e.g. notifications/initialized)
    try:
        if method == "initialize":
            requested = (msg.get("params") or {}).get("protocolVersion")
            version = requested if requested in SUPPORTED_PROTOCOLS else PROTOCOL_VERSION
            result: Any = {"protocolVersion": version, "capabilities": {"tools": {"listChanged": False}},
                           "serverInfo": SERVER_INFO,
                           "instructions": "Write X3CCP policies with x3ccp_language, then x3ccp_compile and "
                                           "repair from its hints. Show x3ccp_explain output to the owner. "
                                           "These tools never sign or send anything."}
        elif method == "ping":
            result = {}
        elif method == "tools/list":
            result = {"tools": TOOLS}
        elif method == "tools/call":
            params = msg.get("params") or {}
            name = params.get("name")
            if name not in {t["name"] for t in TOOLS}:
                return {"jsonrpc": "2.0", "id": msg_id, "error": {"code": -32602, "message": f"unknown tool {name}"}}
            try:
                result = call_tool(name, params.get("arguments") or {})
            except (KeyError, TypeError, ValueError) as exc:
                result = _result({"ok": False, "message": f"bad arguments: {exc}"}, is_error=True)
        else:
            return {"jsonrpc": "2.0", "id": msg_id, "error": {"code": -32601, "message": f"unknown method {method}"}}
        return {"jsonrpc": "2.0", "id": msg_id, "result": result}
    except Exception as exc:  # pragma: no cover - last-resort guard keeps the server alive
        traceback.print_exc(file=sys.stderr)
        return {"jsonrpc": "2.0", "id": msg_id, "error": {"code": -32603, "message": str(exc)}}


def main() -> int:
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            msg = json.loads(line)
        except json.JSONDecodeError:
            out: Optional[Dict[str, Any]] = {"jsonrpc": "2.0", "id": None,
                                             "error": {"code": -32700, "message": "parse error"}}
        else:
            out = handle(msg)
        if out is not None:
            sys.stdout.write(json.dumps(out) + "\n")
            sys.stdout.flush()
    return 0


if __name__ == "__main__":
    sys.exit(main())
