"""Comms Rig: connections between an agent and the systems it works with.

A connection is a channel. It grants nothing: what an agent may do through it is set by its
compiled mandate and runtime capabilities. This module only records which connector an owner
plugged in, verifies the setup round trip, checks the target is reachable, and keeps any
secret server-side where no manifest, log or export can include it.
"""
from __future__ import annotations

import ipaddress
import json
import os
import secrets
import threading
import time
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path
from typing import Any, Callable, Dict, List, Optional, Tuple

from .core import ForgeError, Registry, canonical, sha256_hex
import registry as chain_registry

STATE_TTL_S = 600
HEALTH_TIMEOUT_S = 6
DEFAULT_ALLOWED_DOMAINS = ("dotx3.net", "x3star.net", "x3star.org", "nearblocks.io", "chaindefuser.com")
LOCAL_HOSTS = {"localhost", "127.0.0.1", "::1"}
MAX_SECRET = 4096
ACCESS_LOGIN_HOSTS = ("cloudflareaccess.com",)

Transport = Callable[[str], Tuple[int, Dict[str, str]]]


def allowed_domains() -> Tuple[str, ...]:
    extra = [d.strip().lower() for d in os.environ.get("FORGE_EXTRA_DOMAINS", "").split(",") if d.strip()]
    return DEFAULT_ALLOWED_DOMAINS + tuple(extra)


def host_allowed(host: str, *, local: bool = False) -> bool:
    host = (host or "").lower().rstrip(".")
    if local:
        return host in LOCAL_HOSTS
    if not host or host in LOCAL_HOSTS:
        return False
    try:
        ipaddress.ip_address(host)
        return False  # raw IPs are never allowlisted
    except ValueError:
        pass
    return any(host == d or host.endswith("." + d) for d in allowed_domains())


def check_url(url: Any, *, local: bool = False, node: str = "url") -> str:
    if not isinstance(url, str) or len(url) > 512:
        raise ForgeError("X3_FORGE_URL", "URL must be a string of at most 512 characters", node)
    parts = urllib.parse.urlsplit(url)
    scheme_ok = parts.scheme == "https" or (local and parts.scheme == "http")
    if not scheme_ok or not parts.hostname or parts.username or parts.password:
        raise ForgeError("X3_FORGE_URL", f"{url!r} must be an https URL with no credentials in it", node)
    if not host_allowed(parts.hostname, local=local):
        where = "localhost" if local else ", ".join(allowed_domains())
        raise ForgeError("X3_FORGE_URL", f"{parts.hostname} is not on the allowlist ({where}). Add domains with "
                         "FORGE_EXTRA_DOMAINS.", node)
    return url


class _NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):  # noqa: D401 - urllib hook
        return None


def urllib_transport(url: str) -> Tuple[int, Dict[str, str]]:
    opener = urllib.request.build_opener(_NoRedirect)
    req = urllib.request.Request(url, method="GET", headers={"User-Agent": "x3-agent-forge/1 health-check"})
    try:
        with opener.open(req, timeout=HEALTH_TIMEOUT_S) as resp:
            return resp.status, {k.lower(): v for k, v in resp.headers.items()}
    except urllib.error.HTTPError as exc:
        return exc.code, {k.lower(): v for k, v in (exc.headers or {}).items()}


def classify(status: int, headers: Dict[str, str]) -> Tuple[str, str]:
    location = headers.get("location", "")
    loc_host = (urllib.parse.urlsplit(location).hostname or "").lower() if location else ""
    if 200 <= status < 300:
        return "ok", f"HTTP {status}"
    if 300 <= status < 400 and any(loc_host == h or loc_host.endswith("." + h) for h in ACCESS_LOGIN_HOSTS):
        return "login_gated", f"HTTP {status} to Cloudflare Access login ({loc_host})"
    if status in (401, 403):
        return "login_gated", f"HTTP {status}: reachable, sign-in required"
    if 300 <= status < 400:
        return "failed", f"HTTP {status} redirect to {loc_host or 'nowhere'}; not followed"
    return "failed", f"HTTP {status}"


class ConnectionStore:
    """Connection records and setup states, persisted under the Forge data directory."""

    def __init__(self, reg: Registry, data_dir: Path, *, transport: Transport = urllib_transport,
                 clock: Callable[[], float] = time.time):
        self.reg = reg
        self.dir = data_dir / "connections"
        self.secret_dir = data_dir / "secrets"
        self.dir.mkdir(parents=True, exist_ok=True)
        self.secret_dir.mkdir(parents=True, exist_ok=True)
        os.chmod(self.secret_dir, 0o700)
        self.transport = transport
        self.clock = clock
        self.lock = threading.Lock()
        self.states: Dict[str, Dict[str, Any]] = {}

    # -- persistence ---------------------------------------------------------------------
    def _path(self, cid: str) -> Path:
        if not (isinstance(cid, str) and len(cid) == 32 and all(c in "0123456789abcdef" for c in cid)):
            raise ForgeError("X3_FORGE_CONNECTION", "unknown connection", "comms")
        return self.dir / f"{cid}.json"

    def _load(self, cid: str) -> Dict[str, Any]:
        path = self._path(cid)
        if not path.exists():
            raise ForgeError("X3_FORGE_CONNECTION", "unknown connection", "comms")
        return json.loads(path.read_text())

    def _save(self, rec: Dict[str, Any]) -> None:
        tmp = self._path(rec["id"]).with_suffix(".tmp")
        tmp.write_text(json.dumps(rec, sort_keys=True, indent=1))
        tmp.replace(self._path(rec["id"]))

    def get(self, cid: str, session: Optional[str] = None) -> Dict[str, Any]:
        rec = self._load(cid)
        if session is not None and rec["session"] != session and rec.get("agent_id") is None:
            raise ForgeError("X3_FORGE_CONNECTION", "unknown connection", "comms")
        return rec

    @staticmethod
    def public(rec: Dict[str, Any]) -> Dict[str, Any]:
        """What the browser, manifests and exports may see. Never the secret or the session."""
        return {k: rec[k] for k in ("id", "connector", "status", "confirmed_by", "config", "config_hash", "has_secret",
                                    "health", "setup_url", "agent_id", "created_at", "history") if k in rec}

    # -- config ----------------------------------------------------------------------------
    def _config(self, spec: Dict[str, Any], raw: Any) -> Dict[str, Any]:
        raw = raw if isinstance(raw, dict) else {}
        known = {f["id"] for f in spec["config"]}
        unknown = sorted(set(raw) - known)
        if unknown:
            raise ForgeError("X3_FORGE_CONFIG", f"{spec['name']} has no setting {unknown}", f"connector.{spec['id']}")
        out: Dict[str, Any] = {}
        for f in spec["config"]:
            node = f"connector.{spec['id']}.{f['id']}"
            v = raw.get(f["id"])
            if f["type"] == "url":
                out[f["id"]] = check_url(v, local=bool(f.get("local")), node=node)
            elif f["type"] == "near_account":
                if not isinstance(v, str) or not chain_registry.is_valid_near_account(v):
                    raise ForgeError("X3_FORGE_CONFIG", f"{f['label']}: {v!r} is not a NEAR account", node)
                out[f["id"]] = v
            elif f["type"] == "int":
                if isinstance(v, bool) or not isinstance(v, int) or not f["min"] <= v <= f["max"]:
                    raise ForgeError("X3_FORGE_CONFIG", f"{f['label']} must be {f['min']}..{f['max']}", node)
                out[f["id"]] = v
            else:
                v = v if v is not None else []
                if not isinstance(v, list) or len(v) > f["max"] or not all(isinstance(x, str) and 0 < len(x) <= 64
                                                                           for x in v):
                    raise ForgeError("X3_FORGE_CONFIG", f"{f['label']}: up to {f['max']} short entries", node)
                if f["type"] == "choice_list" and any(x not in f["choices"] for x in v):
                    raise ForgeError("X3_FORGE_CONFIG", f"{f['label']}: choose from {f['choices']}", node)
                out[f["id"]] = sorted(set(v))
        return out

    def setup_target(self, spec: Dict[str, Any], config: Dict[str, Any], agent_account: Optional[str]) -> Optional[str]:
        url = spec["setup_url"]
        if url == "forge:self":
            return None
        if url is None:
            return config.get("url")
        if "{agent}" in url:
            if not agent_account or not chain_registry.is_valid_near_account(agent_account):
                raise ForgeError("X3_FORGE_CONFIG", "set the agent's NEAR account first", "origin.agent")
            url = url.replace("{agent}", urllib.parse.quote(agent_account))
        return check_url(url, node=f"connector.{spec['id']}")

    # -- lifecycle -----------------------------------------------------------------------
    def start(self, session: str, connector: str, raw_config: Any, agent_account: Optional[str],
              callback_base: str) -> Dict[str, Any]:
        spec = self.reg.connectors.get(connector)
        if spec is None:
            raise ForgeError("X3_FORGE_CONNECTION", f"unknown connector {connector!r}", f"connector.{connector}")
        config = self._config(spec, raw_config)
        target = self.setup_target(spec, config, agent_account)
        now = self.clock()
        cid = secrets.token_hex(16)
        rec = {"id": cid, "connector": connector, "session": session, "status": "pending", "confirmed_by": None,
               "config": config, "config_hash": sha256_hex(canonical({"connector": connector, "config": config})),
               "has_secret": False, "health": None, "setup_url": target, "agent_id": None,
               "created_at": int(now), "history": [{"at": int(now), "event": "created"}]}
        if spec["setup_url"] == "forge:self":
            rec.update(status="linked", confirmed_by="forge")
            rec["history"].append({"at": int(now), "event": "linked (this Forge)"})
            with self.lock:
                self._save(rec)
            return {"connection": self.public(rec), "open_url": None, "state": None}
        state = secrets.token_urlsafe(24)
        with self.lock:
            self._prune(now)
            self.states[state] = {"session": session, "connection": cid, "expires": now + STATE_TTL_S, "used": False}
            self._save(rec)
        open_url = None
        if spec["setup_url"] is not None:
            ret = f"{callback_base}?connection={cid}"
            sep = "&" if "?" in target else "?"
            open_url = (f"{target}{sep}x3_forge_state={urllib.parse.quote(state)}"
                        f"&x3_forge_return={urllib.parse.quote(ret, safe='')}")
        return {"connection": self.public(rec), "open_url": open_url, "state": state}

    def _prune(self, now: float) -> None:
        for k in [k for k, v in self.states.items() if v["expires"] < now or v["used"]]:
            self.states.pop(k, None)

    def _consume(self, session: str, state: Any, cid: str) -> None:
        now = self.clock()
        if not isinstance(state, str):
            raise ForgeError("X3_FORGE_STATE", "missing setup state", "comms")
        entry = self.states.get(state)
        if entry is None or entry["used"] or entry["expires"] < now:
            raise ForgeError("X3_FORGE_STATE", "setup state is unknown, used or expired; start setup again", "comms")
        if not secrets.compare_digest(entry["session"], session) or entry["connection"] != cid:
            raise ForgeError("X3_FORGE_STATE", "setup state does not belong to this session and connection", "comms")
        entry["used"] = True

    def callback(self, session: str, state: Any, cid: str) -> Dict[str, Any]:
        """The connector page sent the owner back. Only a matching, unused state links it."""
        with self.lock:
            rec = self.get(cid, session)
            self._consume(session, state, cid)
            rec.update(status="linked", confirmed_by="page")
            rec["history"].append({"at": int(self.clock()), "event": "linked (setup page returned)"})
            self._save(rec)
        return self.public(rec)

    def confirm(self, session: str, state: Any, cid: str) -> Dict[str, Any]:
        """The page has no return callback yet: the owner says setup is done. Labelled as such."""
        with self.lock:
            rec = self.get(cid, session)
            self._consume(session, state, cid)
            rec.update(status="linked", confirmed_by="owner")
            rec["history"].append({"at": int(self.clock()), "event": "linked (owner confirmed setup)"})
            self._save(rec)
        return self.public(rec)

    def set_secret(self, session: str, cid: str, secret: Any) -> Dict[str, Any]:
        with self.lock:
            rec = self.get(cid, session)
            spec = self.reg.connectors[rec["connector"]]
            if spec["auth"] != "api_key":
                raise ForgeError("X3_FORGE_SECRET", f"{spec['name']} does not take a key", f"connector.{spec['id']}")
            if rec.get("agent_id"):
                raise ForgeError("X3_FORGE_SECRET", "connection is bound to a born agent", f"connector.{spec['id']}")
            if not isinstance(secret, str) or not 8 <= len(secret) <= MAX_SECRET or any(ord(c) < 32 for c in secret):
                raise ForgeError("X3_FORGE_SECRET", "key must be 8..4096 printable characters", f"connector.{spec['id']}")
            path = self.secret_dir / rec["id"]
            fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
            with os.fdopen(fd, "w") as f:
                f.write(secret)
            rec["has_secret"] = True
            rec["history"].append({"at": int(self.clock()), "event": "key stored server-side"})
            if spec["setup_url"] is None and rec["status"] == "pending":
                rec.update(status="linked", confirmed_by="owner")
            self._save(rec)
        return self.public(rec)

    def health(self, session: Optional[str], cid: str, *, self_ok: bool = True) -> Dict[str, Any]:
        rec = self.get(cid, session)
        spec = self.reg.connectors[rec["connector"]]
        now = int(self.clock())
        if spec["setup_url"] == "forge:self":
            result = {"status": "ok" if self_ok else "failed", "detail": "this Forge's own mesh", "checked_at": now}
        else:
            target = rec["setup_url"]
            try:
                check_url(target, local=any(f.get("local") for f in spec["config"] if f["type"] == "url"))
                status, headers = self.transport(target)
                state, detail = classify(status, headers)
                result = {"status": state, "detail": detail, "http_status": status, "checked_at": now,
                          "target": target}
            except ForgeError as exc:
                result = {"status": "failed", "detail": exc.message, "checked_at": now, "target": target}
            except Exception as exc:  # network failure: report it, never pass
                result = {"status": "failed", "detail": f"unreachable: {type(exc).__name__}: {exc}"[:240],
                          "checked_at": now, "target": target}
        with self.lock:
            rec = self._load(cid)
            rec["health"] = result
            if result["status"] == "failed" and rec["status"] == "linked":
                rec["status"] = "failed"
            elif result["status"] != "failed" and rec["status"] == "failed" and rec["confirmed_by"]:
                rec["status"] = "linked"
            rec["history"].append({"at": now, "event": f"health {result['status']}: {result['detail']}"})
            rec["history"] = rec["history"][-50:]
            self._save(rec)
        return self.public(rec)

    def usable(self, rec: Dict[str, Any]) -> Tuple[bool, str]:
        spec = self.reg.connectors[rec["connector"]]
        if rec["status"] == "revoked":
            return False, "revoked"
        if rec["status"] != "linked":
            return False, f"status is {rec['status']}"
        if spec["auth"] == "api_key" and not rec["has_secret"]:
            return False, "needs its key"
        health = rec.get("health") or {}
        if health.get("status") not in ("ok", "login_gated"):
            return False, "health check has not passed"
        return True, "ok"

    def bind(self, session: str, ids: List[str], agent_id: str) -> List[Dict[str, Any]]:
        with self.lock:
            out = []
            for cid in ids:
                rec = self.get(cid, session)
                rec["agent_id"] = agent_id
                rec["history"].append({"at": int(self.clock()), "event": f"bound to {agent_id}"})
                self._save(rec)
                out.append(self.public(rec))
            return out

    def revoke(self, cid: str, agent_id: str) -> Dict[str, Any]:
        with self.lock:
            rec = self._load(cid)
            if rec.get("agent_id") != agent_id:
                raise ForgeError("X3_FORGE_CONNECTION", "connection does not belong to this agent", "comms")
            rec["status"] = "revoked"
            rec["history"].append({"at": int(self.clock()), "event": "revoked by owner"})
            self._save(rec)
            secret = self.secret_dir / cid
            if secret.exists():
                secret.unlink()
                rec["has_secret"] = False
                self._save(rec)
            return self.public(rec)
