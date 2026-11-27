"""Run a born Forge agent headless: `python -m forge.agent_cli run --agent vexa.x3-owner.near --key vexa.agent-key.jwk.json "Pay ..."`.

The key is the agent key exported from its profile page (JWK). The CLI asks the Forge for the exact
payload, signs it locally with the agent key, and posts the signature: the key never leaves this
machine and the Forge gate decides. Against forge.dotx3.net (Cloudflare Access), pass the owner's
Access token in FORGE_ACCESS_JWT; locally (FORGE_ACCESS_MODE=local) nothing extra is needed."""
from __future__ import annotations

import argparse
import base64
import json
import os
import secrets
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path
from typing import Any, Dict, List, Optional

from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey

DEFAULT_URL = "http://127.0.0.1:7420"


class CliError(Exception):
    pass


def load_key(path: Path) -> Ed25519PrivateKey:
    try:
        jwk = json.loads(path.read_text())
    except (OSError, ValueError) as exc:
        raise CliError(f"cannot read agent key {path}: {exc}") from exc
    if not isinstance(jwk, dict) or jwk.get("kty") != "OKP" or jwk.get("crv") != "Ed25519" or not isinstance(jwk.get("d"), str):
        raise CliError("agent key must be an Ed25519 private JWK (kty OKP, crv Ed25519, d)")
    d = jwk["d"]
    raw = base64.urlsafe_b64decode(d + "=" * (-len(d) % 4))
    if len(raw) != 32:
        raise CliError("agent key: d must be 32 bytes")
    return Ed25519PrivateKey.from_private_bytes(raw)


class ForgeClient:
    def __init__(self, base: str, access_jwt: Optional[str] = None, timeout: float = 240):
        self.base, self.access_jwt, self.timeout = base.rstrip("/"), access_jwt, timeout

    def post(self, path: str, body: Dict[str, Any]) -> Dict[str, Any]:
        headers = {"Content-Type": "application/json", "Accept": "application/json"}
        if self.access_jwt:
            headers["Cf-Access-Token"] = self.access_jwt
            headers["Cookie"] = f"CF_Authorization={self.access_jwt}"
        req = urllib.request.Request(self.base + path, data=json.dumps(body).encode(), headers=headers, method="POST")
        try:
            with urllib.request.urlopen(req, timeout=self.timeout) as resp:
                return json.loads(resp.read())
        except urllib.error.HTTPError as exc:
            try:
                detail = json.loads(exc.read())
            except ValueError:
                detail = {"status": exc.code}
            raise CliError(f"{path}: HTTP {exc.code}: {json.dumps(detail)}") from exc
        except (urllib.error.URLError, OSError, ValueError) as exc:
            raise CliError(f"{path}: {exc}") from exc


def signed(client: ForgeClient, key: Ed25519PrivateKey, agent: str, kind: str, body: Dict[str, Any],
           ttl: int = 120) -> Dict[str, Any]:
    body = dict(body, nonce=secrets.token_hex(16), expires_at=int(time.time()) + ttl)
    payload = bytes.fromhex(client.post(f"/api/agents/{agent}/{kind}/payload", body)["payload_hex"])
    return client.post(f"/api/agents/{agent}/{kind}", dict(body, signature=key.sign(payload).hex()))


def render(out: Dict[str, Any]) -> List[str]:
    lines = [f"gate: {out.get('verdict')}" + (f" ({', '.join(out['reasons'])})" if out.get("reasons") else "")]
    if out.get("error"):
        lines.append(f"error: {out['error'].get('code')}: {out['error'].get('message')}")
    budget = out.get("budget") or {}
    if budget:
        lines.append(f"budget: {budget.get('used_24h')}/{budget.get('daily_tokens')} tokens in 24h")
    report = out.get("result") or {}
    if report.get("status"):
        lines.append(f"result: {report['status']}")
    for st in report.get("stages") or []:
        lines.append(f"  {'pass' if st.get('ok') else 'FAIL'}  {st.get('stage')}")
    if (report.get("program") or {}).get("source"):
        lines += ["--- proposed .x3 (untrusted until the mandate passes it) ---", report["program"]["source"].rstrip()]
    if report.get("text"):
        lines += ["--- model text (untrusted) ---", report["text"].rstrip()]
    if report.get("note"):
        lines.append(report["note"])
    if report.get("approve_with"):
        lines.append(f"owner approval would sign {report['approve_with']} (not sent)")
    return lines


def main(argv: Optional[List[str]] = None) -> int:
    ap = argparse.ArgumentParser(prog="forge.agent_cli", description=__doc__.split("\n")[0])
    ap.add_argument("--url", default=os.environ.get("FORGE_URL", DEFAULT_URL))
    ap.add_argument("--agent", required=True, help="agent NEAR account id")
    ap.add_argument("--key", required=True, type=Path, help="agent key JWK exported from the profile page")
    ap.add_argument("--model", required=True)
    ap.add_argument("--max-tokens", type=int, default=1500)
    ap.add_argument("--json", action="store_true", help="print the raw gate response")
    sub = ap.add_subparsers(dest="cmd", required=True)
    r = sub.add_parser("run", help="task -> mesh -> .x3 -> compile -> mandate check (dry)")
    r.add_argument("task")
    i = sub.add_parser("infer", help="one raw chat completion (untrusted text)")
    i.add_argument("prompt")
    args = ap.parse_args(argv)
    try:
        key = load_key(args.key)
        client = ForgeClient(args.url, os.environ.get("FORGE_ACCESS_JWT") or None)
        if args.cmd == "run":
            out = signed(client, key, args.agent, "run",
                         {"model": args.model, "task": args.task, "max_tokens": args.max_tokens})
        else:
            out = signed(client, key, args.agent, "infer",
                         {"model": args.model, "max_tokens": args.max_tokens,
                          "messages": [{"role": "user", "content": args.prompt}]})
    except CliError as exc:
        print(f"forge.agent_cli: {exc}", file=sys.stderr)
        return 2
    print(json.dumps(out, indent=2) if args.json else "\n".join(render(out)))
    if out.get("verdict") != "answered":
        return 1
    status = (out.get("result") or {}).get("status")
    return 0 if status in (None, "allowed", "answered") else 1


if __name__ == "__main__":
    raise SystemExit(main())
