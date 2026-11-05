"""NEAR light-client *proof acquisition* only, not proof verification.

A light_client_proof JSON response is untrusted input, even from two RPCs.
No function in this module authenticates validator signatures, state roots,
merkle branches, code identity, or payment. No signing capability exists.
"""
from __future__ import annotations

import hashlib
import json
import re
from dataclasses import dataclass
from urllib.parse import urlparse

from near_observer import _get


class ReceiptProofError(ValueError):
    pass


BASE58 = re.compile(r"[1-9A-HJ-NP-Za-km-z]{32,48}\Z")
ACCOUNT = re.compile(r"[a-z0-9_-]+(?:\.[a-z0-9_-]+)*\Z")


def _b58(s):
    if type(s) is not str or not BASE58.fullmatch(s):
        raise ReceiptProofError("invalid NEAR base58 identifier")
    # Structural format check; exact 32 decoded bytes. Do not trust text length.
    n = 0
    alphabet = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"
    for c in s:
        n = n * 58 + alphabet.index(c)
    zeros = len(s) - len(s.lstrip("1"))
    decoded = b"\x00" * zeros + (n.to_bytes((n.bit_length()+7)//8,'big') if n else b'')
    if len(decoded) != 32:
        raise ReceiptProofError("NEAR hash must decode to 32 bytes")
    return decoded


def _path(x):
    if type(x) is not list or len(x) > 128:
        raise ReceiptProofError("invalid or oversized merkle path")
    for node in x:
        if type(node) is not dict or set(node) != {"hash", "direction"} or node["direction"] not in ("Left", "Right"):
            raise ReceiptProofError("invalid merkle branch")
        _b58(node["hash"])


@dataclass(frozen=True)
class UnauthenticatedReceiptCandidate:
    receipt_id: str
    receiver_id: str
    light_client_head: str
    block_hash: str
    payload_sha256: str
    authenticated: bool = False
    authorized_to_sign: bool = False


def get_candidate(rpc_response, *, receipt_id: str, receiver_id: str, light_client_head: str):
    """Reject syntactically inconsistent proof *claims*. No proof is verified."""
    _b58(receipt_id)
    _b58(light_client_head)
    if type(receiver_id) is not str or len(receiver_id) > 64 or not ACCOUNT.fullmatch(receiver_id):
        raise ReceiptProofError("invalid NEAR receiver")
    if type(rpc_response) is not dict or rpc_response.get("id") != "x3-near-light-proof" or rpc_response.get("error") is not None:
        raise ReceiptProofError("RPC request mismatch or error")
    r = rpc_response.get("result")
    if type(r) is not dict or set(("outcome_proof","outcome_root_proof","block_header_lite","block_proof")) - set(r):
        raise ReceiptProofError("missing light-client proof components")
    op = r["outcome_proof"]
    if type(op) is not dict or op.get("id") != receipt_id:
        raise ReceiptProofError("unrelated receipt")
    _b58(op.get("block_hash"))
    _path(op.get("proof"))
    _path(r["outcome_root_proof"])
    _path(r["block_proof"])
    header = r["block_header_lite"]
    if type(header) is not dict or type(header.get("inner_lite")) is not dict:
        raise ReceiptProofError("invalid block header")
    _b58(header.get("prev_block_hash"))
    _b58(header.get("inner_rest_hash"))
    inner = header["inner_lite"]
    for field in ("epoch_id", "next_epoch_id", "prev_state_root", "outcome_root", "next_bp_hash", "block_merkle_root"):
        _b58(inner.get(field))
    if type(inner.get("height")) is not int or inner["height"] <= 0:
        raise ReceiptProofError("invalid header height")
    outcome = op.get("outcome")
    if type(outcome) is not dict or outcome.get("executor_id") != receiver_id:
        raise ReceiptProofError("receipt executed on wrong contract")
    status = outcome.get("status")
    if type(status) is not dict or set(status) != {"SuccessValue"} or type(status["SuccessValue"]) is not str:
        raise ReceiptProofError("receipt execution not final success")
    if type(outcome.get("logs")) is not list or any(type(x) is not str for x in outcome["logs"]):
        raise ReceiptProofError("malformed outcome logs")
    raw = json.dumps(r, sort_keys=True, separators=(",", ":"), allow_nan=False).encode("utf-8")
    if len(raw) > 1_000_000:
        raise ReceiptProofError("oversized proof")
    return UnauthenticatedReceiptCandidate(receipt_id, receiver_id, light_client_head,
                                           op["block_hash"], hashlib.sha256(raw).hexdigest())


def request_receipt_proof(endpoint: str, *, receipt_id: str, receiver_id: str,
                          light_client_head: str, fetch=None):
    endpoint_url = urlparse(endpoint)
    if (endpoint_url.scheme != "https" or not endpoint_url.hostname or
            endpoint_url.username is not None or endpoint_url.password is not None or
            endpoint_url.query or endpoint_url.fragment):
        raise ReceiptProofError("HTTPS RPC endpoint required")
    _b58(receipt_id)
    _b58(light_client_head)
    if type(receiver_id) is not str or not ACCOUNT.fullmatch(receiver_id) or len(receiver_id) > 64:
        raise ReceiptProofError("invalid receiver")
    payload = {"jsonrpc":"2.0", "id":"x3-near-light-proof", "method":"light_client_proof",
               "params":{"type":"receipt", "receipt_id":receipt_id,
                         "receiver_id":receiver_id, "light_client_head":light_client_head}}
    try:
        response = (fetch or _get)(endpoint, payload)
    except Exception as exc:
        raise ReceiptProofError("RPC proof fetch failed") from exc
    return get_candidate(response,receipt_id=receipt_id,receiver_id=receiver_id,
                         light_client_head=light_client_head)


def authorize_receipt(candidate: UnauthenticatedReceiptCandidate):
    """Intentionally unimplemented trust boundary. NO FINANCIAL AUTHORITY."""
    raise ReceiptProofError("NEAR validator/light-client proof verification not implemented; signing forbidden")
