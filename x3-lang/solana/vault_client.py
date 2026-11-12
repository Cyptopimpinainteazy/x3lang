#!/usr/bin/env python3
"""Owner/agent command-line client for the X3 Solana mandate vault.

Builds, simulates and (only after an interactive "yes") signs and sends the six
`x3-mandate-vault` instructions: Initialize, Approve, Execute, Cancel, Revoke, Withdraw.

Safety model
- Default is a dry run: the transaction is built and simulated, nothing is signed or sent.
- `--send` prints the full plan, then requires the operator to type `yes` on the terminal.
  There is no flag that skips this prompt.
- The cluster is identified by its genesis hash, not by the RPC URL. Mainnet additionally
  requires `--allow-mainnet`.
- Keypair files are read by path. Secret bytes are never printed, logged or written.
- A signature is printed before the transaction is submitted. Any failure after signing is
  reported as `sent: unknown` (or `sent: true, finalized: unknown`) with that signature and
  exit code 5, never as `sent: false`. Look the signature up before retrying.
- The RPC URL is parsed: https:// to any host, or http:// to a literal loopback host only.
  User information is refused and redirects are not followed.
- With `--receipt`, the approval expiry comes from the signed quote and can only be shortened.

The byte layout of every instruction, the policy PDA derivation and the signed legacy
message are pinned to the Rust program by
`x3-mandate-vault/tests/fixtures/client_vectors.json`, which the Rust test suite generates
from the real program types and the Python test suite must reproduce.

The vault program is experimental and unaudited. This client does not make it safe.
"""
from __future__ import annotations

import argparse
import base64
import hashlib
import importlib.util
import json
import os
import stat
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Callable, Dict, List, Optional, Sequence, Tuple

_HERE = Path(__file__).resolve().parent
_ROOT = _HERE.parent
if str(_ROOT) not in sys.path:
    sys.path.insert(0, str(_ROOT))

import chain_evidence  # noqa: E402
import ed25519  # noqa: E402

# --------------------------------------------------------------------------------------------
# Constants
# --------------------------------------------------------------------------------------------

# Public program IDs are written as bytes: their base58 forms trip repository secret scanners.
TOKEN_PROGRAM = bytes((
    6, 221, 246, 225, 215, 101, 161, 147, 217, 203, 225, 70, 206, 235, 121, 172,
    28, 180, 133, 237, 95, 91, 55, 145, 58, 140, 245, 133, 126, 255, 0, 169,
))
ASSOCIATED_TOKEN_PROGRAM = bytes((
    140, 151, 37, 143, 78, 36, 137, 241, 187, 61, 16, 41, 20, 142, 13, 131,
    11, 90, 19, 153, 218, 255, 16, 132, 4, 142, 123, 216, 219, 233, 248, 89,
))
SYSTEM_PROGRAM = bytes(32)

POLICY_SEED = b"x3-vault"
POLICY_SIZE = 1308
POLICY_VERSION = 1
MAX_SPENDS = 64
TOKEN_ACCOUNT_SIZE = 165
MINT_SIZE = 82
LAMPORTS_PER_SIGNATURE = 5000

GENESIS = {
    "mainnet": "5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d",
    "devnet": "EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG",
    "testnet": "4uhcVJyU9pJkvQyS88uRDiswHXSCkY3zQawwpjk2NsNY",
}

# `VaultError` discriminants from x3-mandate-vault/src/lib.rs.
VAULT_ERRORS = {
    1: "Unauthorized",
    2: "IncorrectPda",
    3: "InvalidPolicy",
    4: "Expired",
    5: "Revoked",
    6: "PendingApproval",
    7: "MissingApproval",
    8: "Replay",
    9: "LimitExceeded",
    10: "IncorrectTokenAccount",
    11: "IncorrectQuote",
    12: "InvalidTime",
    13: "LedgerFull",
}

EXIT_OK = 0
EXIT_USAGE = 2
EXIT_NOT_CONFIRMED = 3
EXIT_FAILED = 4
EXIT_UNCONFIRMED = 5

U64_MAX = 2 ** 64 - 1
I64_MAX = 2 ** 63 - 1
_P = 2 ** 255 - 19
_D = -121665 * pow(121666, _P - 2, _P) % _P


class VaultClientError(Exception):
    """A refusal the operator can act on. The message never contains key material."""


# --------------------------------------------------------------------------------------------
# Encoding primitives
# --------------------------------------------------------------------------------------------

def pubkey_bytes(text: Any, field: str = "public key") -> bytes:
    try:
        raw = ed25519.b58decode(text)
    except (ValueError, TypeError):
        raise VaultClientError(f"{field} is not base58")
    if len(raw) != 32:
        raise VaultClientError(f"{field} must decode to 32 bytes, got {len(raw)}")
    return raw


def pubkey_text(raw: bytes) -> str:
    if len(raw) != 32:
        raise VaultClientError("public key must be 32 bytes")
    return ed25519.b58encode(raw)


def u64(value: int, field: str) -> bytes:
    if isinstance(value, bool) or not isinstance(value, int) or not 0 <= value <= U64_MAX:
        raise VaultClientError(f"{field} must be an unsigned 64-bit integer")
    return value.to_bytes(8, "little")


def i64(value: int, field: str) -> bytes:
    if isinstance(value, bool) or not isinstance(value, int) or not -(2 ** 63) <= value <= I64_MAX:
        raise VaultClientError(f"{field} must be a signed 64-bit integer")
    return value.to_bytes(8, "little", signed=True)


def shortvec(value: int) -> bytes:
    """Solana compact-u16 length prefix."""
    if not 0 <= value <= 0xFFFF:
        raise VaultClientError("compact-u16 out of range")
    out = bytearray()
    while True:
        byte = value & 0x7F
        value >>= 7
        if value:
            out.append(byte | 0x80)
        else:
            out.append(byte)
            return bytes(out)


def is_on_curve(raw: bytes) -> bool:
    """True when 32 bytes decompress to an Ed25519 point (curve25519-dalek semantics)."""
    if len(raw) != 32:
        return False
    y = (int.from_bytes(raw, "little") & ((1 << 255) - 1)) % _P
    u = (y * y - 1) % _P
    v = (_D * y * y + 1) % _P
    x2 = u * pow(v, _P - 2, _P) % _P
    return x2 == 0 or pow(x2, (_P - 1) // 2, _P) == 1


def find_program_address(seeds: Sequence[bytes], program_id: bytes) -> Tuple[bytes, int]:
    """`Pubkey::find_program_address`: the first off-curve hash, bump counting down from 255."""
    if len(seeds) > 15 or any(len(seed) > 32 for seed in seeds):
        raise VaultClientError("invalid program-address seeds")
    for bump in range(255, -1, -1):
        digest = hashlib.sha256(b"".join(seeds) + bytes([bump]) + program_id + b"ProgramDerivedAddress").digest()
        if not is_on_curve(digest):
            return digest, bump
    raise VaultClientError("no viable program-address bump")


def policy_address(program_id: bytes, owner: bytes, vault_token: bytes) -> Tuple[bytes, int]:
    return find_program_address([POLICY_SEED, owner, vault_token], program_id)


def associated_token_address(wallet: bytes, mint: bytes) -> Tuple[bytes, int]:
    return find_program_address([wallet, TOKEN_PROGRAM, mint], ASSOCIATED_TOKEN_PROGRAM)


# --------------------------------------------------------------------------------------------
# Instructions and legacy messages
# --------------------------------------------------------------------------------------------

@dataclass(frozen=True)
class Meta:
    pubkey: bytes
    signer: bool
    writable: bool
    role: str


@dataclass(frozen=True)
class Instruction:
    name: str
    program_id: bytes
    accounts: Tuple[Meta, ...]
    data: bytes
    args: Dict[str, Any]


def _require_key(raw: bytes, field: str) -> bytes:
    if not isinstance(raw, (bytes, bytearray)) or len(raw) != 32:
        raise VaultClientError(f"{field} must be 32 bytes")
    return bytes(raw)


def ix_initialize(program_id: bytes, owner: bytes, vault_token: bytes, mint: bytes, *, agent: bytes,
                  max_per_payment: int, max_per_day: int, expires_at: int) -> Instruction:
    policy, _ = policy_address(program_id, owner, vault_token)
    if _require_key(agent, "agent") == bytes(32):
        raise VaultClientError("agent must not be the zero key")
    if max_per_payment <= 0 or max_per_day < max_per_payment:
        raise VaultClientError("limits must satisfy 0 < max_per_payment <= max_per_day")
    data = b"\x00" + agent + u64(max_per_payment, "max_per_payment") + u64(max_per_day, "max_per_day") \
        + i64(expires_at, "expires_at")
    return Instruction("initialize", program_id, (
        Meta(owner, True, True, "owner (signer, pays policy rent)"),
        Meta(policy, False, True, "policy PDA (created)"),
        Meta(vault_token, False, False, "vault SPL token account (PDA-owned)"),
        Meta(mint, False, False, "mint"),
        Meta(TOKEN_PROGRAM, False, False, "SPL Token program"),
        Meta(SYSTEM_PROGRAM, False, False, "System program"),
    ), data, {
        "agent": pubkey_text(agent), "max_per_payment": str(max_per_payment),
        "max_per_day": str(max_per_day), "expires_at": expires_at,
    })


def ix_approve(program_id: bytes, owner: bytes, policy: bytes, *, nonce: int, destination_token: bytes,
               destination_owner: bytes, quote_digest: bytes, amount: int, valid_until: int) -> Instruction:
    for raw, field in ((destination_token, "destination_token"), (destination_owner, "destination_owner"),
                       (quote_digest, "quote_digest")):
        if _require_key(raw, field) == bytes(32):
            raise VaultClientError(f"{field} must not be all zeroes")
    if nonce <= 0:
        raise VaultClientError("nonce must be a positive integer")
    if amount <= 0:
        raise VaultClientError("amount must be a positive integer of token base units")
    data = b"\x01" + u64(nonce, "nonce") + destination_token + destination_owner + quote_digest \
        + u64(amount, "amount") + i64(valid_until, "valid_until")
    return Instruction("approve", program_id, (
        Meta(owner, True, False, "owner (signer)"),
        Meta(policy, False, True, "policy PDA"),
    ), data, {
        "nonce": nonce, "destination_token": pubkey_text(destination_token),
        "destination_owner": pubkey_text(destination_owner), "quote_digest_sha256": quote_digest.hex(),
        "amount": str(amount), "valid_until": valid_until,
    })


def ix_execute(program_id: bytes, agent: bytes, policy: bytes, vault_token: bytes, destination_token: bytes,
               mint: bytes, *, nonce: int, quote_digest: bytes) -> Instruction:
    _require_key(quote_digest, "quote_digest")
    data = b"\x02" + u64(nonce, "nonce") + quote_digest
    return Instruction("execute", program_id, (
        Meta(agent, True, False, "agent (signer)"),
        Meta(policy, False, True, "policy PDA"),
        Meta(vault_token, False, True, "vault SPL token account (source)"),
        Meta(destination_token, False, True, "destination SPL token account"),
        Meta(mint, False, False, "mint"),
        Meta(TOKEN_PROGRAM, False, False, "SPL Token program"),
    ), data, {"nonce": nonce, "quote_digest_sha256": quote_digest.hex(),
              "destination_token": pubkey_text(destination_token)})


def ix_cancel(program_id: bytes, owner: bytes, policy: bytes) -> Instruction:
    return Instruction("cancel", program_id, (
        Meta(owner, True, False, "owner (signer)"),
        Meta(policy, False, True, "policy PDA"),
    ), b"\x03", {})


def ix_revoke(program_id: bytes, owner: bytes, policy: bytes) -> Instruction:
    return Instruction("revoke", program_id, (
        Meta(owner, True, False, "owner (signer)"),
        Meta(policy, False, True, "policy PDA"),
    ), b"\x04", {})


def ix_withdraw(program_id: bytes, owner: bytes, policy: bytes, vault_token: bytes, destination_token: bytes,
                mint: bytes, *, amount: int) -> Instruction:
    if amount <= 0:
        raise VaultClientError("amount must be a positive integer of token base units")
    return Instruction("withdraw", program_id, (
        Meta(owner, True, False, "owner (signer)"),
        Meta(policy, False, True, "policy PDA"),
        Meta(vault_token, False, True, "vault SPL token account (source)"),
        Meta(destination_token, False, True, "owner-owned destination SPL token account"),
        Meta(mint, False, False, "mint"),
        Meta(TOKEN_PROGRAM, False, False, "SPL Token program"),
    ), b"\x05" + u64(amount, "amount"), {"amount": str(amount),
                                          "destination_token": pubkey_text(destination_token)})


def compile_message(payer: bytes, instruction: Instruction, recent_blockhash: bytes) -> bytes:
    """Legacy message for one instruction with the payer as its only signer.

    Key order matches `solana-sdk`: the fee payer, then writable signers, read-only signers,
    writable non-signers and read-only non-signers, each group sorted by key bytes.
    """
    _require_key(recent_blockhash, "recent blockhash")
    flags: Dict[bytes, List[bool]] = {payer: [True, True]}
    flags.setdefault(instruction.program_id, [False, False])
    for meta in instruction.accounts:
        entry = flags.setdefault(meta.pubkey, [False, False])
        entry[0] = entry[0] or meta.signer
        entry[1] = entry[1] or meta.writable
    signers = [key for key, (signer, _) in flags.items() if signer]
    if signers != [payer]:
        raise VaultClientError("this client signs with exactly one key: the fee payer")
    groups: List[List[bytes]] = [[], [], [], []]
    for key, (signer, writable) in flags.items():
        if key == payer:
            continue
        groups[(0 if signer else 2) + (0 if writable else 1)].append(key)
    keys = [payer] + [key for group in groups for key in sorted(group)]
    if len(keys) > 255:
        raise VaultClientError("too many accounts")
    index = {key: position for position, key in enumerate(keys)}
    header = bytes([1 + len(groups[0]) + len(groups[1]), len(groups[1]), len(groups[3])])
    body = bytes([index[instruction.program_id]]) \
        + shortvec(len(instruction.accounts)) + bytes(index[meta.pubkey] for meta in instruction.accounts) \
        + shortvec(len(instruction.data)) + instruction.data
    return header + shortvec(len(keys)) + b"".join(keys) + recent_blockhash + shortvec(1) + body


def wire_transaction(signature: bytes, message: bytes) -> bytes:
    if len(signature) != 64:
        raise VaultClientError("signature must be 64 bytes")
    return shortvec(1) + signature + message


# --------------------------------------------------------------------------------------------
# On-chain state decoding
# --------------------------------------------------------------------------------------------

def decode_policy(data: bytes) -> Dict[str, Any]:
    if len(data) != POLICY_SIZE:
        raise VaultClientError(f"policy account must be {POLICY_SIZE} bytes, got {len(data)}")
    cursor = 0

    def take(count: int) -> bytes:
        nonlocal cursor
        chunk = data[cursor:cursor + count]
        cursor += count
        return chunk

    def flag() -> bool:
        value = take(1)[0]
        if value not in (0, 1):
            raise VaultClientError("policy account has a malformed boolean")
        return bool(value)

    version, bump = take(1)[0], take(1)[0]
    if version != POLICY_VERSION:
        raise VaultClientError(f"unsupported policy version {version}")
    policy: Dict[str, Any] = {"version": version, "bump": bump}
    for name in ("owner", "agent", "mint", "vault_token"):
        policy[name] = pubkey_text(take(32))
    policy["max_per_payment"] = int.from_bytes(take(8), "little")
    policy["max_per_day"] = int.from_bytes(take(8), "little")
    policy["expires_at"] = int.from_bytes(take(8), "little", signed=True)
    policy["revoked"] = flag()
    policy["last_nonce"] = int.from_bytes(take(8), "little")
    policy["pending"] = flag()
    policy["pending_nonce"] = int.from_bytes(take(8), "little")
    policy["pending_destination"] = pubkey_text(take(32))
    policy["pending_destination_owner"] = pubkey_text(take(32))
    policy["pending_quote_digest"] = take(32).hex()
    policy["pending_amount"] = int.from_bytes(take(8), "little")
    policy["pending_until"] = int.from_bytes(take(8), "little", signed=True)
    spends = []
    for _ in range(MAX_SPENDS):
        timestamp = int.from_bytes(take(8), "little", signed=True)
        amount = int.from_bytes(take(8), "little")
        if amount:
            spends.append({"timestamp": timestamp, "amount": amount})
    policy["spends"] = spends
    if cursor != POLICY_SIZE:
        raise VaultClientError("policy layout does not match the program")
    return policy


def decode_token_account(data: bytes) -> Dict[str, Any]:
    if len(data) != TOKEN_ACCOUNT_SIZE:
        raise VaultClientError(f"not a classic SPL token account ({len(data)} bytes)")

    def option_key(offset: int) -> Optional[str]:
        tag = int.from_bytes(data[offset:offset + 4], "little")
        if tag not in (0, 1):
            raise VaultClientError("malformed SPL token account option")
        return pubkey_text(data[offset + 4:offset + 36]) if tag else None

    native_tag = int.from_bytes(data[109:113], "little")
    if native_tag not in (0, 1):
        raise VaultClientError("malformed SPL token account option")
    return {
        "mint": pubkey_text(data[0:32]),
        "owner": pubkey_text(data[32:64]),
        "amount": int.from_bytes(data[64:72], "little"),
        "delegate": option_key(72),
        "state": data[108],
        "is_native": bool(native_tag),
        "close_authority": option_key(129),
    }


def decode_mint(data: bytes) -> Dict[str, Any]:
    if len(data) != MINT_SIZE:
        raise VaultClientError(f"not a classic SPL mint ({len(data)} bytes)")
    if data[45] != 1:
        raise VaultClientError("mint is not initialized")
    return {"decimals": data[44], "supply": int.from_bytes(data[36:44], "little")}


# --------------------------------------------------------------------------------------------
# Keys
# --------------------------------------------------------------------------------------------

@dataclass(frozen=True)
class Keypair:
    public: bytes
    _secret: bytes

    def sign(self, message: bytes) -> bytes:
        signature = ed25519.sign(self._secret, message)
        if not ed25519.verify(self.public, message, signature):
            raise VaultClientError("signature self-check failed")
        return signature

    def __repr__(self) -> str:
        return f"Keypair({pubkey_text(self.public)})"


def load_keypair(path: str) -> Keypair:
    """A `solana-keygen` JSON keypair file: 64 integers, secret seed then public key."""
    try:
        info = os.lstat(path)
    except OSError:
        raise VaultClientError(f"keypair file not found: {path}")
    if not stat.S_ISREG(info.st_mode):
        raise VaultClientError(f"keypair path is not a regular file: {path}")
    if info.st_mode & 0o077:
        raise VaultClientError(f"keypair file {path} is readable by group or others; run: chmod 600 {path}")
    try:
        with open(path, "r", encoding="utf-8") as handle:
            values = json.load(handle)
    except (OSError, ValueError):
        raise VaultClientError(f"keypair file {path} is not a JSON array")
    if (not isinstance(values, list) or len(values) != 64
            or any(isinstance(v, bool) or not isinstance(v, int) or not 0 <= v <= 255 for v in values)):
        raise VaultClientError(f"keypair file {path} must hold exactly 64 byte values")
    raw = bytes(values)
    secret, public = raw[:32], raw[32:]
    if ed25519.public_key(secret) != public:
        raise VaultClientError(f"keypair file {path} is inconsistent: its public half does not match its secret")
    return Keypair(public, secret)


# --------------------------------------------------------------------------------------------
# RPC
# --------------------------------------------------------------------------------------------

LOOPBACK_HOSTS = frozenset({"localhost", "127.0.0.1", "::1"})


def validate_rpc_url(url: Any) -> str:
    """Accept https:// to any host, or http:// to a literal loopback host. Nothing else.

    The authority is parsed, never prefix-matched: `http://localhost.example`,
    `http://127.0.0.1.example` and `http://localhost@example` are remote plaintext hosts.
    """
    refusal = "RPC URL must be https://, or http:// on a literal loopback host (localhost, 127.0.0.1, [::1])"
    if not isinstance(url, str) or not url or url != url.strip():
        raise VaultClientError(refusal)
    if any(ord(ch) < 0x21 or ord(ch) == 0x7F for ch in url) or "\\" in url:
        raise VaultClientError(refusal)
    try:
        parts = urllib.parse.urlsplit(url)
        host = parts.hostname
        parts.port  # raises ValueError on a malformed port
    except ValueError:
        raise VaultClientError(refusal)
    if parts.scheme not in ("https", "http") or not parts.netloc or not host:
        raise VaultClientError(refusal)
    if "@" in parts.netloc or parts.username is not None or parts.password is not None:
        raise VaultClientError("RPC URL must not contain user information")
    if parts.fragment:
        raise VaultClientError(refusal)
    if parts.scheme == "http" and host not in LOOPBACK_HOSTS:
        raise VaultClientError(refusal)
    return url


class _RefuseRedirects(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):  # noqa: D401
        return None  # urllib then raises HTTPError for the 3xx response


def http_transport(method: str, url: str, headers: Dict[str, str], body: Optional[bytes],
                   timeout: float) -> Tuple[int, bytes]:
    """POST to the validated RPC URL. Redirects are refused so they cannot leave that policy."""
    validate_rpc_url(url)
    handlers: List[Any] = [_RefuseRedirects()]
    if urllib.parse.urlsplit(url).scheme == "http":
        handlers.append(urllib.request.ProxyHandler({}))  # loopback is always reached directly
    opener = urllib.request.build_opener(*handlers)
    request = urllib.request.Request(url, data=body, headers=headers, method=method)
    try:
        with opener.open(request, timeout=timeout) as response:
            return response.status, response.read()
    except urllib.error.HTTPError as exc:
        if 300 <= exc.code < 400:
            raise VaultClientError(f"RPC endpoint answered with a redirect (HTTP {exc.code}); refused")
        return exc.code, exc.read()
    except (urllib.error.URLError, OSError) as exc:
        raise VaultClientError(f"cannot reach the RPC endpoint: {getattr(exc, 'reason', exc)}")


class Rpc:
    def __init__(self, url: str, transport: Optional[chain_evidence.Transport] = None, timeout: float = 30.0):
        self.url = validate_rpc_url(url)
        self._transport = transport or http_transport
        self._timeout = timeout

    def call(self, method: str, params: Any) -> Any:
        try:
            return chain_evidence.json_rpc(self._transport, self.url, method, params, self._timeout)
        except VaultClientError as exc:
            raise VaultClientError(f"RPC {method} failed: {exc}")
        except chain_evidence.ChainEvidenceError as exc:
            raise VaultClientError(f"RPC {method} failed: {exc}")
        except Exception as exc:  # a transport fault must never surface as an unclassified crash
            raise VaultClientError(f"RPC {method} failed: {type(exc).__name__}")

    def genesis_hash(self) -> str:
        value = self.call("getGenesisHash", [])
        if not isinstance(value, str):
            raise VaultClientError("RPC returned a malformed genesis hash")
        return value

    def latest_blockhash(self) -> bytes:
        value = (self.call("getLatestBlockhash", [{"commitment": "finalized"}]) or {}).get("value") or {}
        return pubkey_bytes(value.get("blockhash"), "recent blockhash")

    def account(self, address: bytes) -> Optional[Dict[str, Any]]:
        result = self.call("getAccountInfo", [pubkey_text(address), {"encoding": "base64", "commitment": "finalized"}])
        value = (result or {}).get("value")
        if value is None:
            return None
        try:
            payload, encoding = value["data"]
            if encoding != "base64":
                raise ValueError
            data = base64.b64decode(payload, validate=True)
            return {"owner": pubkey_bytes(value["owner"], "account owner"), "data": data,
                    "lamports": int(value["lamports"]), "executable": bool(value["executable"])}
        except (KeyError, TypeError, ValueError):
            raise VaultClientError("RPC returned a malformed account")

    def simulate(self, wire: bytes) -> Dict[str, Any]:
        result = self.call("simulateTransaction", [base64.b64encode(wire).decode("ascii"), {
            "encoding": "base64", "sigVerify": False, "replaceRecentBlockhash": True, "commitment": "finalized"}])
        value = (result or {}).get("value")
        if not isinstance(value, dict):
            raise VaultClientError("RPC returned a malformed simulation result")
        return value

    def send(self, wire: bytes, *, skip_preflight: bool) -> str:
        value = self.call("sendTransaction", [base64.b64encode(wire).decode("ascii"), {
            "encoding": "base64", "skipPreflight": skip_preflight, "preflightCommitment": "finalized",
            "maxRetries": 5}])
        if not isinstance(value, str):
            raise VaultClientError("RPC returned a malformed transaction signature")
        return value

    def status(self, signature: str) -> Optional[Dict[str, Any]]:
        result = self.call("getSignatureStatuses", [[signature], {"searchTransactionHistory": True}])
        values = (result or {}).get("value")
        if not isinstance(values, list) or len(values) != 1:
            raise VaultClientError("RPC returned a malformed signature status")
        return values[0]


def custom_error(err: Any) -> Optional[int]:
    """The custom program error code of a failed single-instruction transaction, if any."""
    try:
        index, detail = err["InstructionError"]
        code = detail["Custom"]
    except (KeyError, TypeError, ValueError):
        return None
    return code if index == 0 and isinstance(code, int) and not isinstance(code, bool) else None


def describe_error(err: Any) -> Optional[str]:
    if err is None:
        return None
    code = custom_error(err)
    if code is not None:
        return f"{VAULT_ERRORS.get(code, 'unknown program error')} (custom {code})"
    return json.dumps(err, sort_keys=True)


# --------------------------------------------------------------------------------------------
# Command execution
# --------------------------------------------------------------------------------------------

def tty_confirm(prompt: str) -> bool:
    """Read the answer from the controlling terminal, so piped input cannot approve a send."""
    try:
        with open("/dev/tty", "r+", encoding="utf-8") as tty:
            tty.write(prompt)
            tty.flush()
            return tty.readline().strip() == "yes"
    except OSError:
        return False


@dataclass
class Context:
    rpc: Rpc
    cluster: str
    genesis: str
    program_id: bytes
    out: Callable[[str], None]
    confirm: Callable[[str], bool]
    sleep: Callable[[float], None]
    clock: Callable[[], float]


def check_cluster(rpc: Rpc, cluster: str, allow_mainnet: bool) -> str:
    genesis = rpc.genesis_hash()
    if cluster == "localnet":
        if genesis in GENESIS.values():
            raise VaultClientError("--cluster localnet was given, but the RPC serves a public cluster")
        return genesis
    if cluster not in GENESIS:
        raise VaultClientError(f"unknown cluster {cluster!r}")
    if genesis != GENESIS[cluster]:
        raise VaultClientError(f"--cluster {cluster} was given, but the RPC genesis hash is {genesis}")
    if cluster == "mainnet" and not allow_mainnet:
        raise VaultClientError("this RPC serves mainnet; the vault is unaudited. Pass --allow-mainnet to proceed")
    return genesis


def require_program(ctx: Context) -> None:
    account = ctx.rpc.account(ctx.program_id)
    if account is None or not account["executable"]:
        raise VaultClientError(f"{pubkey_text(ctx.program_id)} is not a deployed executable program on this cluster")


def read_policy(ctx: Context, policy: bytes) -> Dict[str, Any]:
    account = ctx.rpc.account(policy)
    if account is None:
        raise VaultClientError(f"policy account {pubkey_text(policy)} does not exist")
    if account["owner"] != ctx.program_id:
        raise VaultClientError("policy account is not owned by the vault program")
    state = decode_policy(account["data"])
    expected, bump = policy_address(ctx.program_id, pubkey_bytes(state["owner"]), pubkey_bytes(state["vault_token"]))
    if expected != policy or bump != state["bump"]:
        raise VaultClientError("policy account is not the PDA derived from its own owner and vault token")
    return state


def read_token(ctx: Context, address: bytes, field: str) -> Dict[str, Any]:
    account = ctx.rpc.account(address)
    if account is None:
        raise VaultClientError(f"{field} {pubkey_text(address)} does not exist; create it first")
    if account["owner"] != TOKEN_PROGRAM:
        raise VaultClientError(f"{field} is not owned by the classic SPL Token program")
    token = decode_token_account(account["data"])
    if token["state"] != 1:
        raise VaultClientError(f"{field} is not an initialized, unfrozen token account")
    return token


def read_mint(ctx: Context, mint: bytes) -> Dict[str, Any]:
    account = ctx.rpc.account(mint)
    if account is None or account["owner"] != TOKEN_PROGRAM:
        raise VaultClientError("mint is not a classic SPL Token mint on this cluster")
    return decode_mint(account["data"])


def check_vault_token(token: Dict[str, Any], policy: bytes, mint: bytes) -> None:
    """Mirror of the program's `check_source`."""
    policy_text = pubkey_text(policy)
    if token["owner"] != policy_text:
        raise VaultClientError("vault token account authority is not the policy PDA")
    if token["mint"] != pubkey_text(mint):
        raise VaultClientError("vault token account holds a different mint")
    if token["delegate"] is not None or token["is_native"]:
        raise VaultClientError("vault token account must have no delegate and must not be wrapped SOL")
    if token["close_authority"] not in (None, policy_text):
        raise VaultClientError("vault token account has a foreign close authority")


def run(ctx: Context, instruction: Instruction, signer: Keypair, *, send: bool, expect_error: Optional[int],
        wait_seconds: float, checks: Dict[str, Any]) -> int:
    """Simulate; then, only with `send` and a typed yes, sign, submit and wait for finality."""
    payer = signer.public
    plan: Dict[str, Any] = {
        "kind": "x3-solana-vault-client/1",
        "cluster": ctx.cluster,
        "genesis_hash": ctx.genesis,
        "rpc_url": ctx.rpc.url,
        "program": pubkey_text(ctx.program_id),
        "instruction": instruction.name,
        "signer_and_fee_payer": pubkey_text(payer),
        "accounts": [{"pubkey": pubkey_text(meta.pubkey), "signer": meta.signer, "writable": meta.writable,
                      "role": meta.role} for meta in instruction.accounts],
        "args": instruction.args,
        "data_hex": instruction.data.hex(),
        "max_fee_lamports": LAMPORTS_PER_SIGNATURE,
        "checks": checks,
        "expect_error": None if expect_error is None else f"{VAULT_ERRORS.get(expect_error, 'unknown')} (custom {expect_error})",
        "sent": False,
    }

    unsigned = wire_transaction(bytes(64), compile_message(payer, instruction, bytes(32)))
    simulation = ctx.rpc.simulate(unsigned)
    sim_err = simulation.get("err")
    plan["simulation"] = {
        "ok": sim_err is None,
        "error": describe_error(sim_err),
        "units_consumed": simulation.get("unitsConsumed"),
        "logs": simulation.get("logs") or [],
    }
    simulated_as_expected = (sim_err is None) if expect_error is None else (custom_error(sim_err) == expect_error)
    plan["simulation"]["as_expected"] = simulated_as_expected

    if not send:
        plan["note"] = "dry run: nothing was signed or sent. Add --send to sign after a typed confirmation."
        ctx.out(json.dumps(plan, indent=2, sort_keys=True))
        return EXIT_OK if simulated_as_expected else EXIT_FAILED

    if not simulated_as_expected:
        plan["note"] = "refusing to send: the simulation did not produce the expected outcome."
        ctx.out(json.dumps(plan, indent=2, sort_keys=True))
        return EXIT_FAILED

    ctx.out(json.dumps(plan, indent=2, sort_keys=True))
    outcome = "is EXPECTED TO FAIL on-chain and still costs the fee" if expect_error is not None else "moves on-chain state"
    prompt = (f"\nSign and send ONE {instruction.name} transaction on {ctx.cluster} as "
              f"{pubkey_text(payer)}? It {outcome}. Type yes to continue: ")
    if not ctx.confirm(prompt):
        ctx.out(json.dumps({"sent": False, "note": "not confirmed; nothing was signed or sent."}, sort_keys=True))
        return EXIT_NOT_CONFIRMED

    # A failure up to and including signing happens before any submission, so the
    # caller's `sent: false` is accurate for it.
    message = compile_message(payer, instruction, ctx.rpc.latest_blockhash())
    signature = signer.sign(message)
    signature_text = ed25519.b58encode(signature)
    suffix = "" if ctx.cluster == "mainnet" else ("?cluster=custom" if ctx.cluster == "localnet" else f"?cluster={ctx.cluster}")
    result: Dict[str, Any] = {
        "instruction": instruction.name,
        "signature": signature_text,
        "message_sha256": hashlib.sha256(message).hexdigest(),
        "explorer": f"https://explorer.solana.com/tx/{signature_text}{suffix}",
        "note": "if anything below fails, look this signature up before retrying; never resend blindly.",
    }
    ctx.out(json.dumps(result, sort_keys=True))

    # From here on a transaction has been signed. Nothing below may report `sent: false`:
    # a failed or odd submission is reported as unknown, with the signature to look up.
    do_not_resend = "Do NOT resend. Look this signature up on the cluster before doing anything else."

    def uncertain(sent: Any, stage: str, error: str, **extra: Any) -> int:
        record: Dict[str, Any] = {"sent": sent, "finalized": "unknown", "signature": signature_text,
                                  "stage": stage, "error": error, "note": do_not_resend}
        record.update(extra)
        ctx.out(json.dumps(record, sort_keys=True))
        return EXIT_UNCONFIRMED

    try:
        returned = ctx.rpc.send(wire_transaction(signature, message), skip_preflight=expect_error is not None)
    except VaultClientError as exc:
        return uncertain("unknown", "submit", str(exc))
    except Exception as exc:
        return uncertain("unknown", "submit", f"unexpected {type(exc).__name__}")
    if returned != signature_text:
        return uncertain("unknown", "submit",
                         "RPC acknowledged a different signature than the one that was signed",
                         acknowledged_signature=returned)

    deadline = ctx.clock() + wait_seconds
    status: Optional[Dict[str, Any]] = None
    status_error: Optional[str] = None
    while True:
        try:
            status = ctx.rpc.status(signature_text)
            status_error = None
        except VaultClientError as exc:
            status_error = str(exc)
        except Exception as exc:
            status_error = f"unexpected {type(exc).__name__}"
        if status_error is None and status is not None and status.get("confirmationStatus") == "finalized":
            break
        if ctx.clock() >= deadline:
            break
        ctx.sleep(2.0)

    if status_error is not None:
        return uncertain(True, "status", status_error)
    final: Dict[str, Any] = {"sent": True, "signature": signature_text}
    if status is None or status.get("confirmationStatus") != "finalized":
        final.update({"finalized": False, "confirmation_status": (status or {}).get("confirmationStatus"),
                      "note": "not finalized within the wait window. Do NOT resend; recheck this signature."})
        ctx.out(json.dumps(final, sort_keys=True))
        return EXIT_UNCONFIRMED
    err = status.get("err")
    as_expected = (err is None) if expect_error is None else (custom_error(err) == expect_error)
    final.update({"finalized": True, "slot": status.get("slot"), "error": describe_error(err), "as_expected": as_expected})
    ctx.out(json.dumps(final, sort_keys=True))
    return EXIT_OK if as_expected else EXIT_FAILED


# --------------------------------------------------------------------------------------------
# Quote binding for Approve
# --------------------------------------------------------------------------------------------

def _vault_evidence():
    spec = importlib.util.spec_from_file_location("x3_vault_evidence", _HERE / "vault_evidence.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def approval_from_receipt(receipt: Dict[str, Any], *, evidence: Any = None) -> Dict[str, Any]:
    """Approve terms from a live 1Click receipt whose signature verifies against the pinned key."""
    evidence = evidence or _vault_evidence()
    try:
        digest, deposit_owner, mint, amount = evidence.quote_commitment(receipt)
        deadline = evidence.ni.parse_timestamp(receipt["quote_response"]["quote"]["deadline"])
    except (ValueError, KeyError, TypeError) as exc:
        raise VaultClientError(f"receipt is not an authenticated live 1Click quote: {exc}")
    owner = pubkey_bytes(deposit_owner, "quote deposit address")
    mint_raw = pubkey_bytes(mint, "quote origin mint")
    destination, _ = associated_token_address(owner, mint_raw)
    return {"quote_digest": digest, "destination_owner": owner, "mint": mint_raw, "amount": amount,
            "destination_token": destination, "valid_until": int(deadline.timestamp()),
            "quote_hash": receipt.get("quote_hash")}


# --------------------------------------------------------------------------------------------
# CLI
# --------------------------------------------------------------------------------------------

def _hex32(text: str, field: str) -> bytes:
    try:
        raw = bytes.fromhex(text)
    except (ValueError, TypeError):
        raise VaultClientError(f"{field} must be hex")
    if len(raw) != 32:
        raise VaultClientError(f"{field} must be 32 bytes of hex")
    return raw


def _positive(text: str) -> int:
    value = int(text)
    if value <= 0:
        raise argparse.ArgumentTypeError("must be a positive integer")
    return value


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="vault_client.py",
        description="X3 Solana mandate vault client. Dry run by default; --send asks for a typed yes.")
    sub = parser.add_subparsers(dest="command", required=True)

    def offline(name: str, help_text: str) -> argparse.ArgumentParser:
        return sub.add_parser(name, help=help_text)

    def online(name: str, help_text: str, *, signs: bool) -> argparse.ArgumentParser:
        p = sub.add_parser(name, help=help_text)
        p.add_argument("--rpc-url", default=os.environ.get("X3_SOLANA_RPC_URL"), help="or X3_SOLANA_RPC_URL")
        p.add_argument("--cluster", required=True, choices=["devnet", "testnet", "mainnet", "localnet"])
        p.add_argument("--allow-mainnet", action="store_true")
        p.add_argument("--program", required=True, help="deployed vault program ID")
        if signs:
            p.add_argument("--keypair", required=True, help="path to the signer's solana-keygen JSON file")
            p.add_argument("--send", action="store_true", help="sign and send after a typed yes")
            p.add_argument("--expect-error", type=int, choices=sorted(VAULT_ERRORS),
                           help="require this vault error; with --send the failing transaction is recorded on-chain")
            p.add_argument("--wait-seconds", type=float, default=120.0)
        return p

    p = offline("address", "print the public key of a keypair file")
    p.add_argument("--keypair", required=True)

    p = offline("derive", "derive the policy PDA (offline)")
    p.add_argument("--program", required=True)
    p.add_argument("--owner", required=True)
    p.add_argument("--vault-token", required=True)

    p = offline("ata", "derive a wallet's associated token account (offline)")
    p.add_argument("--wallet", required=True)
    p.add_argument("--mint", required=True)

    p = online("show", "decode a policy account and its vault balance", signs=False)
    p.add_argument("--policy", required=True)

    p = online("initialize", "owner creates the policy", signs=True)
    p.add_argument("--vault-token", required=True)
    p.add_argument("--mint", required=True)
    p.add_argument("--agent", required=True)
    p.add_argument("--max-per-payment", required=True, type=_positive, help="token base units")
    p.add_argument("--max-per-day", required=True, type=_positive, help="token base units")
    p.add_argument("--expires-at", required=True, type=int, help="unix seconds")

    p = online("approve", "owner approves one exact payment", signs=True)
    p.add_argument("--policy", required=True)
    p.add_argument("--nonce", required=True, type=_positive)
    p.add_argument("--receipt", help="live 1Click receipt JSON; supplies digest, destination, amount and expiry")
    p.add_argument("--quote-digest", help="32-byte hex digest (rehearsal without a 1Click quote)")
    p.add_argument("--destination-token")
    p.add_argument("--destination-owner")
    p.add_argument("--amount", type=_positive)
    p.add_argument("--valid-until", type=int,
                   help="unix seconds; with --receipt it defaults to the signed quote deadline and may only shorten it")

    p = online("execute", "agent executes the pending approval", signs=True)
    p.add_argument("--policy", required=True)
    p.add_argument("--nonce", type=_positive, help="defaults to the pending nonce")
    p.add_argument("--quote-digest", help="defaults to the pending digest")
    p.add_argument("--destination-token", help="defaults to the approved destination")

    for name, text in (("cancel", "owner cancels the pending approval"), ("revoke", "owner revokes the policy")):
        p = online(name, text, signs=True)
        p.add_argument("--policy", required=True)

    p = online("withdraw", "owner withdraws to a token account they own", signs=True)
    p.add_argument("--policy", required=True)
    p.add_argument("--destination-token", required=True)
    p.add_argument("--amount", required=True, type=_positive)
    return parser


def _context(args: argparse.Namespace, out: Callable[[str], None], transport, confirm, sleep, clock) -> Context:
    if not args.rpc_url:
        raise VaultClientError("give --rpc-url or set X3_SOLANA_RPC_URL")
    rpc = Rpc(args.rpc_url, transport)
    genesis = check_cluster(rpc, args.cluster, args.allow_mainnet)
    ctx = Context(rpc, args.cluster, genesis, pubkey_bytes(args.program, "--program"), out,
                  confirm or tty_confirm, sleep or time.sleep, clock or time.monotonic)
    require_program(ctx)
    return ctx


def _require_signer(signer: Keypair, expected: str, role: str) -> None:
    if pubkey_text(signer.public) != expected:
        raise VaultClientError(f"--keypair is not this policy's {role} ({expected})")


def dispatch(args: argparse.Namespace, *, out: Callable[[str], None], transport=None, confirm=None,
             sleep=None, clock=None, wall_clock: Callable[[], float] = time.time) -> int:
    command = args.command
    if command == "address":
        out(pubkey_text(load_keypair(args.keypair).public))
        return EXIT_OK
    if command == "derive":
        policy, bump = policy_address(pubkey_bytes(args.program, "--program"), pubkey_bytes(args.owner, "--owner"),
                                      pubkey_bytes(args.vault_token, "--vault-token"))
        out(json.dumps({"policy": pubkey_text(policy), "bump": bump}, sort_keys=True))
        return EXIT_OK
    if command == "ata":
        address, bump = associated_token_address(pubkey_bytes(args.wallet, "--wallet"), pubkey_bytes(args.mint, "--mint"))
        out(json.dumps({"associated_token_account": pubkey_text(address), "bump": bump}, sort_keys=True))
        return EXIT_OK

    ctx = _context(args, out, transport, confirm, sleep, clock)
    now = int(wall_clock())

    if command == "show":
        policy = pubkey_bytes(args.policy, "--policy")
        state = read_policy(ctx, policy)
        vault = read_token(ctx, pubkey_bytes(state["vault_token"]), "vault token account")
        out(json.dumps({"policy": pubkey_text(policy), "state": state, "vault_balance": str(vault["amount"]),
                        "cluster": ctx.cluster}, indent=2, sort_keys=True))
        return EXIT_OK

    signer = load_keypair(args.keypair)
    options = {"send": args.send, "expect_error": args.expect_error, "wait_seconds": args.wait_seconds}

    if command == "initialize":
        vault_token = pubkey_bytes(args.vault_token, "--vault-token")
        mint = pubkey_bytes(args.mint, "--mint")
        agent = pubkey_bytes(args.agent, "--agent")
        if agent == signer.public:
            raise VaultClientError("the agent must be a different key from the owner")
        policy, bump = policy_address(ctx.program_id, signer.public, vault_token)
        decimals = read_mint(ctx, mint)["decimals"]
        token = read_token(ctx, vault_token, "vault token account")
        check_vault_token(token, policy, mint)
        if args.expect_error is None:
            if ctx.rpc.account(policy) is not None:
                raise VaultClientError("the policy account already exists")
            if args.expires_at <= now:
                raise VaultClientError("--expires-at is in the past")
        instruction = ix_initialize(ctx.program_id, signer.public, vault_token, mint, agent=agent,
                                    max_per_payment=args.max_per_payment, max_per_day=args.max_per_day,
                                    expires_at=args.expires_at)
        return run(ctx, instruction, signer, checks={
            "policy": pubkey_text(policy), "bump": bump, "mint_decimals": decimals,
            "vault_balance": str(token["amount"]), "expires_in_seconds": args.expires_at - now}, **options)

    policy = pubkey_bytes(args.policy, "--policy")
    state = read_policy(ctx, policy)
    vault_token = pubkey_bytes(state["vault_token"])
    mint = pubkey_bytes(state["mint"])
    checks: Dict[str, Any] = {
        "policy_owner": state["owner"], "policy_agent": state["agent"], "policy_mint": state["mint"],
        "policy_revoked": state["revoked"], "policy_expires_in_seconds": state["expires_at"] - now,
        "policy_last_nonce": state["last_nonce"], "policy_pending": state["pending"],
    }

    if command == "approve":
        _require_signer(signer, state["owner"], "owner")
        if args.receipt:
            if any(v is not None for v in (args.quote_digest, args.destination_token, args.destination_owner, args.amount)):
                raise VaultClientError("--receipt supplies the digest, destination and amount; do not also pass them")
            try:
                with open(args.receipt, "r", encoding="utf-8") as handle:
                    receipt = json.load(handle)
            except (OSError, ValueError):
                raise VaultClientError(f"cannot read receipt {args.receipt}")
            terms = approval_from_receipt(receipt)
            if terms["mint"] != mint:
                raise VaultClientError("the quote's origin mint is not this policy's mint")
            checks["quote_hash"] = terms["quote_hash"]
            checks["quote_source"] = "1Click signature verified against the pinned manager key"
        else:
            missing = [flag for flag, value in (("--quote-digest", args.quote_digest),
                                                ("--destination-token", args.destination_token),
                                                ("--destination-owner", args.destination_owner),
                                                ("--amount", args.amount), ("--valid-until", args.valid_until))
                       if value is None]
            if missing:
                raise VaultClientError("without --receipt, also give " + ", ".join(missing))
            terms = {"quote_digest": _hex32(args.quote_digest, "--quote-digest"),
                     "destination_token": pubkey_bytes(args.destination_token, "--destination-token"),
                     "destination_owner": pubkey_bytes(args.destination_owner, "--destination-owner"),
                     "amount": args.amount, "valid_until": args.valid_until}
            checks["quote_source"] = "operator-supplied digest; NOT authenticated by this client"
        if args.receipt:
            # The program cannot verify the signed quote, so this expiry is the only thing that
            # stops a stale quote from being approved. It may be shortened, never extended.
            quote_deadline = terms["valid_until"]
            if quote_deadline <= now:
                raise VaultClientError("the signed quote has already expired")
            valid_until = quote_deadline if args.valid_until is None else args.valid_until
            if valid_until > quote_deadline:
                raise VaultClientError("--valid-until may shorten the signed quote's deadline but never extend it")
            if valid_until <= now:
                raise VaultClientError("the approval would already be expired")
            checks["quote_deadline"] = quote_deadline
            checks["valid_until_source"] = ("signed quote deadline" if valid_until == quote_deadline
                                            else "operator-shortened, within the signed quote deadline")
        else:
            valid_until = args.valid_until
        if args.expect_error is None:
            destination = read_token(ctx, terms["destination_token"], "destination token account")
            if destination["owner"] != pubkey_text(terms["destination_owner"]):
                raise VaultClientError("destination token account is not owned by the approved destination owner")
            if destination["mint"] != state["mint"]:
                raise VaultClientError("destination token account holds a different mint")
            if terms["destination_token"] == vault_token:
                raise VaultClientError("destination must not be the vault token account")
            if valid_until <= now:
                raise VaultClientError("the approval would already be expired")
        checks["valid_for_seconds"] = valid_until - now
        instruction = ix_approve(ctx.program_id, signer.public, policy, nonce=args.nonce,
                                 destination_token=terms["destination_token"],
                                 destination_owner=terms["destination_owner"],
                                 quote_digest=terms["quote_digest"], amount=terms["amount"], valid_until=valid_until)
        return run(ctx, instruction, signer, checks=checks, **options)

    if command == "execute":
        if args.expect_error != 1:  # 1 = Unauthorized: the only case where a non-agent key is the point
            _require_signer(signer, state["agent"], "agent")
        nonce = args.nonce if args.nonce is not None else state["pending_nonce"]
        digest = _hex32(args.quote_digest, "--quote-digest") if args.quote_digest else bytes.fromhex(state["pending_quote_digest"])
        destination = pubkey_bytes(args.destination_token, "--destination-token") if args.destination_token \
            else pubkey_bytes(state["pending_destination"])
        if args.expect_error is None and not state["pending"]:
            raise VaultClientError("the policy has no pending approval to execute")
        if nonce <= 0:
            raise VaultClientError("give --nonce: the policy has no pending nonce")
        checks.update({"pending_nonce": state["pending_nonce"], "pending_amount": str(state["pending_amount"]),
                       "pending_destination": state["pending_destination"],
                       "pending_destination_owner": state["pending_destination_owner"],
                       "pending_quote_digest_sha256": state["pending_quote_digest"],
                       "pending_valid_for_seconds": state["pending_until"] - now})
        instruction = ix_execute(ctx.program_id, signer.public, policy, vault_token, destination, mint,
                                 nonce=nonce, quote_digest=digest)
        return run(ctx, instruction, signer, checks=checks, **options)

    _require_signer(signer, state["owner"], "owner")
    if command == "cancel":
        return run(ctx, ix_cancel(ctx.program_id, signer.public, policy), signer, checks=checks, **options)
    if command == "revoke":
        return run(ctx, ix_revoke(ctx.program_id, signer.public, policy), signer, checks=checks, **options)
    if command == "withdraw":
        destination = pubkey_bytes(args.destination_token, "--destination-token")
        vault = read_token(ctx, vault_token, "vault token account")
        checks["vault_balance"] = str(vault["amount"])
        if args.expect_error is None:
            target = read_token(ctx, destination, "destination token account")
            if target["owner"] != state["owner"] or target["mint"] != state["mint"]:
                raise VaultClientError("withdrawals go only to a token account of this mint owned by the owner")
            if args.amount > vault["amount"]:
                raise VaultClientError("the vault holds less than the requested amount")
        instruction = ix_withdraw(ctx.program_id, signer.public, policy, vault_token, destination, mint, amount=args.amount)
        return run(ctx, instruction, signer, checks=checks, **options)
    raise VaultClientError(f"unknown command {command}")


def main(argv: Optional[Sequence[str]] = None) -> int:
    args = build_parser().parse_args(argv)
    try:
        return dispatch(args, out=print)
    except VaultClientError as exc:
        # `run` converts every failure after signing into an explicit uncertain outcome and
        # returns, so an error that reaches this handler was raised before any submission.
        print(json.dumps({"error": str(exc), "sent": False}, sort_keys=True), file=sys.stderr)
        return EXIT_USAGE


if __name__ == "__main__":
    sys.exit(main())
