"""Wire format for the X3 GPU contributor mesh.

Every message a worker sends is an Ed25519-signed envelope over canonical JSON:

    {"protocol": "x3.gpu-mesh.v1", "kind": "...", "worker": <pubkey hex>, "ts": <unix s>,
     "nonce": <32 hex>, "payload": {...}, "sig": <128 hex>}

The signature covers every field except ``sig``. The coordinator rejects an envelope whose
kind is not the one the endpoint expects, whose timestamp is outside the allowed clock skew,
or whose nonce it has already seen from that worker, so a captured poll, heartbeat or result
cannot be replayed.

Model output is never trusted because it is signed. A signature says *which* contributor
produced the bytes; the compiler, the owner's mandate and Guard still decide whether anything
the model wrote is allowed to happen.
"""

from __future__ import annotations

import hashlib
import json
import os
import re
import secrets
import time
from typing import Any, Dict, List, Optional, Tuple

from cryptography.exceptions import InvalidSignature
from cryptography.hazmat.primitives import serialization
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey, Ed25519PublicKey

PROTOCOL = "x3.gpu-mesh.v1"
MAX_BODY_BYTES = 256 * 1024
MAX_CLOCK_SKEW_S = 120
MAX_MESSAGES = 64
MAX_MESSAGE_CHARS = 64 * 1024
MAX_REQUEST_CHARS = 128 * 1024
MAX_RESPONSE_CHARS = 64 * 1024
MAX_TOKENS_CAP = 8192
DEFAULT_MAX_TOKENS = 1024
ROLES = ("system", "user", "assistant")

_HEX64 = re.compile(r"^[0-9a-f]{64}$")
_HEX128 = re.compile(r"^[0-9a-f]{128}$")
_HEX32 = re.compile(r"^[0-9a-f]{32}$")
_MODEL_NAME = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._:/@+-]{0,127}$")
_DIGEST = re.compile(r"^(sha256:)?[0-9a-f]{12,64}$")
_LABEL = re.compile(r"^[A-Za-z0-9][A-Za-z0-9 ._@+-]{0,63}$")
_NEAR_ACCOUNT = re.compile(r"^(?=.{2,64}$)([a-z0-9]+[-_])*[a-z0-9]+(\.([a-z0-9]+[-_])*[a-z0-9]+)*$")


class ProtocolError(ValueError):
    """A message that must be refused. ``status`` is the HTTP status to answer with."""

    def __init__(self, message: str, status: int = 400, code: str = "invalid_request"):
        super().__init__(message)
        self.status = status
        self.code = code


def canonical(obj: Any) -> bytes:
    """Deterministic JSON: sorted keys, no whitespace, UTF-8. Floats are refused."""
    _reject_floats(obj)
    return json.dumps(obj, sort_keys=True, separators=(",", ":"), ensure_ascii=False,
                      allow_nan=False).encode("utf-8")


def _reject_floats(obj: Any) -> None:
    if isinstance(obj, float):
        raise ProtocolError("floats are not allowed in signed messages; use integers")
    if isinstance(obj, dict):
        for value in obj.values():
            _reject_floats(value)
    elif isinstance(obj, (list, tuple)):
        for value in obj:
            _reject_floats(value)


def sha256_hex(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def now_s() -> int:
    return int(time.time())


def now_ms() -> int:
    return int(time.time() * 1000)


# --------------------------------------------------------------------------------------------
# Keys
# --------------------------------------------------------------------------------------------

class Identity:
    """An Ed25519 key pair. The private key never leaves the machine that generated it."""

    def __init__(self, private_key: Ed25519PrivateKey):
        self._key = private_key
        raw = private_key.public_key().public_bytes(serialization.Encoding.Raw, serialization.PublicFormat.Raw)
        self.public_hex = raw.hex()

    @classmethod
    def generate(cls) -> "Identity":
        return cls(Ed25519PrivateKey.generate())

    @classmethod
    def load(cls, path: str) -> "Identity":
        with open(path, "r", encoding="utf-8") as fh:
            data = json.load(fh)
        if data.get("protocol") != PROTOCOL or not isinstance(data.get("secret_key"), str):
            raise ProtocolError(f"{path} is not an {PROTOCOL} key file", status=500, code="bad_key_file")
        seed = bytes.fromhex(data["secret_key"])
        if len(seed) != 32:
            raise ProtocolError(f"{path} has a malformed secret key", status=500, code="bad_key_file")
        identity = cls(Ed25519PrivateKey.from_private_bytes(seed))
        if data.get("public_key") and data["public_key"] != identity.public_hex:
            raise ProtocolError(f"{path}: public_key does not match secret_key", status=500, code="bad_key_file")
        return identity

    @classmethod
    def load_or_create(cls, path: str) -> Tuple["Identity", bool]:
        if os.path.exists(path):
            return cls.load(path), False
        identity = cls.generate()
        identity.save(path)
        return identity, True

    def save(self, path: str) -> None:
        """Write the key with mode 0600. Refuses to overwrite an existing key file."""
        directory = os.path.dirname(os.path.abspath(path))
        os.makedirs(directory, mode=0o700, exist_ok=True)
        seed = self._key.private_bytes(serialization.Encoding.Raw, serialization.PrivateFormat.Raw,
                                       serialization.NoEncryption())
        body = json.dumps({"protocol": PROTOCOL, "public_key": self.public_hex, "secret_key": seed.hex()}, indent=2)
        fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(fd, "w", encoding="utf-8") as fh:
            fh.write(body + "\n")

    def sign(self, message: bytes) -> str:
        return self._key.sign(message).hex()


def verify_signature(public_hex: str, message: bytes, signature_hex: str) -> bool:
    if not (isinstance(public_hex, str) and _HEX64.match(public_hex)):
        return False
    if not (isinstance(signature_hex, str) and _HEX128.match(signature_hex)):
        return False
    try:
        Ed25519PublicKey.from_public_bytes(bytes.fromhex(public_hex)).verify(bytes.fromhex(signature_hex), message)
        return True
    except (InvalidSignature, ValueError):
        return False


def is_pubkey(value: Any) -> bool:
    return isinstance(value, str) and bool(_HEX64.match(value))


# --------------------------------------------------------------------------------------------
# Envelopes
# --------------------------------------------------------------------------------------------

def seal(identity: Identity, kind: str, payload: Dict[str, Any], *, ts: Optional[int] = None,
         nonce: Optional[str] = None) -> Dict[str, Any]:
    envelope = {
        "protocol": PROTOCOL,
        "kind": kind,
        "worker": identity.public_hex,
        "ts": now_s() if ts is None else ts,
        "nonce": nonce or secrets.token_hex(16),
        "payload": payload,
    }
    envelope["sig"] = identity.sign(canonical(envelope))
    return envelope


def open_envelope(envelope: Any, kind: str, *, now: Optional[int] = None,
                  max_skew: int = MAX_CLOCK_SKEW_S) -> Tuple[str, str, int, Dict[str, Any]]:
    """Check shape, kind, freshness and signature. Returns (worker, nonce, ts, payload).

    The caller must still record the nonce (``Store.consume_nonce``) to refuse replays.
    """
    if not isinstance(envelope, dict):
        raise ProtocolError("expected a signed envelope object")
    expected = {"protocol", "kind", "worker", "ts", "nonce", "payload", "sig"}
    if set(envelope) != expected:
        raise ProtocolError("envelope fields must be exactly: " + ", ".join(sorted(expected)))
    if envelope["protocol"] != PROTOCOL:
        raise ProtocolError(f"unsupported protocol {envelope['protocol']!r}")
    if envelope["kind"] != kind:
        raise ProtocolError(f"expected a {kind!r} envelope, got {envelope['kind']!r}")
    worker, nonce, ts, payload = envelope["worker"], envelope["nonce"], envelope["ts"], envelope["payload"]
    if not is_pubkey(worker):
        raise ProtocolError("worker must be a 64-hex Ed25519 public key")
    if not (isinstance(nonce, str) and _HEX32.match(nonce)):
        raise ProtocolError("nonce must be 32 lowercase hex characters")
    if not isinstance(ts, int) or isinstance(ts, bool):
        raise ProtocolError("ts must be an integer unix timestamp")
    if not isinstance(payload, dict):
        raise ProtocolError("payload must be an object")
    current = now_s() if now is None else now
    if abs(current - ts) > max_skew:
        raise ProtocolError(f"envelope timestamp is {current - ts}s from coordinator time "
                            f"(allowed ±{max_skew}s); check the worker clock", status=401, code="stale")
    unsigned = {k: v for k, v in envelope.items() if k != "sig"}
    if not verify_signature(worker, canonical(unsigned), envelope["sig"]):
        raise ProtocolError("bad signature", status=401, code="bad_signature")
    return worker, nonce, ts, payload


# --------------------------------------------------------------------------------------------
# Inference requests and responses
# --------------------------------------------------------------------------------------------

_ALLOWED_REQUEST_FIELDS = {"model", "messages", "temperature", "max_tokens", "seed", "top_p", "stop", "stream",
                           "stream_options", "user", "n"}
_REFUSED_REQUEST_FIELDS = {"tools", "tool_choice", "functions", "function_call", "response_format", "logit_bias",
                           "logprobs", "top_logprobs"}


def _milli(value: Any, name: str, lo: int, hi: int) -> int:
    """Convert a float parameter (0.0–2.0) to integer thousandths so it can be signed."""
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise ProtocolError(f"{name} must be a number")
    milli = int(round(float(value) * 1000))
    if milli < lo or milli > hi:
        raise ProtocolError(f"{name} must be between {lo / 1000} and {hi / 1000}")
    return milli


def normalize_chat_request(body: Any, *, max_tokens_cap: int = MAX_TOKENS_CAP) -> Tuple[Dict[str, Any], bool]:
    """Turn an OpenAI-style chat request into the canonical job request the worker runs.

    Returns (request, stream). Only plain text chat is accepted: tool calling, function calling,
    structured-output modes and logprobs are refused rather than silently dropped, because a
    caller that relies on them would otherwise get a different contract than it asked for.
    """
    if not isinstance(body, dict):
        raise ProtocolError("request body must be a JSON object")
    refused = sorted(set(body) & _REFUSED_REQUEST_FIELDS)
    if refused:
        raise ProtocolError(f"not supported by the X3 GPU mesh: {', '.join(refused)}", code="unsupported_parameter")
    unknown = sorted(set(body) - _ALLOWED_REQUEST_FIELDS)
    if unknown:
        raise ProtocolError(f"unknown request fields: {', '.join(unknown)}", code="unsupported_parameter")
    model = body.get("model")
    if not (isinstance(model, str) and _MODEL_NAME.match(model)):
        raise ProtocolError("model must be a model name such as qwen3:8b")
    if body.get("n", 1) != 1:
        raise ProtocolError("only n=1 is supported", code="unsupported_parameter")
    stream = body.get("stream", False)
    if not isinstance(stream, bool):
        raise ProtocolError("stream must be true or false")
    messages = body.get("messages")
    if not isinstance(messages, list) or not messages:
        raise ProtocolError("messages must be a non-empty list")
    if len(messages) > MAX_MESSAGES:
        raise ProtocolError(f"at most {MAX_MESSAGES} messages")
    clean: List[Dict[str, str]] = []
    total = 0
    for i, message in enumerate(messages):
        if not isinstance(message, dict) or set(message) - {"role", "content", "name"}:
            raise ProtocolError(f"messages[{i}] must have only role and content")
        role, content = message.get("role"), message.get("content")
        if role not in ROLES:
            raise ProtocolError(f"messages[{i}].role must be one of {', '.join(ROLES)}")
        if isinstance(content, list):
            parts = []
            for part in content:
                if not (isinstance(part, dict) and part.get("type") == "text" and isinstance(part.get("text"), str)):
                    raise ProtocolError(f"messages[{i}].content: only text parts are supported",
                                        code="unsupported_parameter")
                parts.append(part["text"])
            content = "".join(parts)
        if not isinstance(content, str):
            raise ProtocolError(f"messages[{i}].content must be text")
        if len(content) > MAX_MESSAGE_CHARS:
            raise ProtocolError(f"messages[{i}] is longer than {MAX_MESSAGE_CHARS} characters", status=413)
        total += len(content)
        clean.append({"role": role, "content": content})
    if total > MAX_REQUEST_CHARS:
        raise ProtocolError(f"messages total more than {MAX_REQUEST_CHARS} characters", status=413)
    max_tokens = body.get("max_tokens", DEFAULT_MAX_TOKENS)
    if isinstance(max_tokens, bool) or not isinstance(max_tokens, int) or max_tokens < 1:
        raise ProtocolError("max_tokens must be a positive integer")
    request: Dict[str, Any] = {
        "model": model,
        "messages": clean,
        "max_tokens": min(max_tokens, max_tokens_cap),
        "temperature_milli": _milli(body.get("temperature", 0.7), "temperature", 0, 2000),
        "top_p_milli": _milli(body.get("top_p", 1.0), "top_p", 1, 1000),
    }
    if "seed" in body and body["seed"] is not None:
        seed = body["seed"]
        if isinstance(seed, bool) or not isinstance(seed, int) or not 0 <= seed < 2 ** 63:
            raise ProtocolError("seed must be a non-negative integer")
        request["seed"] = seed
    stop = body.get("stop")
    if stop is not None:
        stops = [stop] if isinstance(stop, str) else stop
        if not (isinstance(stops, list) and len(stops) <= 4 and all(isinstance(s, str) and 0 < len(s) <= 64
                                                                     for s in stops)):
            raise ProtocolError("stop must be a string or up to 4 strings of at most 64 characters")
        request["stop"] = stops
    return request, stream


def validate_job_request(request: Any) -> Dict[str, Any]:
    """Re-check a canonical request (worker side): the worker never trusts the coordinator either."""
    if not isinstance(request, dict):
        raise ProtocolError("job request must be an object")
    allowed = {"model", "messages", "max_tokens", "temperature_milli", "top_p_milli", "seed", "stop"}
    if set(request) - allowed or not {"model", "messages", "max_tokens", "temperature_milli",
                                      "top_p_milli"} <= set(request):
        raise ProtocolError("job request has unexpected or missing fields")
    body = {
        "model": request["model"],
        "messages": request["messages"],
        "max_tokens": request["max_tokens"],
        "temperature": request["temperature_milli"] / 1000 if isinstance(request["temperature_milli"], int) else None,
        "top_p": request["top_p_milli"] / 1000 if isinstance(request["top_p_milli"], int) else None,
    }
    if "seed" in request:
        body["seed"] = request["seed"]
    if "stop" in request:
        body["stop"] = request["stop"]
    again, _ = normalize_chat_request(body)
    if again != request:
        raise ProtocolError("job request is not in canonical form")
    return request


def validate_response(response: Any) -> Dict[str, Any]:
    """Shape of what a worker returns for a job (before signing, and when verifying)."""
    if not isinstance(response, dict) or set(response) != {"content", "finish_reason", "model", "usage"}:
        raise ProtocolError("response must have exactly content, finish_reason, model and usage")
    content, finish, model, usage = (response["content"], response["finish_reason"], response["model"],
                                     response["usage"])
    if not isinstance(content, str) or len(content) > MAX_RESPONSE_CHARS:
        raise ProtocolError(f"response content must be text of at most {MAX_RESPONSE_CHARS} characters")
    if finish not in ("stop", "length"):
        raise ProtocolError("finish_reason must be stop or length")
    if not (isinstance(model, str) and _MODEL_NAME.match(model)):
        raise ProtocolError("response model is malformed")
    if not isinstance(usage, dict) or set(usage) != {"prompt_tokens", "completion_tokens"}:
        raise ProtocolError("usage must have exactly prompt_tokens and completion_tokens")
    for key, value in usage.items():
        if isinstance(value, bool) or not isinstance(value, int) or not 0 <= value <= 10_000_000:
            raise ProtocolError(f"usage.{key} must be a non-negative integer")
    return response


# --------------------------------------------------------------------------------------------
# Worker self-description (all self-reported; the coordinator labels it so)
# --------------------------------------------------------------------------------------------

def validate_label(label: Any) -> str:
    if not (isinstance(label, str) and _LABEL.match(label)):
        raise ProtocolError("label must be 1–64 letters, digits, spaces or ._@+-")
    return label


def validate_payout_account(account: Any) -> Optional[str]:
    if account in (None, ""):
        return None
    if not (isinstance(account, str) and _NEAR_ACCOUNT.match(account)):
        raise ProtocolError("payout_account must be a NEAR account id")
    return account


def validate_gpus(gpus: Any) -> List[Dict[str, Any]]:
    if not isinstance(gpus, list) or len(gpus) > 16:
        raise ProtocolError("gpus must be a list of at most 16 entries")
    clean = []
    for i, gpu in enumerate(gpus):
        if not isinstance(gpu, dict):
            raise ProtocolError(f"gpus[{i}] must be an object")
        name = gpu.get("name")
        vendor = gpu.get("vendor")
        vram = gpu.get("vram_mb")
        if not (isinstance(name, str) and 0 < len(name) <= 96):
            raise ProtocolError(f"gpus[{i}].name must be 1–96 characters")
        if vendor not in ("nvidia", "amd", "intel", "apple", "other"):
            raise ProtocolError(f"gpus[{i}].vendor must be nvidia, amd, intel, apple or other")
        if isinstance(vram, bool) or not isinstance(vram, int) or not 0 <= vram <= 4_000_000:
            raise ProtocolError(f"gpus[{i}].vram_mb must be a non-negative integer")
        entry = {"name": name, "vendor": vendor, "vram_mb": vram}
        for key in ("util_pct", "temp_c", "mem_used_mb"):
            value = gpu.get(key)
            if value is not None:
                if isinstance(value, bool) or not isinstance(value, int) or not 0 <= value <= 4_000_000:
                    raise ProtocolError(f"gpus[{i}].{key} must be a non-negative integer")
                entry[key] = value
        clean.append(entry)
    return clean


def validate_models(models: Any) -> List[Dict[str, str]]:
    if not isinstance(models, list) or not 0 < len(models) <= 64:
        raise ProtocolError("models must list 1–64 models")
    clean, seen = [], set()
    for i, model in enumerate(models):
        if not isinstance(model, dict) or set(model) != {"name", "digest"}:
            raise ProtocolError(f"models[{i}] must have exactly name and digest")
        name, digest = model["name"], model["digest"]
        if not (isinstance(name, str) and _MODEL_NAME.match(name)):
            raise ProtocolError(f"models[{i}].name is malformed")
        if not (isinstance(digest, str) and _DIGEST.match(digest)):
            raise ProtocolError(f"models[{i}].digest must be a hex model digest")
        if name in seen:
            raise ProtocolError(f"models[{i}]: {name} listed twice")
        seen.add(name)
        clean.append({"name": name, "digest": digest})
    return clean
