"""Minimal NEAR transaction signing for one action type: FunctionCall.

Borsh layout (nearcore ``Transaction`` V0 and ``SignedTransaction``), checked byte for byte
against ``@near-js/transactions`` 2.5.1 vectors in ``tests/test_dx3hack_faucet.py``:

    Transaction   = signer_id: string, public_key: PublicKey, nonce: u64, receiver_id: string,
                    block_hash: [u8; 32], actions: Vec<Action>
    PublicKey     = key_type: u8 (0 = ed25519), data: [u8; 32]
    Action        = variant u8 (2 = FunctionCall), method_name: string, args: Vec<u8>, gas: u64,
                    deposit: u128
    Signed        = Transaction, Signature (key_type u8, data [u8; 64])

The transaction hash is ``sha256(borsh(Transaction))``; the signature is Ed25519 over that hash.
"""

from __future__ import annotations

import base64
import hashlib
import json
import re
import struct
import urllib.error
import urllib.request
from typing import Any, Callable, Dict, List, Optional, Sequence, Tuple

from cryptography.hazmat.primitives import serialization
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey

from ed25519 import b58decode, b58encode

ACTION_FUNCTION_CALL = 2
KEY_ED25519 = 0
MAX_GAS = 300 * 10 ** 12
U64_MAX = (1 << 64) - 1
U128_MAX = (1 << 128) - 1
ACCOUNT_RE = re.compile(r"^(?=.{2,64}$)([a-z0-9]+[-_])*[a-z0-9]+(\.([a-z0-9]+[-_])*[a-z0-9]+)*$")


class NearError(RuntimeError):
    """An RPC or transport failure (not a contract refusal)."""

    def __init__(self, message: str, name: str = "RPC_ERROR"):
        super().__init__(message)
        self.name = name


def valid_account(account: Any) -> bool:
    return isinstance(account, str) and bool(ACCOUNT_RE.match(account))


# --------------------------------------------------------------------------------------------
# Borsh
# --------------------------------------------------------------------------------------------

def _u8(v: int) -> bytes:
    return struct.pack("<B", v)


def _u32(v: int) -> bytes:
    return struct.pack("<I", v)


def _u64(v: int) -> bytes:
    if not 0 <= v <= U64_MAX:
        raise ValueError("u64 out of range")
    return struct.pack("<Q", v)


def _u128(v: int) -> bytes:
    if not 0 <= v <= U128_MAX:
        raise ValueError("u128 out of range")
    return v.to_bytes(16, "little")


def _string(s: str) -> bytes:
    raw = s.encode("utf-8")
    return _u32(len(raw)) + raw


def _vec_u8(b: bytes) -> bytes:
    return _u32(len(b)) + b


def public_key_bytes(public_key: str) -> bytes:
    if not isinstance(public_key, str) or not public_key.startswith("ed25519:"):
        raise ValueError("public key must be ed25519:<base58>")
    raw = b58decode(public_key[len("ed25519:"):])
    if len(raw) != 32:
        raise ValueError("ed25519 public key must be 32 bytes")
    return raw


def encode_function_call(*, signer_id: str, public_key: str, nonce: int, receiver_id: str, block_hash: str,
                         method_name: str, args: bytes, gas: int, deposit: int) -> bytes:
    if not valid_account(signer_id) or not valid_account(receiver_id):
        raise ValueError("signer_id and receiver_id must be NEAR account ids")
    if not re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]{0,255}", method_name):
        raise ValueError("bad method name")
    if not 0 < gas <= MAX_GAS:
        raise ValueError("gas must be between 1 and 300 Tgas")
    block = b58decode(block_hash)
    if len(block) != 32:
        raise ValueError("block hash must be 32 bytes")
    action = (_u8(ACTION_FUNCTION_CALL) + _string(method_name) + _vec_u8(args) + _u64(gas) + _u128(deposit))
    return (_string(signer_id) + _u8(KEY_ED25519) + public_key_bytes(public_key) + _u64(nonce) +
            _string(receiver_id) + block + _u32(1) + action)


def json_args(args: Dict[str, Any]) -> bytes:
    return json.dumps(args, separators=(",", ":"), ensure_ascii=False).encode("utf-8")


# --------------------------------------------------------------------------------------------
# Keys
# --------------------------------------------------------------------------------------------

class SigningKey:
    """A NEAR CLI style key file: {"account_id", "public_key", "private_key": "ed25519:<b58>"}."""

    def __init__(self, account_id: str, seed: bytes):
        if not valid_account(account_id):
            raise ValueError("key file account_id is not a NEAR account id")
        if len(seed) != 32:
            raise ValueError("ed25519 seed must be 32 bytes")
        self.account_id = account_id
        self._key = Ed25519PrivateKey.from_private_bytes(seed)
        raw = self._key.public_key().public_bytes(serialization.Encoding.Raw, serialization.PublicFormat.Raw)
        self.public_key = "ed25519:" + b58encode(raw)

    @classmethod
    def from_file(cls, path: str) -> "SigningKey":
        with open(path, "r", encoding="utf-8") as fh:
            data = json.load(fh)
        secret = data.get("private_key") or data.get("secret_key")
        if not isinstance(secret, str) or not secret.startswith("ed25519:"):
            raise ValueError(f"{path}: private_key must be ed25519:<base58>")
        raw = b58decode(secret[len("ed25519:"):])
        if len(raw) not in (32, 64):
            raise ValueError(f"{path}: private key must be a 32-byte seed or 64-byte seed+public key")
        key = cls(data.get("account_id", ""), raw[:32])
        if data.get("public_key") and data["public_key"] != key.public_key:
            raise ValueError(f"{path}: public_key does not match private_key")
        if len(raw) == 64 and "ed25519:" + b58encode(raw[32:]) != key.public_key:
            raise ValueError(f"{path}: embedded public half does not match the seed")
        return key

    def sign_transaction(self, tx: bytes) -> Tuple[str, str]:
        """Returns (signed_tx_base64, tx_hash_base58)."""
        digest = hashlib.sha256(tx).digest()
        signature = self._key.sign(digest)
        signed = tx + _u8(KEY_ED25519) + signature
        return base64.b64encode(signed).decode("ascii"), b58encode(digest)


# --------------------------------------------------------------------------------------------
# RPC
# --------------------------------------------------------------------------------------------

Transport = Callable[[str, bytes, float], Tuple[int, bytes]]


def urllib_transport(url: str, body: bytes, timeout: float) -> Tuple[int, bytes]:
    req = urllib.request.Request(url, data=body, method="POST", headers={"Content-Type": "application/json"})
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:  # noqa: S310 - fixed https RPC URLs
            return resp.status, resp.read(8 * 1024 * 1024)
    except urllib.error.HTTPError as exc:
        return exc.code, exc.read(64 * 1024)
    except (urllib.error.URLError, OSError) as exc:
        raise NearError(f"{url}: {exc}", "UNREACHABLE") from exc


class NearRpc:
    def __init__(self, urls: Sequence[str], transport: Transport = urllib_transport, timeout: float = 30.0):
        if not urls:
            raise ValueError("at least one RPC URL required")
        self.urls = list(urls)
        self.transport = transport
        self.timeout = timeout

    def call(self, method: str, params: Any, *, retry_all: bool = True, timeout: Optional[float] = None) -> Any:
        body = json.dumps({"jsonrpc": "2.0", "id": "dx3hack", "method": method, "params": params}).encode()
        last: Optional[NearError] = None
        for url in (self.urls if retry_all else self.urls[:1]):
            try:
                status, raw = self.transport(url, body, timeout or self.timeout)
            except NearError as exc:
                last = exc
                continue
            try:
                data = json.loads(raw.decode("utf-8"))
            except (UnicodeDecodeError, json.JSONDecodeError):
                last = NearError(f"{url} answered {status} with non-JSON", "BAD_RESPONSE")
                continue
            if isinstance(data, dict) and data.get("error"):
                err = data["error"]
                name = (err.get("cause") or {}).get("name") or err.get("name") or "RPC_ERROR"
                info = (err.get("cause") or {}).get("info") or err.get("data") or err.get("message")
                raise NearError(f"{name}: {json.dumps(info)[:400]}", name)
            if status != 200 or not isinstance(data, dict) or "result" not in data:
                last = NearError(f"{url} answered {status}", "BAD_RESPONSE")
                continue
            return data["result"]
        raise last or NearError("no RPC answered")

    def view_access_key(self, account_id: str, public_key: str) -> Dict[str, Any]:
        return self.call("query", {"request_type": "view_access_key", "finality": "final",
                                   "account_id": account_id, "public_key": public_key})

    def view(self, contract: str, method: str, args: Dict[str, Any]) -> Any:
        result = self.call("query", {"request_type": "call_function", "finality": "final", "account_id": contract,
                                     "method_name": method,
                                     "args_base64": base64.b64encode(json_args(args)).decode()})
        raw = bytes(result.get("result") or [])
        return json.loads(raw.decode("utf-8")) if raw else None

    def send_tx(self, signed_b64: str, wait_until: str = "FINAL") -> Dict[str, Any]:
        # Broadcast to one RPC only: retrying a send on another node could submit twice; the
        # nonce makes a duplicate harmless, but the second answer would be misleading.
        return self.call("send_tx", {"signed_tx_base64": signed_b64, "wait_until": wait_until}, retry_all=False,
                         timeout=60)


def _find(obj: Any, key: str) -> Optional[Any]:
    if isinstance(obj, dict):
        if key in obj:
            return obj[key]
        for value in obj.values():
            found = _find(value, key)
            if found is not None:
                return found
    elif isinstance(obj, list):
        for value in obj:
            found = _find(value, key)
            if found is not None:
                return found
    return None


def classify(result: Dict[str, Any]) -> Tuple[str, Optional[str], Any]:
    """(outcome, refusal_reason, return_value) for a final ``send_tx`` result.

    ``success`` only when the transaction and every receipt succeeded; otherwise ``refused``
    with the contract's panic message when there is one.
    """
    failures: List[Any] = []
    status = (result or {}).get("status") or {}
    if "Failure" in status:
        failures.append(status["Failure"])
    for receipt in (result or {}).get("receipts_outcome") or []:
        rstatus = ((receipt or {}).get("outcome") or {}).get("status") or {}
        if "Failure" in rstatus:
            failures.append(rstatus["Failure"])
    if failures:
        message = _find(failures, "ExecutionError")
        if isinstance(message, str):
            message = message.replace("Smart contract panicked: ", "")
        return "refused", message if isinstance(message, str) else json.dumps(failures[0])[:400], None
    value = None
    if isinstance(status.get("SuccessValue"), str) and status["SuccessValue"]:
        try:
            value = json.loads(base64.b64decode(status["SuccessValue"]).decode("utf-8"))
        except (ValueError, UnicodeDecodeError):
            value = status["SuccessValue"]
    if "SuccessValue" not in status and "SuccessReceiptId" not in status:
        return "unknown", "transaction status is neither success nor failure", None
    return "success", None, value
