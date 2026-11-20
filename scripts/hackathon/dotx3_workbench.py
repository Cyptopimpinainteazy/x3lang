#!/usr/bin/env python3
"""Serve the real .x3 compiler and a non-signing dotx3 workbench on loopback."""
from __future__ import annotations

import argparse
from datetime import datetime, timezone
from functools import partial
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
from pathlib import Path
import sys
import tempfile
from threading import Lock
from types import SimpleNamespace
from urllib.parse import urlsplit

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "x3-lang"))
import agent_law
import near_intents as ni
import track_control
import mainnet_agent_preflight as preflight

STATIC = ROOT / "x3-lang/near/dotx3"
TOKENS = ROOT / "x3-lang/tests/fixtures/near_intents_tokens.json"
MAX_BODY = 96 * 1024
MAX_SOURCE = 32 * 1024


def source_input(payload):
    if not isinstance(payload, dict):
        raise ValueError("Expected a JSON object")
    for field in ("program", "mandate"):
        text = payload.get(field)
        if not isinstance(text, str) or not text.strip() or len(text.encode()) > MAX_SOURCE:
            raise ValueError(f"{field} must contain 1–32768 bytes of .x3 source")
    return payload["program"], payload["mandate"]


def compile_sources(payload, *, now=None):
    """No model, network, wallet or chain state. Execute existing compiler/checker."""
    program, mandate = source_input(payload)
    now = now or datetime.now(timezone.utc)
    result = {
        "schema": "x3.dotx3.workbench.v1", "evaluated_at": now.isoformat(),
        "program_sha256": hashlib.sha256(program.encode()).hexdigest(),
        "mandate_sha256": hashlib.sha256(mandate.encode()).hexdigest(),
        "evidence_level": "local_compiler_and_policy_only",
        "token_registry": "checked_in_fixture_not_live_discovery",
        "model_provenance": "user_supplied_source_not_verified_model_output",
        "wallet_authorized": False, "onchain_authorization_verified": False,
        "settlement_proven": False,
        "assumed_state": {"revoked": False, "spends": []},
    }
    with tempfile.TemporaryDirectory(prefix="x3-workbench-") as folder:
        p, m = Path(folder) / "payment.x3", Path(folder) / "mandate.x3"
        p.write_text(program, encoding="utf-8")
        m.write_text(mandate, encoding="utf-8")
        try:
            _, leg = ni.compile_program(str(p), now=now)
            result.update(compiled=True, stage="mandate", leg=leg)
            tokens = json.loads(TOKENS.read_text())
            policy = track_control.evaluate(str(m), str(p), tokens, now=now)
            result.update(decision=policy["decision"], policy=policy)
        except ni.NearIntentsError as exc:
            result.update(decision="compile_error" if "compiled" not in result else "mandate_error",
                          compiled=result.get("compiled", False),
                          error={"code": exc.code, "message": exc.message, "detail": exc.detail})
    return result


def dry_preview(payload):
    program, mandate = source_input(payload)
    required = ("max_input_usdc", "guard", "expected_guard_code_hash")
    if any(not isinstance(payload.get(k), str) or not payload[k] for k in required):
        raise ValueError("Supply an explicit budget, mainnet Guard and reviewed code hash")
    with tempfile.TemporaryDirectory(prefix="x3-workbench-dry-") as folder:
        p, m = Path(folder) / "payment.x3", Path(folder) / "mandate.x3"
        p.write_text(program, encoding="utf-8")
        m.write_text(mandate, encoding="utf-8")
        return preflight.run(SimpleNamespace(program=str(p), mandate=str(m),
                                            **{k: payload[k] for k in required}))


def examples():
    return {"program": (ROOT / "x3-lang/examples/near_intents_agent_payment.x3").read_text(),
            "mandate": (ROOT / "x3-lang/examples/near_mandate_agent.x3").read_text(),
            "label": "Checked-in staged payment; illustrative accounts; no live model invocation"}


def agent_law_view():
    """The local invoice_06 path. It never authorizes a wallet or a settlement."""
    try:
        return agent_law.public_view(agent_law.demonstrate())
    except Exception:
        return {"ok": False, "wallet_authorized": False, "settlement_proven": False,
                "broadcast": False, "settlement": False, "error": "Local path failed"}


_DOOR = agent_law.Door()
_DOOR_LOCK = Lock()


def reset_door():
    """Tests start from an empty process-local session."""
    global _DOOR
    with _DOOR_LOCK:
        _DOOR = agent_law.Door()


def law_act_input(payload):
    if not isinstance(payload, dict):
        raise ValueError("Expected a JSON object")
    act = payload.get("act")
    if act not in agent_law.DOOR_ACTS:
        raise ValueError("Unknown act")
    mandate = payload.get("mandate", "")
    program = payload.get("program", "")
    if not isinstance(mandate, str) or not isinstance(program, str):
        raise ValueError("Source fields must be strings")
    if len(mandate.encode()) > MAX_SOURCE or len(program.encode()) > MAX_SOURCE:
        raise ValueError("Source exceeds size limit")
    if act == "birth" and not mandate.strip():
        raise ValueError("mandate must contain .x3 source")
    if act in {"test", "authorize"} and not program.strip():
        raise ValueError("program must contain .x3 source")
    return {"act": act, "mandate": mandate, "program": program}


def law_act(payload):
    """Run one editor source through the local door. The archived exhibit stays separate."""
    fields = law_act_input(payload)
    with _DOOR_LOCK:
        report = _DOOR.act(fields)
    report["wallet_authorized"] = False
    report["settlement"] = False
    report["settlement_proven"] = False
    report["broadcast"] = False
    if report.get("receipt"):
        report["receipt"]["broadcast"] = False
        report["receipt"]["solana_custody"] = "outside_this_path"
    return report


class Handler(BaseHTTPRequestHandler):
    server_version = "DotX3Workbench/1"

    def __init__(self, *args, enable_mainnet_readonly=False, **kwargs):
        self.enable_mainnet_readonly = enable_mainnet_readonly
        super().__init__(*args, **kwargs)

    def log_message(self, *_args):
        pass  # Never log source, quote contents or request parameters.

    def reply(self, status, data, kind="application/json; charset=utf-8"):
        raw = json.dumps(data, default=str).encode() if kind.startswith("application/json") else data
        self.send_response(status)
        self.send_header("Content-Type", kind)
        self.send_header("Content-Length", str(len(raw)))
        self.send_header("Cache-Control", "no-store")
        self.send_header("X-Content-Type-Options", "nosniff")
        self.send_header("X-Frame-Options", "DENY")
        self.send_header("Referrer-Policy", "no-referrer")
        self.send_header("Content-Security-Policy", "default-src 'self'; script-src 'self'; style-src 'self'; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'")
        self.end_headers()
        self.wfile.write(raw)

    def local_request(self):
        host = self.headers.get("Host", "")
        expected = f"127.0.0.1:{self.server.server_port}"
        if host not in {expected, f"localhost:{self.server.server_port}"}:
            return False
        origin = self.headers.get("Origin")
        return (origin is None or origin in {f"http://{expected}", f"http://localhost:{self.server.server_port}"}) and self.headers.get("Sec-Fetch-Site") not in {"cross-site"}

    def do_GET(self):
        if not self.local_request():
            return self.reply(403, {"error": "Loopback origin required"})
        path = urlsplit(self.path).path
        if path == "/api/examples":
            return self.reply(200, examples())
        if path == "/api/health":
            return self.reply(200, {"compiler": "python_x3_frontend", "signing": False,
                                    "mainnet_readonly": self.enable_mainnet_readonly})
        if path == "/api/agent-law":
            return self.reply(200, agent_law_view())
        files = {"/": ("index.html", "text/html; charset=utf-8"),
                 "/workbench.mjs": ("workbench.mjs", "text/javascript; charset=utf-8"),
                 "/workbench.css": ("workbench.css", "text/css; charset=utf-8")}
        if path not in files:
            return self.reply(404, {"error": "Not found"})
        name, kind = files[path]
        return self.reply(200, (STATIC / name).read_bytes(), kind)

    def do_POST(self):
        if not self.local_request():
            return self.reply(403, {"error": "Loopback origin required"})
        if self.path not in {"/api/compile", "/api/dry-preflight", "/api/agent-law/act"}:
            return self.reply(404, {"error": "Not found"})
        if self.path == "/api/dry-preflight" and not self.enable_mainnet_readonly:
            return self.reply(403, {"error": "Read-only mainnet preflight is disabled"})
        try:
            if self.headers.get("Content-Type", "").split(";")[0] != "application/json":
                return self.reply(415, {"error": "application/json required"})
            if self.headers.get("Transfer-Encoding"):
                return self.reply(400, {"error": "Transfer encoding unsupported"})
            length = int(self.headers.get("Content-Length", "0"))
            if not 0 < length <= MAX_BODY:
                return self.reply(413, {"error": "Request exceeds size limit or is empty"})
            payload = json.loads(self.rfile.read(length))
            if self.path == "/api/agent-law/act":
                report = law_act(payload)
            elif self.path == "/api/compile":
                report = compile_sources(payload)
            else:
                report = dry_preview(payload)
            return self.reply(200, report)
        except (ValueError, TypeError, KeyError, UnicodeError):
            return self.reply(400, {"error": "Invalid request; use bounded .x3 source and public inputs only"})
        except Exception:
            return self.reply(500, {"error": "Evaluation failed; no authorization was produced"})


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--port", type=int, default=8793)
    parser.add_argument("--enable-mainnet-readonly", action="store_true")
    args = parser.parse_args()
    server = ThreadingHTTPServer(("127.0.0.1", args.port),
                                 partial(Handler, enable_mainnet_readonly=args.enable_mainnet_readonly))
    server.daemon_threads = True
    print(f".x3 workbench: http://127.0.0.1:{args.port} · no signing or deposits", flush=True)
    server.serve_forever()


if __name__ == "__main__":
    main()
