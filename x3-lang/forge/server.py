"""X3 Agent Forge HTTP host (stdlib only).

    PYTHONPATH=x3-lang python3 -m forge.server --data ~/.x3-forge [--port 8787] [--tokens FILE | --live-tokens]

Serves the built web app from `forge/web/dist` and a JSON API under `/api`. Binds to
127.0.0.1 by default; put it behind Cloudflare Access (or another login) before exposing it.
On first start it prints a one-time setup token: High Command is claimed by signing it with the
owner's browser-held Ed25519 key.
"""
from __future__ import annotations

import argparse
import json
import logging
import mimetypes
import os
import re
import secrets
import sys
import urllib.parse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Any, Callable, Dict, Optional, Tuple

from . import core
from .store import Forge

log = logging.getLogger("x3.forge")
WEB_DIST = core.FORGE_DIR / "web" / "dist"
MAX_BODY = 256 * 1024
SESSION_COOKIE = "forge_sid"
CSP = ("default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; "
       "connect-src 'self'; font-src 'self' data:; worker-src 'self' blob:; frame-ancestors 'none'; "
       "base-uri 'none'; form-action 'self'")
AGENT_RE = r"([a-z0-9._-]{2,64})"
CONN_RE = r"([0-9a-f]{32})"


class Api:
    """Routes: (method, regex) -> handler(ctx, match, body)."""

    def __init__(self, forge: Forge):
        self.forge = forge
        f = forge
        self.routes: list[Tuple[str, re.Pattern, Callable[..., Any]]] = []
        r = self._route
        r("GET", r"/api/status", lambda c, m, b: f.status())
        r("GET", r"/api/registry", lambda c, m, b: {**f.reg.public(), "assets": f.assets.public(), "status": f.status()})
        r("POST", r"/api/claim/payload", lambda c, m, b: {
            "payload_hex": f.claim_payload(_s(b, "public_key"), _s(b, "token")).hex()})
        r("POST", r"/api/claim", lambda c, m, b: f.claim(b.get("token"), b.get("public_key"), b.get("signature")))
        r("POST", r"/api/compile", lambda c, m, b: f.compile(b.get("build")))
        r("POST", r"/api/prove", lambda c, m, b: f.compile_and_prove(b.get("build")))
        r("POST", r"/api/connect/start", lambda c, m, b: f.connections.start(
            c["session"], b.get("connector"), b.get("config"), b.get("agent"), c["base"] + "/api/connect/callback"))
        r("POST", r"/api/connect/confirm", lambda c, m, b: f.connections.confirm(
            c["session"], b.get("state"), _s(b, "connection")))
        r("POST", r"/api/connect/secret", lambda c, m, b: f.connections.set_secret(
            c["session"], _s(b, "connection"), b.get("secret")))
        r("POST", r"/api/connect/health", lambda c, m, b: f.connections.health(c["session"], _s(b, "connection")))
        r("GET", rf"/api/connect/{CONN_RE}", lambda c, m, b: f.connections.public(
            f.connections.get(m.group(1), c["session"])))
        r("POST", r"/api/birth/prepare", lambda c, m, b: f.prepare_birth(
            c["session"], b.get("build"), b.get("agent_public_key")))
        r("POST", r"/api/birth/commit", lambda c, m, b: f.commit_birth(
            c["session"], b.get("birth_id"), b.get("signature")))
        r("GET", r"/api/agents", lambda c, m, b: {"agents": f.roster()})
        r("GET", rf"/api/agents/{AGENT_RE}", lambda c, m, b: f.agent(m.group(1)))
        r("POST", rf"/api/agents/{AGENT_RE}/revoke/payload", lambda c, m, b: {"payload_hex": f.revoke_payload(
            m.group(1), _s(b, "reason"), _s(b, "nonce"), connection_id=b.get("connection_id")).hex()})
        r("POST", rf"/api/agents/{AGENT_RE}/revoke", lambda c, m, b: f.revoke(
            m.group(1), b.get("reason"), b.get("nonce"), b.get("signature")))
        r("POST", rf"/api/agents/{AGENT_RE}/connections/{CONN_RE}/revoke", lambda c, m, b: f.revoke_connection(
            m.group(1), m.group(2), b.get("reason"), b.get("nonce"), b.get("signature")))
        r("GET", rf"/api/agents/{AGENT_RE}/export", lambda c, m, b: f.export(m.group(1)))
        r("POST", r"/api/import", lambda c, m, b: f.verify_import(b.get("manifest")))
        r("POST", r"/api/mesh/payload", lambda c, m, b: {"payload_hex": f.mesh_payload(
            _s(b, "from"), _s(b, "to"), _s(b, "type"), _s(b, "body"), _s(b, "nonce"), _i(b, "expires_at")).hex()})
        r("POST", r"/api/mesh/send", lambda c, m, b: f.mesh_send(
            b.get("from"), b.get("to"), b.get("type"), b.get("body"), b.get("nonce"), b.get("expires_at"),
            b.get("signature")))

    def _route(self, method: str, pattern: str, fn: Callable[..., Any]) -> None:
        self.routes.append((method, re.compile(pattern + r"\Z"), fn))

    def find(self, method: str, path: str):
        allowed = False
        for meth, pat, fn in self.routes:
            m = pat.match(path)
            if m:
                if meth == method:
                    return fn, m, True
                allowed = True
        return None, None, allowed


def _s(body: Dict[str, Any], key: str) -> str:
    v = body.get(key)
    if not isinstance(v, str) or len(v) > 4096:
        raise core.ForgeError("X3_FORGE_REQUEST", f"{key} must be a string", "request")
    return v


def _i(body: Dict[str, Any], key: str) -> int:
    v = body.get(key)
    if type(v) is not int:
        raise core.ForgeError("X3_FORGE_REQUEST", f"{key} must be an integer", "request")
    return v


def make_handler(api: Api, *, web_dist: Path = WEB_DIST, secure_cookie: bool = False):
    class Handler(BaseHTTPRequestHandler):
        server_version = "x3-agent-forge/1"
        sys_version = ""

        def log_message(self, fmt: str, *args: Any) -> None:  # keep query strings (setup states) out of logs
            log.info("%s %s %s", self.command, urllib.parse.urlsplit(self.path).path, args[1] if len(args) > 1 else "")

        # -- helpers -------------------------------------------------------------------------
        def _session(self) -> Tuple[str, bool]:
            raw = self.headers.get("Cookie", "")
            for part in raw.split(";"):
                k, _, v = part.strip().partition("=")
                if k == SESSION_COOKIE and re.fullmatch(r"[A-Za-z0-9_-]{32,64}", v):
                    return v, False
            return secrets.token_urlsafe(32), True

        def _base(self) -> str:
            proto = self.headers.get("X-Forwarded-Proto") or ("https" if secure_cookie else "http")
            host = self.headers.get("Host", "127.0.0.1")
            return f"{proto}://{host}"

        def _headers(self, status: int, ctype: str, length: int, new_session: Optional[str], extra=()):
            self.send_response(status)
            self.send_header("Content-Type", ctype)
            self.send_header("Content-Length", str(length))
            self.send_header("Content-Security-Policy", CSP)
            self.send_header("X-Content-Type-Options", "nosniff")
            self.send_header("Referrer-Policy", "no-referrer")
            self.send_header("Cross-Origin-Opener-Policy", "same-origin")
            self.send_header("Cache-Control", "no-store" if ctype.startswith("application/json") else "no-cache")
            if new_session:
                flags = "; Secure" if secure_cookie else ""
                self.send_header("Set-Cookie", f"{SESSION_COOKIE}={new_session}; Path=/; HttpOnly; SameSite=Lax{flags}")
            for k, v in extra:
                self.send_header(k, v)
            self.end_headers()

        def _json(self, status: int, payload: Any, new_session: Optional[str]) -> None:
            data = json.dumps(payload, sort_keys=True).encode()
            self._headers(status, "application/json; charset=utf-8", len(data), new_session)
            self.wfile.write(data)

        def _same_origin(self) -> bool:
            origin = self.headers.get("Origin")
            if origin is None:
                return True
            host = self.headers.get("Host", "")
            return urllib.parse.urlsplit(origin).netloc == host

        # -- verbs ---------------------------------------------------------------------------
        def do_GET(self) -> None:
            self._dispatch("GET")

        def do_POST(self) -> None:
            self._dispatch("POST")

        def _dispatch(self, method: str) -> None:
            session, new = self._session()
            new_session = session if new else None
            parts = urllib.parse.urlsplit(self.path)
            path = parts.path
            if path == "/health":
                return self._json(200, {"ok": True}, None)
            if path == "/api/connect/callback" and method == "GET":
                return self._callback(session, new_session, parts.query)
            if path.startswith("/api/"):
                return self._api(method, path, session, new_session)
            if method != "GET":
                return self._json(405, {"errors": [{"code": "X3_FORGE_METHOD", "message": "method not allowed"}]},
                                  new_session)
            return self._static(path, new_session)

        def _api(self, method: str, path: str, session: str, new_session: Optional[str]) -> None:
            fn, match, allowed = api.find(method, path)
            if fn is None:
                status = 405 if allowed else 404
                return self._json(status, {"errors": [{"code": "X3_FORGE_ROUTE", "message": "no such endpoint"}]},
                                  new_session)
            body: Dict[str, Any] = {}
            if method == "POST":
                if not self._same_origin():
                    return self._json(403, {"errors": [{"code": "X3_FORGE_ORIGIN", "message": "cross-origin request"}]},
                                      new_session)
                if not (self.headers.get("Content-Type") or "").startswith("application/json"):
                    return self._json(415, {"errors": [{"code": "X3_FORGE_CONTENT", "message": "send JSON"}]},
                                      new_session)
                length = int(self.headers.get("Content-Length") or 0)
                if length > MAX_BODY:
                    return self._json(413, {"errors": [{"code": "X3_FORGE_SIZE", "message": "request too large"}]},
                                      new_session)
                try:
                    body = json.loads(self.rfile.read(length) or b"{}")
                    if not isinstance(body, dict):
                        raise ValueError
                except ValueError:
                    return self._json(400, {"errors": [{"code": "X3_FORGE_JSON", "message": "invalid JSON"}]},
                                      new_session)
            ctx = {"session": session, "base": self._base()}
            try:
                return self._json(200, fn(ctx, match, body), new_session)
            except core.ForgeErrors as exc:
                return self._json(400, {"errors": exc.to_list()}, new_session)
            except core.ForgeError as exc:
                return self._json(400, {"errors": [exc.to_dict()]}, new_session)
            except Exception:  # never leak internals; log them
                log.exception("internal error on %s %s", method, path)
                return self._json(500, {"errors": [{"code": "X3_FORGE_INTERNAL", "message": "internal error"}]},
                                  new_session)

        def _callback(self, session: str, new_session: Optional[str], query: str) -> None:
            q = urllib.parse.parse_qs(query)
            state = (q.get("state") or q.get("x3_forge_state") or [None])[0]
            cid = (q.get("connection") or [""])[0]
            try:
                rec = api.forge.connections.callback(session, state, cid)
                target = f"/?linked={rec['id']}#comms"
                status = 303
            except core.ForgeError as exc:
                target = f"/?link_error={urllib.parse.quote(exc.message)}#comms"
                status = 303
            self._headers(status, "text/plain", 0, new_session, extra=[("Location", target)])

        def _static(self, path: str, new_session: Optional[str]) -> None:
            if not web_dist.exists():
                msg = b"Forge web app not built. Run: (cd x3-lang/forge/web && npm ci && npm run build)"
                self._headers(503, "text/plain; charset=utf-8", len(msg), new_session)
                self.wfile.write(msg)
                return
            rel = path.lstrip("/") or "index.html"
            target = (web_dist / rel).resolve()
            if not target.is_relative_to(web_dist.resolve()) or not target.is_file():
                target = web_dist / "index.html"  # SPA routes such as /agents/<id>
            data = target.read_bytes()
            ctype = mimetypes.guess_type(str(target))[0] or "application/octet-stream"
            if ctype.startswith("text/") or ctype in ("application/javascript",):
                ctype += "; charset=utf-8"
            self._headers(200, ctype, len(data), new_session)
            self.wfile.write(data)

    return Handler


def main(argv: Optional[list] = None) -> int:
    p = argparse.ArgumentParser(description="X3 Agent Forge host")
    p.add_argument("--host", default="127.0.0.1")
    p.add_argument("--port", type=int, default=8787)
    p.add_argument("--data", default=os.path.expanduser("~/.x3-forge"))
    g = p.add_mutually_exclusive_group()
    g.add_argument("--tokens", help="1Click /v0/tokens JSON file (default: the committed snapshot)")
    g.add_argument("--live-tokens", action="store_true", help="fetch the live 1Click token list at start")
    p.add_argument("--secure-cookie", action="store_true", help="set when served over HTTPS")
    args = p.parse_args(argv)
    logging.basicConfig(level=logging.INFO, format="%(asctime)s %(levelname)s %(message)s")
    mimetypes.add_type("application/javascript", ".js")
    mimetypes.add_type("application/wasm", ".wasm")
    reg = core.load_registry()
    assets = core.load_assets(args.tokens, live=args.live_tokens)
    # FORGE_SETUP_TOKEN lets an operator (or the e2e suite) choose the one-time claim token.
    forge = Forge(Path(args.data), reg, assets, setup_token=os.environ.get("FORGE_SETUP_TOKEN") or None)
    server = ThreadingHTTPServer((args.host, args.port), make_handler(Api(forge), secure_cookie=args.secure_cookie))
    print(f"X3 Agent Forge on http://{args.host}:{args.port}  (data: {args.data})", flush=True)
    print(f"Token list: {assets.source}", flush=True)
    if forge.runtime is None:
        print(f"High Command is unclaimed. One-time setup token: {forge.setup_token}", flush=True)
    else:
        print(f"High Command key: {forge.operator_key.hex()}", flush=True)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    return 0


if __name__ == "__main__":
    sys.exit(main())
