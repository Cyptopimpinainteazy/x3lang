"""Local bounded agent-tool ingress. No network listener or OS sandbox.

Trusted service owns Runtime and the stamp issuer. Untrusted callers submit a
signed Action envelope, never adapter callbacks, stamp claims, or issuer keys.
External tools MUST be inaccessible except through the trusted service process.
"""
from __future__ import annotations

import base64
import binascii
import json
from collections.abc import Mapping

from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from cryptography.hazmat.primitives import serialization

from agent_runtime import Action, Runtime

MAX_REQUEST_BYTES = 1024 * 1024 + 4096
MAX_REPLY_BYTES = 1024 * 1024
REQUEST_FIELDS = frozenset({"agent_id", "adapter", "nonce", "expires_at",
                             "artifact_b64", "agent_signature_hex"})


def _unique_object(pairs):
    obj = {}
    for key, value in pairs:
        if key in obj:
            raise ValueError("duplicate request field")
        obj[key] = value
    return obj


def _text(value, label):
    if not isinstance(value, str) or not 1 <= len(value) <= 128:
        raise ValueError("invalid " + label)
    return value


class LocalToolGateway:
    """A narrow trusted process-level interface for strictly stamped adapters.

    The calling host must isolate the issuer secret, SQLite file and adapter
    callbacks from the untrusted agent; Python object visibility alone is NOT
    an isolation boundary.
    """

    def __init__(self, runtime: Runtime, trusted_issuers: Mapping[str, Ed25519PrivateKey]):
        if not isinstance(runtime, Runtime) or runtime.stamp_enforcement != "all":
            raise ValueError("gateway requires strictly stamped Runtime")
        if not isinstance(trusted_issuers, Mapping):
            raise ValueError("invalid trusted issuer registry")
        if set(trusted_issuers) != set(runtime.adapters):
            raise ValueError("issuer registry must cover every registered adapter")
        issuers = dict(trusted_issuers)
        for name, issuer in issuers.items():
            if not isinstance(issuer, Ed25519PrivateKey):
                raise ValueError("issuer must be trusted signing key")
            pinned = issuer.public_key().public_bytes(
                serialization.Encoding.Raw, serialization.PublicFormat.Raw)
            if pinned != runtime.stamp_policies[name].issuer_key:
                raise ValueError("issuer does not match runtime-pinned public key")
        self._runtime = runtime
        self._issuers = issuers

    def submit(self, serialized_request: bytes) -> bytes:
        """Authorize and dispatch one signed action; reject arbitrary tool RPC."""
        if not isinstance(serialized_request, bytes) or not 0 < len(serialized_request) <= MAX_REQUEST_BYTES:
            raise ValueError("invalid request size")
        try:
            request = json.loads(serialized_request, object_pairs_hook=_unique_object)
        except (UnicodeError, json.JSONDecodeError) as exc:
            raise ValueError("invalid request encoding") from exc
        if not isinstance(request, dict) or set(request) != REQUEST_FIELDS:
            raise ValueError("unexpected gateway request fields")
        agent_id = _text(request["agent_id"], "agent")
        adapter_name = _text(request["adapter"], "adapter")
        nonce = _text(request["nonce"], "nonce")
        expiry = request["expires_at"]
        if type(expiry) is not int or expiry <= 0:
            raise ValueError("invalid expiry")
        if adapter_name not in self._issuers:
            raise ValueError("adapter not allowed by gateway")
        try:
            raw_artifact = base64.b64decode(request["artifact_b64"], validate=True)
            signature = bytes.fromhex(request["agent_signature_hex"])
        except (TypeError, ValueError, binascii.Error) as exc:
            raise ValueError("invalid artifact encoding or signature") from exc
        if not 0 < len(raw_artifact) <= MAX_REPLY_BYTES or len(signature) != 64:
            raise ValueError("invalid artifact or agent signature length")
        action = Action(agent_id, adapter_name, raw_artifact, nonce, expiry)
        action_signature = signature
        # No untrusted stamp is accepted. The trusted runtime performs live
        # admission checks, issues a scoped stamp, and verifies it AGAIN
        # transactionally before dispatching its host-owned adapter.
        stamp = self._runtime.issue_stamp_for_action(
            action, action_signature, self._issuers[adapter_name])
        output = self._runtime.execute(action, action_signature, stamp=stamp)
        if not isinstance(output, bytes) or len(output) > MAX_REPLY_BYTES:
            raise ValueError("invalid adapter output")
        return output
