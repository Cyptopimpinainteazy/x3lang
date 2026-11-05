"""Read-only NEAR escrow state observer: RPC agreement is NOT cryptographic finality.

This module never signs BSC attestations, moves funds, or marks swaps settled.
Two separately addressed RPC services can collude, share an upstream, or lie.
A trusted finalized-header/light-client validation remains mandatory.
"""
from __future__ import annotations

import json
import re
import sqlite3
import urllib.request
from dataclasses import dataclass
from pathlib import Path
from urllib.parse import urlparse


class ObservationError(ValueError):
    pass


@dataclass(frozen=True)
class Observation:
    network: str
    vault: str
    swap_id: str
    outcome: str
    block_hash: str
    block_height: int
    amount_yocto: int
    recipient: str
    # Multiple RPCs agreeing is insufficient to authorize fund movement.
    authorized_to_sign: bool = False


def _get(endpoint: str, payload: dict) -> dict:
    request = urllib.request.Request(
        endpoint, data=json.dumps(payload, separators=(",", ":")).encode("utf-8"),
        headers={"Content-Type": "application/json", "Accept": "application/json"},
        method="POST",
    )
    try:
        with urllib.request.urlopen(request, timeout=8) as response:
            if response.status != 200 or response.headers.get("Content-Type", "").split(";")[0] != "application/json":
                raise ObservationError("untrusted NEAR RPC response")
            raw = response.read(1024 * 1024 + 1)
            if len(raw) > 1024 * 1024:
                raise ObservationError("oversized NEAR RPC response")
            return json.loads(raw)
    except (OSError, ValueError, TimeoutError) as exc:
        raise ObservationError("NEAR RPC unavailable or malformed") from exc


def _strict_bytes(value: object) -> bytes:
    if (not isinstance(value, list) or len(value) > 100_000
            or any(type(i) is not int or i < 0 or i > 255 for i in value)):
        raise ObservationError("invalid NEAR view_call byte array")
    return bytes(value)


def _parse(response: object, *, vault: str, swap_id: str, recipient: str,
           initiator: str, hashlock: bytes, amount: int) -> tuple:
    if not isinstance(response, dict) or response.get("error") is not None or response.get("id") != "x3-near-observer":
        raise ObservationError("NEAR RPC error or request mismatch")
    result = response.get("result")
    if not isinstance(result, dict) or result.get("logs") != []:
        raise ObservationError("malformed NEAR execution result")
    block_hash = result.get("block_hash")
    height = result.get("block_height")
    if not isinstance(block_hash, str) or not block_hash or type(height) is not int or height <= 0:
        raise ObservationError("unidentified finalized block")
    try:
        swap = json.loads(_strict_bytes(result.get("result")))
    except (ValueError, UnicodeError) as exc:
        raise ObservationError("invalid NEAR swap JSON") from exc
    if not isinstance(swap, dict):
        raise ObservationError("swap absent or invalid")
    if swap.get("recipient") != recipient or swap.get("initiator") != initiator:
        raise ObservationError("NEAR owner mismatch")
    if _strict_bytes(swap.get("hashlock")) != hashlock:
        raise ObservationError("NEAR hashlock mismatch")
    amount_raw = swap.get("amount")
    if type(amount_raw) is not str or not re.fullmatch(r"[1-9][0-9]*", amount_raw) or int(amount_raw) != amount:
        raise ObservationError("NEAR amount mismatch")
    if type(swap.get("expires_at_ns")) is not int or swap["expires_at_ns"] <= 0:
        raise ObservationError("invalid NEAR expiry")
    outcome = swap.get("state")
    if outcome not in ("Locked", "Claimed", "Refunded") or type(outcome) is not str:
        raise ObservationError("unrecognized NEAR outcome")
    return (block_hash, height, swap["initiator"], swap["recipient"], bytes(hashlock), amount,
            swap["expires_at_ns"], outcome)


def observe(endpoints: list[str], *, vault: str, swap_id: str, recipient: str,
            initiator: str, hashlock: bytes, near_amount_yocto: int,
            fetch=None, network: str = "testnet") -> Observation:
    """Return an *untrusted observation*, never a signed settlement certificate.

    Even unanimous finality-tag RPC answers do not verify NEAR's finality proof,
    receipt causality, vault code integrity, or authenticated deployed identity.
    """
    if network not in ("testnet", "mainnet") or type(near_amount_yocto) is not int or near_amount_yocto <= 0:
        raise ObservationError("bad network or amount")
    if not isinstance(swap_id, str) or not re.fullmatch(r"[0-9a-f]{64}", swap_id):
        raise ObservationError("bad swap id")
    if not isinstance(hashlock, bytes) or len(hashlock) != 32:
        raise ObservationError("bad hashlock")
    for account in (vault, recipient, initiator):
        if not isinstance(account, str) or len(account) > 64 or not re.fullmatch(r"[a-z0-9_-]+(?:\.[a-z0-9_-]+)*", account):
            raise ObservationError("bad account identifier")
    if not isinstance(endpoints, list) or len(endpoints) < 2 or len(endpoints) > 5:
        raise ObservationError("two or more independent endpoints required")
    hosts = set()
    for endpoint in endpoints:
        parsed = urlparse(endpoint)
        if (parsed.scheme != "https" or not parsed.hostname or parsed.username is not None
                or parsed.password is not None or parsed.fragment or parsed.query or parsed.hostname in hosts):
            raise ObservationError("invalid or duplicate RPC host")
        hosts.add(parsed.hostname)
    payload = {"jsonrpc": "2.0", "id": "x3-near-observer", "method": "query", "params": {
        "request_type": "call_function", "finality": "final", "account_id": vault,
        "method_name": "get", "args_base64": __import__("base64").b64encode(
            json.dumps({"id": swap_id}, separators=(",", ":")).encode()).decode()}}
    rpc = fetch or _get
    replies = []
    for endpoint in endpoints:
        try:
            replies.append(_parse(rpc(endpoint, payload), vault=vault, swap_id=swap_id,
                                  recipient=recipient, initiator=initiator,
                                  hashlock=hashlock, amount=near_amount_yocto))
        except ObservationError:
            raise
        except Exception as exc:
            raise ObservationError("NEAR RPC service failed") from exc
    if any(reply != replies[0] for reply in replies[1:]):
        raise ObservationError("finalized RPC observations disagree")
    block_hash, height, _, _, _, amount, _, outcome = replies[0]
    return Observation(network, vault, swap_id, outcome, block_hash, height, amount, recipient)


class OutcomeJournal:
    """Local crash-persistent anti-equivocation record, not a distributed lock."""
    def __init__(self, path: str | Path):
        self.path = str(path)
        with self._connect() as db:
            db.execute("""CREATE TABLE IF NOT EXISTS observations (
                network TEXT NOT NULL, vault TEXT NOT NULL, swap_id TEXT NOT NULL,
                outcome TEXT NOT NULL, block_hash TEXT NOT NULL,
                PRIMARY KEY(network,vault,swap_id))""")

    def _connect(self):
        db = sqlite3.connect(self.path, timeout=10)
        db.execute("PRAGMA busy_timeout=10000")
        return db

    def record(self, network: str, vault: str, swap_id: str, outcome: str, block_hash: str) -> bool:
        if network not in ("testnet", "mainnet") or outcome not in ("Claimed", "Refunded") or not block_hash:
            raise ObservationError("not a recordable terminal outcome")
        with self._connect() as db:
            db.execute("BEGIN IMMEDIATE")
            previous = db.execute("SELECT outcome,block_hash FROM observations WHERE network=? AND vault=? AND swap_id=?",
                                  (network, vault, swap_id)).fetchone()
            if previous is not None:
                if previous != (outcome, block_hash):
                    raise ObservationError("conflicting terminal observation")
                return False
            db.execute("INSERT INTO observations VALUES(?,?,?,?,?)", (network, vault, swap_id, outcome, block_hash))
            return True

    def get(self, network: str, vault: str, swap_id: str) -> tuple[str, str] | None:
        with self._connect() as db:
            return db.execute("SELECT outcome,block_hash FROM observations WHERE network=? AND vault=? AND swap_id=?",
                              (network, vault, swap_id)).fetchone()
