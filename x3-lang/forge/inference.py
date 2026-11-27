"""Forge inference gate: a born agent thinks on the X3 GPU mesh only inside what it was born with.

Every request is signed by the agent's own key. Before anything reaches the mesh the gate checks,
fail-closed and with every reason recorded:

- the signature, the expiry window and that the nonce was never used;
- that the agent is active in the Agent Law runtime (a revoked or expired agent stops thinking);
- that its signed manifest holds the `ai.infer` capability and the `compute.gpu_mesh` surface;
- that its GPU-mesh connection is linked, healthy and not revoked;
- that the model is on the agent's allowlist and `max_tokens` is within its per-call cap;
- that the request fits its rolling 24-hour token budget, counting requests still in flight.

The mesh coordinator then applies its own app-key checks (fleet-only vs public, its own budget).
Model output is untrusted: the runner hands it to the compiler and the agent's mandate, which
decide. This module never signs a transaction.
"""
from __future__ import annotations

import json
import re
import time
from typing import Any, Callable, Dict, List, Optional, Tuple

import near_intents as ni

from .core import ForgeError, canonical, sha256_hex

INFER_DOMAIN = "X3/FORGE/INFER/1"
RUN_DOMAIN = "X3/FORGE/RUN/1"
MESH_SURFACE = "compute.gpu_mesh"
INFER_CAPABILITY = "ai.infer"
MESH_CONNECTOR = "gpu_mesh"
WINDOW_S = 86_400
TTL_S = 300
MESH_TIMEOUT_S = 180.0
MAX_MESSAGES = 16
MAX_MESSAGE_CHARS = 16_384
MODEL_RE = re.compile(r"[A-Za-z0-9._:/-]{1,64}")

Transport = Callable[[str, str, Dict[str, str], Optional[bytes], float], Tuple[int, bytes]]


def payload(domain: str, runtime_id: str, agent_id: str, nonce: str, expires_at: int, body: Dict[str, Any]) -> bytes:
    """The exact bytes an agent signs. `body` is hashed so long prompts never need re-encoding."""
    return canonical({"domain": domain, "runtime_id": runtime_id, "agent_id": agent_id, "nonce": nonce,
                      "expires_at": expires_at, "body_sha256": sha256_hex(canonical(body))})


def mind_params(record: Dict[str, Any]) -> Optional[Dict[str, Any]]:
    return record["compile"]["normalized"]["skills"].get("mesh_mind")


def tokens_used(record: Dict[str, Any], now: float) -> int:
    """Tokens charged in the rolling window. A request still running, or stranded by a host crash,
    is charged its whole max_tokens: the persisted entry is the reservation, so it fails closed."""
    total = 0
    for e in record.get("infer_log", []):
        if e.get("at", 0) > now - WINDOW_S:
            total += int(e.get("max_tokens", 0)) if e.get("verdict") == "running" else int(e.get("tokens", 0))
    return total


def validate_messages(messages: Any) -> List[Dict[str, str]]:
    if not isinstance(messages, list) or not 1 <= len(messages) <= MAX_MESSAGES:
        raise ForgeError("X3_FORGE_INFER", f"messages: 1..{MAX_MESSAGES} chat messages", "infer")
    out = []
    for m in messages:
        if not isinstance(m, dict) or set(m) - {"role", "content"} or m.get("role") not in ("system", "user", "assistant") \
                or not isinstance(m.get("content"), str) or len(m["content"]) > MAX_MESSAGE_CHARS:
            raise ForgeError("X3_FORGE_INFER", "each message is {role: system|user|assistant, content: text}", "infer")
        out.append({"role": m["role"], "content": m["content"]})
    return out


def decide(*, record: Dict[str, Any], live: Dict[str, Any], link: Optional[Dict[str, Any]], link_usable: Tuple[bool, str],
           model: str, max_tokens: int, now: float, reserved: int, signature_ok: bool, nonce: str,
           expires_at: int) -> List[str]:
    """Every reason this request may not reach the mesh. Empty means allowed. Pure: no I/O."""
    reasons: List[str] = []
    if not signature_ok:
        reasons.append("signature")
    if expires_at < now or expires_at > now + TTL_S:
        reasons.append("expiry")
    if nonce in {e.get("nonce") for e in record.get("infer_log", [])}:
        reasons.append("replay")
    if live["status"] != "active":
        reasons.append(f"agent_{live['status']}")
    manifest = live["runtime"]["manifest"]
    if INFER_CAPABILITY not in manifest["capabilities"]:
        reasons.append("capability")
    if MESH_SURFACE not in manifest["surfaces"]:
        reasons.append("surface")
    params = mind_params(record)
    if link is None:
        reasons.append("no_mesh_connection")
    else:
        ok, why = link_usable
        if not ok:
            reasons.append(f"connection_{why.replace(' ', '_')}")
    if params is None:
        reasons.append("no_mesh_mind_skill")
    else:
        if model not in params["models"]:
            reasons.append("model_not_allowed")
        if max_tokens > params["max_tokens"]:
            reasons.append("max_tokens_over_cap")
        if tokens_used(record, now) + reserved + max_tokens > params["daily_tokens"]:
            reasons.append("budget_exhausted")
    return reasons


class MeshClient:
    """POSTs one chat completion to the coordinator with the Forge's app key. No redirects."""

    def __init__(self, base_url: str, app_key: str, transport: Transport = ni.urllib_transport,
                 timeout: float = MESH_TIMEOUT_S):
        self.base_url = base_url.rstrip("/")
        self.app_key = app_key
        self.transport = transport
        self.timeout = timeout

    def complete(self, model: str, messages: List[Dict[str, str]], max_tokens: int) -> Dict[str, Any]:
        body = json.dumps({"model": model, "messages": messages, "max_tokens": max_tokens, "temperature": 0}).encode()
        try:
            status, raw = self.transport("POST", f"{self.base_url}/v1/chat/completions",
                                         {"content-type": "application/json", "authorization": f"Bearer {self.app_key}"},
                                         body, self.timeout)
        except (OSError, ni.NearIntentsError) as exc:
            raise ForgeError("X3_FORGE_MESH_UNREACHABLE", f"GPU mesh not reachable: {getattr(exc, 'message', exc)}",
                             "infer") from exc
        try:
            data = json.loads(raw.decode("utf-8")) if raw else None
        except (UnicodeDecodeError, json.JSONDecodeError):
            data = None
        if status != 200 or not isinstance(data, dict):
            err = (data or {}).get("error") if isinstance(data, dict) else None
            msg = err.get("message") if isinstance(err, dict) else None
            code = err.get("code") if isinstance(err, dict) else None
            raise ForgeError("X3_FORGE_MESH", f"GPU mesh answered {status}: {msg or 'no JSON'}", "infer",
                             {"status": status, "code": code})
        return data


class GateModel:
    """The `model` that `near_agent.run` calls. One call, already authorized by the agent's signed run."""

    provider = "x3-gpu-mesh"

    def __init__(self, call: Callable[[List[Dict[str, str]]], Dict[str, Any]], model: str):
        self.call = call
        self.model = model

    def complete(self, system: str, user: str) -> Dict[str, Any]:
        data = self.call([{"role": "system", "content": system}, {"role": "user", "content": user}])
        choices = data.get("choices") or [{}]
        text = ((choices[0] or {}).get("message") or {}).get("content") or ""
        return {"text": text, "provider": self.provider, "model": data.get("model", self.model), "id": data.get("id"),
                "stop_reason": (choices[0] or {}).get("finish_reason"), "usage": data.get("usage")}


def usage_tokens(data: Dict[str, Any], fallback: int) -> int:
    usage = data.get("usage") if isinstance(data, dict) else None
    total = usage.get("total_tokens") if isinstance(usage, dict) else None
    return int(total) if isinstance(total, int) and total >= 0 else fallback


def now_s() -> float:
    return time.time()
