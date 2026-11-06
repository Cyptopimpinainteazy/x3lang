#!/usr/bin/env python3
"""Read-only NEAR proof-evidence collection, storage, and integrity checks.

This module does not authenticate NEAR finality or authorize a settlement.
It preserves untrusted RPC responses so an independent light-client verifier
can consume them. Candidate checkpoints remain explicitly unauthenticated.
"""
from __future__ import annotations

import argparse
import copy
import hashlib
import json
import os
import re
import sys
import tempfile
import urllib.error
import urllib.request
from datetime import UTC, datetime
from pathlib import Path
from urllib.parse import urlparse

from near_receipt_proof import ReceiptProofError, get_candidate


SCHEMA = "x3.near-proof-evidence.v1"
NEGATIVE_SCHEMA = "x3.near-proof-negative-fixture.v1"
MUTATION_INDEX_SCHEMA = "x3.near-proof-mutation-index.v1"
NETWORKS = frozenset(("testnet", "mainnet"))
MAX_RESPONSE_BYTES = 8 * 1024 * 1024
BASE58_ALPHABET = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"
BASE58_RE = re.compile(r"[1-9A-HJ-NP-Za-km-z]{32,48}\Z")
ACCOUNT_RE = re.compile(r"[a-z0-9_-]+(?:\.[a-z0-9_-]+)*\Z")
HEX64_RE = re.compile(r"[0-9a-f]{64}\Z")
NEGATIVE_CASES = (
    "incorrect_block_hash",
    "wrong_receipt_id",
    "wrong_receiving_contract",
    "modified_outcome_status",
    "modified_merkle_branch",
    "incorrect_merkle_direction",
    "wrong_light_client_head",
    "wrong_network",
    "incorrect_validator_information",
    "truncated_or_malformed_proof",
    "cross_swap_evidence_substitution",
    "wrong_epoch",
)


class EvidenceError(ValueError):
    """The input cannot be preserved as valid evidence."""


class RpcUnavailable(EvidenceError):
    """An evidence provider failed or returned an RPC error."""


def _utc_now() -> str:
    return datetime.now(UTC).isoformat().replace("+00:00", "Z")


def _canonical(value: object) -> bytes:
    return json.dumps(
        value, sort_keys=True, separators=(",", ":"), ensure_ascii=False, allow_nan=False
    ).encode("utf-8")


def _sha256(raw: bytes) -> str:
    return hashlib.sha256(raw).hexdigest()


def _base58_encode(raw: bytes) -> str:
    number = int.from_bytes(raw, "big")
    encoded = ""
    while number:
        number, remainder = divmod(number, 58)
        encoded = BASE58_ALPHABET[remainder] + encoded
    zeros = len(raw) - len(raw.lstrip(b"\0"))
    return "1" * zeros + (encoded or "")


def _validate_hash(value: object, label: str = "hash") -> str:
    if type(value) is not str or not BASE58_RE.fullmatch(value):
        raise EvidenceError(f"invalid NEAR {label}")
    number = 0
    for character in value:
        number = number * 58 + BASE58_ALPHABET.index(character)
    zeros = len(value) - len(value.lstrip("1"))
    decoded = b"\0" * zeros + (
        number.to_bytes((number.bit_length() + 7) // 8, "big") if number else b""
    )
    if len(decoded) != 32:
        raise EvidenceError(f"NEAR {label} must decode to 32 bytes")
    return value


def _validate_account(value: object, label: str = "account") -> str:
    if type(value) is not str or len(value) > 64 or not ACCOUNT_RE.fullmatch(value):
        raise EvidenceError(f"invalid NEAR {label}")
    return value


def _without_duplicate_keys(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise EvidenceError(f"duplicate JSON key: {key}")
        result[key] = value
    return result


def _reject_json_constant(value):
    raise EvidenceError(f"invalid JSON constant: {value}")


def strict_json(raw: bytes) -> object:
    """Parse evidence JSON deterministically and reject ambiguous encodings."""
    try:
        return json.loads(
            raw,
            object_pairs_hook=_without_duplicate_keys,
            parse_constant=_reject_json_constant,
        )
    except (UnicodeDecodeError, json.JSONDecodeError, RecursionError) as exc:
        raise EvidenceError("malformed JSON evidence") from exc


def normalize_rpc_url(endpoint: str) -> str:
    if type(endpoint) is not str:
        raise EvidenceError("RPC URL must be text")
    parsed = urlparse(endpoint)
    try:
        port = parsed.port
    except ValueError as exc:
        raise EvidenceError("invalid RPC port") from exc
    if (
        parsed.scheme != "https"
        or not parsed.hostname
        or parsed.username is not None
        or parsed.password is not None
        or parsed.query
        or parsed.fragment
        or parsed.path not in ("", "/")
        or port not in (None, 443)
    ):
        raise EvidenceError(
            "RPC must be an HTTPS origin with no credentials, query, path, or fragment"
        )
    return f"https://{parsed.hostname.lower()}"


def validate_endpoints(endpoints: list[str]) -> list[str]:
    if not isinstance(endpoints, list) or not 2 <= len(endpoints) <= 5:
        raise EvidenceError("provide 2 to 5 RPC providers")
    normalized = [normalize_rpc_url(endpoint) for endpoint in endpoints]
    parsed = [urlparse(endpoint) for endpoint in normalized]
    hosts = [item.hostname for item in parsed]
    if any(host is None for host in hosts) or len(set(hosts)) != len(hosts):
        raise EvidenceError("RPC providers must have distinct hostnames")
    return normalized


def _read_bounded(response, limit: int = MAX_RESPONSE_BYTES) -> bytes:
    raw = response.read(limit + 1)
    if len(raw) > limit:
        raise RpcUnavailable("oversized NEAR RPC response")
    return raw


def rpc_request(
    endpoint: str,
    method: str,
    params: object,
    *,
    request_id: str,
    timeout_sec: int = 20,
) -> tuple[dict, bytes]:
    """Issue one bounded read-only JSON-RPC request and preserve its raw bytes."""
    if type(timeout_sec) is not int or not 1 <= timeout_sec <= 120:
        raise EvidenceError("RPC timeout must be between 1 and 120 seconds")
    endpoint = normalize_rpc_url(endpoint)
    payload = {
        "jsonrpc": "2.0",
        "id": request_id,
        "method": method,
        "params": params,
    }
    request = urllib.request.Request(
        endpoint,
        data=_canonical(payload),
        headers={
            "Content-Type": "application/json",
            "Accept": "application/json",
            "User-Agent": "x3lang-near-proof-evidence/1",
        },
        method="POST",
    )
    try:
        with urllib.request.urlopen(request, timeout=timeout_sec) as response:
            if normalize_rpc_url(response.geturl()) != endpoint:
                raise RpcUnavailable("NEAR RPC redirected to a different origin")
            content_type = response.headers.get("Content-Type", "").split(";", 1)[0]
            if content_type.lower() != "application/json":
                raise RpcUnavailable("NEAR RPC returned a non-JSON response")
            raw = _read_bounded(response)
    except urllib.error.HTTPError as exc:
        try:
            raw = _read_bounded(exc, 256 * 1024)
        except (OSError, TimeoutError) as read_error:
            raise RpcUnavailable("NEAR RPC error response was unreadable") from read_error
        try:
            error_body = strict_json(raw)
            error = error_body.get("error", {}) if isinstance(error_body, dict) else {}
            detail = error.get("data") or error.get("message") or f"HTTP {exc.code}"
        except (EvidenceError, AttributeError):
            detail = f"HTTP {exc.code}"
        raise RpcUnavailable(f"NEAR RPC request failed: {detail}") from exc
    except (OSError, TimeoutError) as exc:
        raise RpcUnavailable("NEAR RPC unavailable") from exc
    parsed = strict_json(raw)
    if not isinstance(parsed, dict):
        raise RpcUnavailable("NEAR RPC response is not an object")
    if parsed.get("jsonrpc") != "2.0" or parsed.get("id") != request_id:
        raise RpcUnavailable("NEAR RPC response ID mismatch")
    if parsed.get("error") is not None or "result" not in parsed:
        raise RpcUnavailable("NEAR RPC returned an error")
    return parsed, raw


def _rpc_tuple(rpc, endpoint, method, params, request_id, timeout_sec=20):
    response = rpc(
        endpoint,
        method,
        params,
        request_id=request_id,
        timeout_sec=timeout_sec,
    )
    if (
        isinstance(response, tuple)
        and len(response) == 2
        and isinstance(response[0], dict)
        and isinstance(response[1], bytes)
    ):
        return response
    if isinstance(response, dict):
        return response, _canonical(response)
    raise EvidenceError("RPC transport returned an invalid value")


class EvidenceStore:
    """Network-separated, content-addressed, append-only evidence storage."""

    def __init__(self, root: str | Path, network: str):
        if network not in NETWORKS:
            raise EvidenceError("unsupported evidence network")
        self.root = Path(root).resolve()
        self.network = network
        self.network_root = self.root / network
        self.network_root.mkdir(parents=True, exist_ok=True, mode=0o700)
        (self.network_root / "sha256").mkdir(exist_ok=True, mode=0o700)
        (self.network_root / "manifests").mkdir(exist_ok=True, mode=0o700)

    @staticmethod
    def _atomic_write(path: Path, raw: bytes) -> bool:
        path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
        if path.exists():
            if path.read_bytes() != raw:
                raise EvidenceError(f"content-addressed path collision: {path}")
            return False
        fd, temporary = tempfile.mkstemp(prefix=f".{path.name}.", dir=path.parent)
        created = True
        try:
            with os.fdopen(fd, "wb") as handle:
                handle.write(raw)
                handle.flush()
                os.fsync(handle.fileno())
            os.chmod(temporary, 0o600)
            try:
                os.link(temporary, path)
            except FileExistsError:
                created = False
                if path.read_bytes() != raw:
                    raise EvidenceError(f"content-addressed path collision: {path}")
            finally:
                os.unlink(temporary)
        except Exception:
            if os.path.exists(temporary):
                os.unlink(temporary)
            raise
        return created

    def put_response(self, response: dict, raw: bytes) -> dict:
        if strict_json(raw) != response:
            raise EvidenceError("raw response and parsed response differ")
        digest = _sha256(raw)
        relative = Path("sha256") / digest[:2] / f"{digest}.json"
        self._atomic_write(self.network_root / relative, raw)
        return {
            "path": relative.as_posix(),
            "sha256": digest,
            "bytes": len(raw),
        }

    def commit_manifest(self, manifest: dict) -> tuple[Path, bool]:
        if manifest.get("schema") != SCHEMA:
            raise EvidenceError("wrong evidence manifest schema")
        if manifest.get("network") != self.network:
            raise EvidenceError("manifest network does not match storage network")
        if manifest.get("authenticated") is not False:
            raise EvidenceError("collector output cannot be authenticated")
        if manifest.get("authorized_to_sign") is not False:
            raise EvidenceError("collector output cannot authorize signing")
        subject = {
            key: manifest[key]
            for key in (
                "schema",
                "kind",
                "network",
                "request",
                "providers",
                "candidate",
                "status",
            )
            if key in manifest
        }
        digest = _sha256(_canonical(subject))
        manifest["evidence_digest"] = digest
        manifest["manifest_id"] = digest
        raw = _canonical(manifest) + b"\n"
        relative = Path("manifests") / f"{digest}.json"
        path = self.network_root / relative
        if path.exists():
            existing = strict_json(path.read_bytes())
            if not isinstance(existing, dict) or existing.get("manifest_id") != digest:
                raise EvidenceError(f"manifest digest collision: {path}")
            manifest.clear()
            manifest.update(existing)
            return path, True
        created = self._atomic_write(path, raw)
        if not created:
            return path, True
        index = self.network_root / "index.jsonl"
        record = {
            "manifest_id": digest,
            "manifest_sha256": _sha256(raw),
            "manifest_path": relative.as_posix(),
            "kind": manifest["kind"],
            "network": self.network,
            "status": manifest["status"],
            "authenticated": False,
        }
        descriptor = os.open(index, os.O_APPEND | os.O_CREAT | os.O_WRONLY, 0o600)
        try:
            os.write(descriptor, _canonical(record) + b"\n")
            os.fsync(descriptor)
        finally:
            os.close(descriptor)
        return path, False


def _identity(status_response: dict) -> dict:
    result = status_response.get("result")
    if not isinstance(result, dict):
        raise EvidenceError("missing NEAR status result")
    chain_id = result.get("chain_id")
    if chain_id not in NETWORKS:
        raise EvidenceError("unrecognized NEAR chain identity")
    genesis = _validate_hash(result.get("genesis_hash"), "genesis hash")
    protocol = result.get("protocol_version")
    if type(protocol) is not int or protocol <= 0:
        raise EvidenceError("invalid NEAR protocol version")
    return {
        "chain_id": chain_id,
        "genesis_hash": genesis,
        "protocol_version": protocol,
    }


def _block_header(block_response: dict) -> dict:
    result = block_response.get("result")
    header = result.get("header") if isinstance(result, dict) else None
    if not isinstance(header, dict):
        raise EvidenceError("missing NEAR block header")
    height = header.get("height")
    if type(height) is not int or height <= 0:
        raise EvidenceError("invalid NEAR block height")
    return {
        "height": height,
        "hash": _validate_hash(header.get("hash"), "block hash"),
        "prev_hash": _validate_hash(header.get("prev_hash"), "previous block hash"),
        "epoch_id": _validate_hash(header.get("epoch_id"), "epoch ID"),
        "next_epoch_id": _validate_hash(header.get("next_epoch_id"), "next epoch ID"),
        "prev_state_root": _validate_hash(header.get("prev_state_root"), "state root"),
        "outcome_root": _validate_hash(header.get("outcome_root"), "outcome root"),
        "timestamp": header.get("timestamp"),
        "next_bp_hash": _validate_hash(header.get("next_bp_hash"), "next block-producer hash"),
        "block_merkle_root": _validate_hash(
            header.get("block_merkle_root"), "block Merkle root"
        ),
    }


def _light_client_block(next_response: dict) -> dict:
    result = next_response.get("result")
    if not isinstance(result, dict):
        raise EvidenceError("missing light-client block")
    prev = _validate_hash(result.get("prev_block_hash"), "previous block hash")
    next_inner = _validate_hash(result.get("next_block_inner_hash"), "next block inner hash")
    inner_rest = _validate_hash(result.get("inner_rest_hash"), "inner rest hash")
    inner = result.get("inner_lite")
    if not isinstance(inner, dict):
        raise EvidenceError("missing light-client inner header")
    normalized_inner = {
        "height": inner.get("height"),
        "epoch_id": _validate_hash(inner.get("epoch_id"), "epoch ID"),
        "next_epoch_id": _validate_hash(inner.get("next_epoch_id"), "next epoch ID"),
        "prev_state_root": _validate_hash(inner.get("prev_state_root"), "state root"),
        "outcome_root": _validate_hash(inner.get("outcome_root"), "outcome root"),
        "timestamp": inner.get("timestamp"),
        "next_bp_hash": _validate_hash(inner.get("next_bp_hash"), "next block-producer hash"),
        "block_merkle_root": _validate_hash(
            inner.get("block_merkle_root"), "block Merkle root"
        ),
    }
    if (
        type(normalized_inner["height"]) is not int
        or normalized_inner["height"] <= 0
        or type(normalized_inner["timestamp"]) is not int
        or normalized_inner["timestamp"] <= 0
    ):
        raise EvidenceError("invalid light-client height or timestamp")
    next_bps = result.get("next_bps")
    if next_bps is not None and not isinstance(next_bps, list):
        raise EvidenceError("invalid next block-producer set")
    total_stake = 0
    accounts = set()
    public_keys = set()
    if next_bps is not None:
        if not 1 <= len(next_bps) <= 1024:
            raise EvidenceError("unreasonable block-producer count")
        for validator in next_bps:
            if not isinstance(validator, dict):
                raise EvidenceError("invalid block producer")
            account = _validate_account(validator.get("account_id"), "validator account")
            public_key = validator.get("public_key")
            stake = validator.get("stake")
            if type(public_key) is not str or not public_key:
                raise EvidenceError("invalid validator public key")
            if type(stake) is not str or not re.fullmatch(r"[0-9]+", stake):
                raise EvidenceError("invalid validator stake")
            if account in accounts or public_key in public_keys:
                raise EvidenceError("duplicate validator identity")
            accounts.add(account)
            public_keys.add(public_key)
            total_stake += int(stake)
    approvals = result.get("approvals_after_next")
    if (
        not isinstance(approvals, list)
        or len(approvals) > 2048
        or any(item is not None and type(item) is not str for item in approvals)
    ):
        raise EvidenceError("invalid light-client approvals")
    raw_bps = _canonical(next_bps).decode("utf-8")
    return {
        "prev_block_hash": prev,
        "next_block_inner_hash": next_inner,
        "inner_rest_hash": inner_rest,
        "inner_lite": normalized_inner,
        "next_bps": next_bps,
        "next_bps_raw_sha256": _sha256(raw_bps.encode("utf-8")),
        "validators_count": len(next_bps) if next_bps is not None else 0,
        "total_stake": str(total_stake),
        "declared_next_bp_hash": normalized_inner["next_bp_hash"],
        "borsh_next_bps_sha256": None,
        "validator_commitment_verified": False,
        "approval_slots": len(approvals),
        "non_null_approvals": sum(item is not None for item in approvals),
    }


def _header_matches_light_client(header: dict, light: dict) -> bool:
    inner = light["inner_lite"]
    fields = (
        "height",
        "epoch_id",
        "next_epoch_id",
        "prev_state_root",
        "outcome_root",
        "timestamp",
        "next_bp_hash",
        "block_merkle_root",
    )
    return all(header.get(field) == inner.get(field) for field in fields)


def _proof_path(value: object, label: str) -> list:
    if not isinstance(value, list) or len(value) > 128:
        raise EvidenceError(f"invalid {label}")
    for branch in value:
        if (
            not isinstance(branch, dict)
            or set(branch) != {"hash", "direction"}
            or branch["direction"] not in ("Left", "Right")
        ):
            raise EvidenceError(f"invalid {label} branch")
        _validate_hash(branch["hash"], f"{label} hash")
    return value


def _proof_summary(
    response: dict, *, expected_id: str, transaction_success: bool = False
) -> dict:
    result = response.get("result")
    if not isinstance(result, dict):
        raise EvidenceError("missing light-client proof result")
    required = {"outcome_proof", "outcome_root_proof", "block_header_lite", "block_proof"}
    if required - set(result):
        raise EvidenceError("missing light-client proof component")
    outcome_proof = result["outcome_proof"]
    if not isinstance(outcome_proof, dict) or outcome_proof.get("id") != expected_id:
        raise EvidenceError("light-client proof is for a different identifier")
    _proof_path(outcome_proof.get("proof"), "outcome proof")
    _proof_path(result["outcome_root_proof"], "outcome-root proof")
    _proof_path(result["block_proof"], "block proof")
    block_hash = _validate_hash(outcome_proof.get("block_hash"), "proof block hash")
    outcome = outcome_proof.get("outcome")
    if not isinstance(outcome, dict):
        raise EvidenceError("missing execution outcome")
    executor = _validate_account(outcome.get("executor_id"), "execution account")
    status = outcome.get("status")
    if transaction_success:
        valid_status = (
            type(status) is dict
            and set(status) == {"SuccessReceiptId"}
            and type(status["SuccessReceiptId"]) is str
        )
        if valid_status:
            _validate_hash(status["SuccessReceiptId"], "success receipt ID")
    else:
        valid_status = (
            type(status) is dict
            and set(status) == {"SuccessValue"}
            and type(status["SuccessValue"]) is str
        )
    if not valid_status:
        raise EvidenceError("proof outcome is not a successful execution")
    if not isinstance(outcome.get("logs"), list) or any(
        type(item) is not str for item in outcome["logs"]
    ):
        raise EvidenceError("invalid outcome logs")
    header = result["block_header_lite"]
    if not isinstance(header, dict) or not isinstance(header.get("inner_lite"), dict):
        raise EvidenceError("missing proof block header")
    inner = header["inner_lite"]
    for field in (
        "epoch_id",
        "next_epoch_id",
        "prev_state_root",
        "outcome_root",
        "next_bp_hash",
        "block_merkle_root",
    ):
        _validate_hash(inner.get(field), field)
    if type(inner.get("height")) is not int or inner["height"] <= 0:
        raise EvidenceError("invalid proof header height")
    return {
        "id": expected_id,
        "executor_id": executor,
        "block_hash": block_hash,
        "head_height": inner["height"],
        "head_epoch_id": inner["epoch_id"],
        "next_epoch_id": inner["next_epoch_id"],
        "outcome_root": inner["outcome_root"],
        "block_merkle_root": inner["block_merkle_root"],
        "outcome_branch_length": len(outcome_proof["proof"]),
        "outcome_root_branch_length": len(result["outcome_root_proof"]),
        "block_branch_length": len(result["block_proof"]),
    }


def _tx_summary(response: dict, transaction_hash: str) -> dict:
    result = response.get("result")
    if not isinstance(result, dict):
        raise EvidenceError("missing transaction result")
    transaction_outcome = result.get("transaction_outcome")
    if not isinstance(transaction_outcome, dict) or transaction_outcome.get("id") != transaction_hash:
        raise EvidenceError("transaction response is for a different hash")
    receipts = result.get("receipts_outcome")
    if not isinstance(receipts, list):
        raise EvidenceError("missing transaction receipt outcomes")
    summaries = []
    for receipt in receipts:
        if not isinstance(receipt, dict) or not isinstance(receipt.get("outcome"), dict):
            raise EvidenceError("invalid receipt outcome")
        summaries.append(
            {
                "receipt_id": _validate_hash(receipt.get("id"), "receipt ID"),
                "executor_id": _validate_account(
                    receipt["outcome"].get("executor_id"), "receipt executor"
                ),
                "status": receipt["outcome"].get("status"),
            }
        )
    return {
        "transaction_id": transaction_hash,
        "status": transaction_outcome.get("outcome", {}).get("status"),
        "receipts": summaries,
    }


def _provider_call(method: str, params: object, request_id: str, artifact: dict) -> dict:
    return {
        "method": method,
        "params": params,
        "request_id": request_id,
        "artifact": artifact,
    }


def _manifest_status(providers: list[dict], comparisons: list[dict]) -> str:
    if any(provider.get("error") for provider in providers):
        return "incomplete"
    if any(item != comparisons[0] for item in comparisons[1:]):
        return "disagreement"
    return "complete_candidate"


def _common_limitations() -> list[str]:
    return [
        "RPC responses are untrusted and may be forged or selectively withheld.",
        "Distinct hostnames do not prove independent operators or consensus.",
        "This collector does not verify validator signatures, stake, epochs, or Borsh commitments.",
        "This collector does not authenticate Merkle paths or finalized block ancestry.",
        "No artifact is authorized to sign, settle, deploy, or move funds.",
    ]


def collect_checkpoint(
    endpoints: list[str],
    *,
    network: str,
    head_hash: str,
    store: EvidenceStore,
    rpc=rpc_request,
    timeout_sec: int = 20,
    retrieved_at: str | None = None,
) -> tuple[dict, Path, bool]:
    endpoints = validate_endpoints(endpoints)
    if network not in NETWORKS:
        raise EvidenceError("unsupported network")
    head_hash = _validate_hash(head_hash, "starting head")
    providers = []
    comparisons = []
    artifacts = {}
    for endpoint in endpoints:
        provider = {"endpoint": endpoint}
        calls = []
        try:
            status_response, status_raw = _rpc_tuple(
                rpc,
                endpoint,
                "status",
                [],
                "x3-evidence-status",
                timeout_sec,
            )
            identity = _identity(status_response)
            if identity["chain_id"] != network:
                raise EvidenceError("RPC chain identity does not match requested network")
            status_ref = store.put_response(status_response, status_raw)
            calls.append(
                _provider_call("status", [], "x3-evidence-status", status_ref)
            )

            head_params = {"block_id": head_hash}
            head_response, head_raw = _rpc_tuple(
                rpc,
                endpoint,
                "block",
                head_params,
                "x3-evidence-head-block",
                timeout_sec,
            )
            head_header = _block_header(head_response)
            if head_header["hash"] != head_hash:
                raise EvidenceError("RPC returned a different starting head")
            head_ref = store.put_response(head_response, head_raw)
            calls.append(
                _provider_call(
                    "block", head_params, "x3-evidence-head-block", head_ref
                )
            )

            next_params = [head_hash]
            next_response, next_raw = _rpc_tuple(
                rpc,
                endpoint,
                "next_light_client_block",
                next_params,
                "x3-evidence-next-light-client-block",
                timeout_sec,
            )
            light = _light_client_block(next_response)
            if light["inner_lite"]["height"] <= head_header["height"]:
                raise EvidenceError("next light-client block did not advance")
            next_ref = store.put_response(next_response, next_raw)
            calls.append(
                _provider_call(
                    "next_light_client_block",
                    next_params,
                    "x3-evidence-next-light-client-block",
                    next_ref,
                )
            )

            candidate_params = {"block_id": light["inner_lite"]["height"]}
            candidate_response, candidate_raw = _rpc_tuple(
                rpc,
                endpoint,
                "block",
                candidate_params,
                "x3-evidence-candidate-block",
                timeout_sec,
            )
            candidate_header = _block_header(candidate_response)
            if not _header_matches_light_client(candidate_header, light):
                raise EvidenceError("candidate block header does not match light-client view")
            candidate_ref = store.put_response(candidate_response, candidate_raw)
            calls.append(
                _provider_call(
                    "block",
                    candidate_params,
                    "x3-evidence-candidate-block",
                    candidate_ref,
                )
            )

            comparison = {
                "light_client_block": {
                    key: light[key]
                    for key in (
                        "prev_block_hash",
                        "next_block_inner_hash",
                        "inner_rest_hash",
                        "inner_lite",
                        "next_bps_raw_sha256",
                        "validators_count",
                        "total_stake",
                        "declared_next_bp_hash",
                        "approval_slots",
                        "non_null_approvals",
                    )
                },
                "candidate_block_hash": candidate_header["hash"],
            }
            provider.update(
                {
                    "identity": identity,
                    "comparison": comparison,
                    "candidate_block": {
                        "hash": candidate_header["hash"],
                        "height": candidate_header["height"],
                        "prev_hash": candidate_header["prev_hash"],
                    },
                    "calls": calls,
                }
            )
            comparisons.append(comparison)
            for call in calls:
                artifacts[call["artifact"]["path"]] = call["artifact"]
        except (EvidenceError, RpcUnavailable) as exc:
            provider["error"] = str(exc)
            provider["calls"] = calls
            for call in calls:
                artifacts[call["artifact"]["path"]] = call["artifact"]
        providers.append(provider)

    status = _manifest_status(providers, comparisons)
    candidate = None
    if comparisons:
        first = comparisons[0]
        light = first["light_client_block"]
        selected_provider = next(
            provider["endpoint"]
            for provider in providers
            if provider.get("comparison") == first
        )
        candidate = {
            "classification": "candidate_checkpoint_untrusted",
            "selected_provider": selected_provider,
            "provider_consensus": all(
                item == comparisons[0] for item in comparisons[1:]
            ),
            "checkpoint_id": first["candidate_block_hash"],
            "block_height": light["inner_lite"]["height"],
            "epoch_id": light["inner_lite"]["epoch_id"],
            "next_epoch_id": light["inner_lite"]["next_epoch_id"],
            "validator_set": {
                "declared_next_bp_hash": light["declared_next_bp_hash"],
                "raw_next_bps_sha256": light["next_bps_raw_sha256"],
                "borsh_next_bps_sha256": None,
                "commitment_verified": False,
                "validators_count": light["validators_count"],
                "total_stake": light["total_stake"],
            },
            "approval_slots": light["approval_slots"],
            "non_null_approvals": light["non_null_approvals"],
        }
    manifest = {
        "schema": SCHEMA,
        "kind": "checkpoint_candidate",
        "network": network,
        "retrieved_at": retrieved_at or _utc_now(),
        "request": {
            "starting_head": head_hash,
            "methods": ["status", "block", "next_light_client_block", "block"],
        },
        "providers": providers,
        "artifacts": [artifacts[path] for path in sorted(artifacts)],
        "candidate": candidate,
        "status": status,
        "authenticated": False,
        "authorized_to_sign": False,
        "limitations": _common_limitations()
        + [
            "The candidate block hash came from an RPC block response.",
            "Validator-set commitment and approval validity require an independent verifier.",
        ],
    }
    path, duplicate = store.commit_manifest(manifest)
    return manifest, path, duplicate


def collect_receipt(
    endpoints: list[str],
    *,
    network: str,
    receipt_id: str,
    receiver_id: str,
    head_hash: str,
    store: EvidenceStore,
    transaction_hash: str | None = None,
    sender_id: str | None = None,
    rpc=rpc_request,
    timeout_sec: int = 20,
    retrieved_at: str | None = None,
) -> tuple[dict, Path, bool]:
    endpoints = validate_endpoints(endpoints)
    if network not in NETWORKS:
        raise EvidenceError("unsupported network")
    receipt_id = _validate_hash(receipt_id, "receipt ID")
    head_hash = _validate_hash(head_hash, "light-client head")
    receiver_id = _validate_account(receiver_id, "receipt receiver")
    if transaction_hash is not None:
        transaction_hash = _validate_hash(transaction_hash, "transaction hash")
        sender_id = _validate_account(sender_id, "transaction sender")
    elif sender_id is not None:
        raise EvidenceError("sender-id requires transaction-hash")
    providers = []
    comparisons = []
    artifacts = {}
    proof_params = {
        "type": "receipt",
        "receipt_id": receipt_id,
        "receiver_id": receiver_id,
        "light_client_head": head_hash,
    }
    for endpoint in endpoints:
        provider = {"endpoint": endpoint}
        calls = []
        try:
            status_response, status_raw = _rpc_tuple(
                rpc,
                endpoint,
                "status",
                [],
                "x3-evidence-status",
                timeout_sec,
            )
            identity = _identity(status_response)
            if identity["chain_id"] != network:
                raise EvidenceError("RPC chain identity does not match requested network")
            status_ref = store.put_response(status_response, status_raw)
            calls.append(
                _provider_call("status", [], "x3-evidence-status", status_ref)
            )

            proof_response, proof_raw = _rpc_tuple(
                rpc,
                endpoint,
                "light_client_proof",
                proof_params,
                "x3-near-light-proof",
                timeout_sec,
            )
            parsed_candidate = get_candidate(
                proof_response,
                receipt_id=receipt_id,
                receiver_id=receiver_id,
                light_client_head=head_hash,
            )
            proof = _proof_summary(proof_response, expected_id=receipt_id)
            proof_ref = store.put_response(proof_response, proof_raw)
            calls.append(
                _provider_call(
                    "light_client_proof",
                    proof_params,
                    "x3-near-light-proof",
                    proof_ref,
                )
            )
            normalized_digest = _sha256(_canonical(proof_response["result"]))
            provider.update(
                {
                    "identity": identity,
                    "candidate": {
                        "receipt_id": parsed_candidate.receipt_id,
                        "receiver_id": parsed_candidate.receiver_id,
                        "proof_payload_sha256": parsed_candidate.payload_sha256,
                        "authenticated": False,
                    },
                    "proof": proof,
                    "calls": calls,
                }
            )
            tx_comparison = None
            if transaction_hash is not None:
                tx_params = {
                    "tx_hash": transaction_hash,
                    "sender_account_id": sender_id,
                    "wait_until": "FINAL",
                }
                tx_response, tx_raw = _rpc_tuple(
                    rpc,
                    endpoint,
                    "tx",
                    tx_params,
                    "x3-evidence-transaction",
                    timeout_sec,
                )
                transaction = _tx_summary(tx_response, transaction_hash)
                tx_ref = store.put_response(tx_response, tx_raw)
                calls.append(
                    _provider_call(
                        "tx", tx_params, "x3-evidence-transaction", tx_ref
                    )
                )
                provider["transaction"] = transaction
                tx_comparison = transaction
            comparison = {
                "proof_digest": normalized_digest,
                "proof": proof,
                "transaction": tx_comparison,
            }
            comparisons.append(comparison)
            for call in calls:
                artifacts[call["artifact"]["path"]] = call["artifact"]
        except (EvidenceError, RpcUnavailable, ReceiptProofError) as exc:
            provider["error"] = str(exc)
            provider["calls"] = calls
            for call in calls:
                artifacts[call["artifact"]["path"]] = call["artifact"]
        providers.append(provider)

    status = _manifest_status(providers, comparisons)
    candidate = None
    if comparisons and all(item == comparisons[0] for item in comparisons[1:]):
        proof = comparisons[0]["proof"]
        candidate = {
            "classification": "syntactically_valid_untrusted_receipt_proof",
            "receipt_id": receipt_id,
            "receiver_id": receiver_id,
            "transaction_hash": transaction_hash,
            "sender_id": sender_id,
            "light_client_head": head_hash,
            "proof_block_hash": proof["block_hash"],
            "proof_head_height": proof["head_height"],
            "proof_head_epoch_id": proof["head_epoch_id"],
            "proof_branch_lengths": {
                "outcome": proof["outcome_branch_length"],
                "outcome_root": proof["outcome_root_branch_length"],
                "block": proof["block_branch_length"],
            },
        }
    manifest = {
        "schema": SCHEMA,
        "kind": "receipt_light_client_proof_candidate",
        "network": network,
        "retrieved_at": retrieved_at or _utc_now(),
        "request": {
            "receipt_id": receipt_id,
            "receiver_id": receiver_id,
            "light_client_head": head_hash,
            "transaction_hash": transaction_hash,
            "sender_id": sender_id,
            "methods": ["status", "light_client_proof"]
            + (["tx"] if transaction_hash else []),
        },
        "providers": providers,
        "artifacts": [artifacts[path] for path in sorted(artifacts)],
        "candidate": candidate,
        "status": status,
        "authenticated": False,
        "authorized_to_sign": False,
        "limitations": _common_limitations()
        + [
            "A successful receipt is not proof of durable escrow storage state.",
            "This evidence does not prove Claimed, Refunded, or NotFunded storage.",
        ],
    }
    path, duplicate = store.commit_manifest(manifest)
    return manifest, path, duplicate


def collect_transaction(
    endpoints: list[str],
    *,
    network: str,
    transaction_hash: str,
    sender_id: str,
    head_hash: str,
    store: EvidenceStore,
    rpc=rpc_request,
    timeout_sec: int = 20,
    retrieved_at: str | None = None,
) -> tuple[dict, Path, bool]:
    endpoints = validate_endpoints(endpoints)
    if network not in NETWORKS:
        raise EvidenceError("unsupported network")
    transaction_hash = _validate_hash(transaction_hash, "transaction hash")
    sender_id = _validate_account(sender_id, "transaction sender")
    head_hash = _validate_hash(head_hash, "light-client head")
    params = {
        "type": "transaction",
        "transaction_hash": transaction_hash,
        "sender_id": sender_id,
        "light_client_head": head_hash,
    }
    providers = []
    comparisons = []
    artifacts = {}
    for endpoint in endpoints:
        provider = {"endpoint": endpoint}
        calls = []
        try:
            status_response, status_raw = _rpc_tuple(
                rpc,
                endpoint,
                "status",
                [],
                "x3-evidence-status",
                timeout_sec,
            )
            identity = _identity(status_response)
            if identity["chain_id"] != network:
                raise EvidenceError("RPC chain identity does not match requested network")
            status_ref = store.put_response(status_response, status_raw)
            calls.append(
                _provider_call("status", [], "x3-evidence-status", status_ref)
            )
            proof_response, proof_raw = _rpc_tuple(
                rpc,
                endpoint,
                "light_client_proof",
                params,
                "x3-near-light-proof",
                timeout_sec,
            )
            proof = _proof_summary(
                proof_response,
                expected_id=transaction_hash,
                transaction_success=True,
            )
            if proof["executor_id"] != sender_id:
                raise EvidenceError("transaction proof signer does not match sender")
            proof_ref = store.put_response(proof_response, proof_raw)
            calls.append(
                _provider_call(
                    "light_client_proof", params, "x3-near-light-proof", proof_ref
                )
            )
            comparison = {
                "proof_digest": _sha256(_canonical(proof_response["result"])),
                "proof": proof,
            }
            comparisons.append(comparison)
            provider.update({"identity": identity, "proof": proof, "calls": calls})
            for call in calls:
                artifacts[call["artifact"]["path"]] = call["artifact"]
        except (EvidenceError, RpcUnavailable) as exc:
            provider["error"] = str(exc)
            provider["calls"] = calls
            for call in calls:
                artifacts[call["artifact"]["path"]] = call["artifact"]
        providers.append(provider)
    status = _manifest_status(providers, comparisons)
    candidate = None
    if comparisons and all(item == comparisons[0] for item in comparisons[1:]):
        proof = comparisons[0]["proof"]
        candidate = {
            "classification": "syntactically_valid_untrusted_transaction_proof",
            "transaction_hash": transaction_hash,
            "sender_id": sender_id,
            "light_client_head": head_hash,
            "proof_block_hash": proof["block_hash"],
            "proof_head_height": proof["head_height"],
            "proof_head_epoch_id": proof["head_epoch_id"],
            "proof_branch_lengths": {
                "outcome": proof["outcome_branch_length"],
                "outcome_root": proof["outcome_root_branch_length"],
                "block": proof["block_branch_length"],
            },
        }
    manifest = {
        "schema": SCHEMA,
        "kind": "transaction_light_client_proof_candidate",
        "network": network,
        "retrieved_at": retrieved_at or _utc_now(),
        "request": {
            "transaction_hash": transaction_hash,
            "sender_id": sender_id,
            "light_client_head": head_hash,
            "methods": ["status", "light_client_proof"],
        },
        "providers": providers,
        "artifacts": [artifacts[path] for path in sorted(artifacts)],
        "candidate": candidate,
        "status": status,
        "authenticated": False,
        "authorized_to_sign": False,
        "limitations": _common_limitations(),
    }
    path, duplicate = store.commit_manifest(manifest)
    return manifest, path, duplicate


def _load_manifest(path: Path) -> dict:
    raw = path.read_bytes()
    manifest = strict_json(raw)
    if not isinstance(manifest, dict) or manifest.get("schema") != SCHEMA:
        raise EvidenceError(f"invalid manifest: {path}")
    if manifest.get("authenticated") is not False or manifest.get("authorized_to_sign") is not False:
        raise EvidenceError(f"manifest claims forbidden authority: {path}")
    network = manifest.get("network")
    if network not in NETWORKS:
        raise EvidenceError(f"manifest has invalid network: {path}")
    subject = {
        key: manifest[key]
        for key in (
            "schema",
            "kind",
            "network",
            "request",
            "providers",
            "candidate",
            "status",
        )
        if key in manifest
    }
    digest = _sha256(_canonical(subject))
    if manifest.get("evidence_digest") != digest or manifest.get("manifest_id") != digest:
        raise EvidenceError(f"manifest subject digest mismatch: {path}")
    if manifest.get("status") not in {
        "complete_candidate",
        "disagreement",
        "incomplete",
    }:
        raise EvidenceError(f"invalid manifest status: {path}")
    return manifest


def verify_store(root: str | Path, *, require_negative_corpus: bool = False) -> dict:
    root = Path(root).resolve()
    manifests_checked = 0
    artifacts_checked = 0
    mutations_checked = 0
    indexed = {}
    for network in sorted(NETWORKS):
        network_root = root / network
        if not network_root.exists():
            continue
        index_path = network_root / "index.jsonl"
        if not index_path.is_file():
            raise EvidenceError(f"missing evidence index: {network}")
        for line_number, line in enumerate(index_path.read_bytes().splitlines(), 1):
            if not line:
                raise EvidenceError(f"blank evidence index line: {network}:{line_number}")
            record = strict_json(line)
            if not isinstance(record, dict) or record.get("network") != network:
                raise EvidenceError(f"invalid index network separation: {network}:{line_number}")
            manifest_id = record.get("manifest_id")
            manifest_path = record.get("manifest_path")
            if (
                type(manifest_id) is not str
                or not HEX64_RE.fullmatch(manifest_id)
                or manifest_id in indexed
                or type(manifest_path) is not str
                or Path(manifest_path).is_absolute()
                or ".." in Path(manifest_path).parts
            ):
                raise EvidenceError(f"invalid or duplicate index entry: {network}:{line_number}")
            indexed[manifest_id] = record
        manifests_dir = network_root / "manifests"
        if not manifests_dir.is_dir():
            raise EvidenceError(f"missing manifest directory: {network}")
        seen_paths = set()
        for path in sorted(manifests_dir.glob("*.json")):
            raw = path.read_bytes()
            manifest = _load_manifest(path)
            manifest_id = manifest["manifest_id"]
            record = indexed.get(manifest_id)
            if record is None:
                raise EvidenceError(f"unindexed manifest: {path}")
            relative = path.relative_to(network_root).as_posix()
            if record.get("manifest_path") != relative or record.get(
                "manifest_sha256"
            ) != _sha256(raw):
                raise EvidenceError(f"manifest/index mismatch: {path}")
            if manifest_id in seen_paths:
                raise EvidenceError(f"duplicate manifest content: {path}")
            seen_paths.add(manifest_id)
            providers = manifest.get("providers")
            if not isinstance(providers, list) or not 2 <= len(providers) <= 5:
                raise EvidenceError(f"invalid provider list: {path}")
            provider_hosts = set()
            referenced_by_call = set()
            for provider in providers:
                if not isinstance(provider, dict):
                    raise EvidenceError(f"invalid provider entry: {path}")
                endpoint = normalize_rpc_url(provider.get("endpoint"))
                host = urlparse(endpoint).hostname
                if host in provider_hosts:
                    raise EvidenceError(f"duplicate provider host: {path}")
                provider_hosts.add(host)
                calls = provider.get("calls")
                if not isinstance(calls, list):
                    raise EvidenceError(f"invalid provider calls: {path}")
                for call in calls:
                    if not isinstance(call, dict) or "artifact" not in call:
                        raise EvidenceError(f"invalid provider call: {path}")
                    artifact = call["artifact"]
                    if not isinstance(artifact, dict) or type(artifact.get("path")) is not str:
                        raise EvidenceError(f"invalid provider artifact: {path}")
                    referenced_by_call.add(artifact["path"])
            artifact_paths = set()
            for artifact in manifest.get("artifacts", []):
                if not isinstance(artifact, dict):
                    raise EvidenceError(f"invalid artifact reference: {path}")
                artifact_path = artifact.get("path")
                digest = artifact.get("sha256")
                if (
                    type(artifact_path) is not str
                    or Path(artifact_path).is_absolute()
                    or ".." in Path(artifact_path).parts
                    or type(digest) is not str
                    or not HEX64_RE.fullmatch(digest)
                ):
                    raise EvidenceError(f"unsafe artifact reference: {path}")
                artifact_paths.add(artifact_path)
                candidate = network_root / artifact_path
                if not candidate.is_file():
                    raise EvidenceError(f"missing artifact: {artifact_path}")
                artifact_raw = candidate.read_bytes()
                if (
                    _sha256(artifact_raw) != digest
                    or candidate.name != f"{digest}.json"
                    or artifact.get("bytes") != len(artifact_raw)
                ):
                    raise EvidenceError(f"artifact checksum mismatch: {artifact_path}")
                strict_json(artifact_raw)
                artifacts_checked += 1
            if artifact_paths != referenced_by_call:
                raise EvidenceError(f"manifest artifact references are incomplete: {path}")
            manifests_checked += 1
        mutation_index = network_root / "mutations" / "manifest.json"
        if mutation_index.is_file():
            mutation_manifest = strict_json(mutation_index.read_bytes())
            if (
                not isinstance(mutation_manifest, dict)
                or mutation_manifest.get("schema") != MUTATION_INDEX_SCHEMA
                or mutation_manifest.get("network") != network
                or mutation_manifest.get("authenticated") is not False
            ):
                raise EvidenceError(f"invalid mutation manifest: {mutation_index}")
            cases = mutation_manifest.get("cases")
            if not isinstance(cases, dict) or set(cases) != set(NEGATIVE_CASES):
                raise EvidenceError("negative mutation corpus is incomplete")
            for case, item in cases.items():
                if not isinstance(item, dict):
                    raise EvidenceError(f"invalid mutation entry: {case}")
                relative = item.get("path")
                digest = item.get("sha256")
                if (
                    type(relative) is not str
                    or Path(relative).is_absolute()
                    or ".." in Path(relative).parts
                    or type(digest) is not str
                    or not HEX64_RE.fullmatch(digest)
                ):
                    raise EvidenceError(f"unsafe mutation entry: {case}")
                path = network_root / "mutations" / relative
                if not path.is_file() or _sha256(path.read_bytes()) != digest:
                    raise EvidenceError(f"mutation checksum mismatch: {case}")
                document = strict_json(path.read_bytes())
                if (
                    not isinstance(document, dict)
                    or document.get("schema") != NEGATIVE_SCHEMA
                    or document.get("case") != case
                    or document.get("label") != "synthetic_negative_fixture"
                    or document.get("authenticated") is not False
                ):
                    raise EvidenceError(f"mutation label mismatch: {case}")
                mutations_checked += 1
        elif require_negative_corpus:
            raise EvidenceError(f"missing negative mutation corpus: {network}")
        if len(indexed) != len(seen_paths):
            raise EvidenceError(f"index contains missing manifests: {network}")
        indexed = {}
    if not manifests_checked:
        raise EvidenceError("no evidence manifests found")
    return {
        "root": str(root),
        "manifests": manifests_checked,
        "artifacts": artifacts_checked,
        "negative_fixtures": mutations_checked,
        "authenticated": False,
        "authorized_to_sign": False,
    }


def _latest_artifact(
    root: Path, network: str, kind: str, method: str
) -> tuple[dict, dict, bytes]:
    manifests_dir = root / network / "manifests"
    candidates = []
    for path in manifests_dir.glob("*.json"):
        manifest = _load_manifest(path)
        if manifest.get("kind") != kind:
            continue
        for provider in manifest.get("providers", []):
            for call in provider.get("calls", []):
                if call.get("method") == method and "artifact" in call:
                    candidates.append((manifest, call["artifact"]))
    if not candidates:
        raise EvidenceError(f"no {kind} artifact for {method} was found")
    manifest, artifact = max(candidates, key=lambda item: item[0]["retrieved_at"])
    raw = (root / network / artifact["path"]).read_bytes()
    return manifest, artifact, raw


def generate_negative_corpus(
    root: str | Path, *, network: str
) -> tuple[Path, dict]:
    if network not in NETWORKS:
        raise EvidenceError("unsupported network")
    root = Path(root).resolve()
    proof_manifest, proof_artifact, proof_raw = _latest_artifact(
        root,
        network,
        "receipt_light_client_proof_candidate",
        "light_client_proof",
    )
    checkpoint_manifest, checkpoint_artifact, checkpoint_raw = _latest_artifact(
        root,
        network,
        "checkpoint_candidate",
        "next_light_client_block",
    )
    proof = strict_json(proof_raw)
    checkpoint = strict_json(checkpoint_raw)
    if not isinstance(proof, dict) or not isinstance(checkpoint, dict):
        raise EvidenceError("source evidence must be JSON objects")
    proof_result = proof.get("result")
    checkpoint_result = checkpoint.get("result")
    if not isinstance(proof_result, dict) or not isinstance(checkpoint_result, dict):
        raise EvidenceError("source evidence is missing results")
    if not proof_result.get("block_proof"):
        raise EvidenceError("source proof has no block Merkle branch")
    next_bps = checkpoint_result.get("next_bps")
    if not isinstance(next_bps, list) or not next_bps:
        raise EvidenceError("source checkpoint has no validator set")
    source_receipt = _validate_hash(
        proof_result["outcome_proof"].get("id"), "receipt ID"
    )
    context = {
        "network": network,
        "light_client_head": proof_manifest["request"]["light_client_head"],
        "expected_epoch": proof_manifest["candidate"]["proof_head_epoch_id"],
        "expected_validator_commitment": checkpoint_result["inner_lite"]["next_bp_hash"],
    }
    mutation_dir = root / network / "mutations"
    mutation_dir.mkdir(parents=True, exist_ok=True, mode=0o700)
    cases = {}
    for case in NEGATIVE_CASES:
        mutated = copy.deepcopy(proof)
        case_context = copy.deepcopy(context)
        source_kind = "receipt_light_client_proof"
        if case == "incorrect_block_hash":
            mutated["result"]["outcome_proof"]["block_hash"] = _base58_encode(
                hashlib.sha256(b"x3:wrong-proof-block").digest()
            )
        elif case == "wrong_receipt_id":
            mutated["result"]["outcome_proof"]["id"] = _base58_encode(
                hashlib.sha256(b"x3:wrong-receipt").digest()
            )
        elif case == "wrong_receiving_contract":
            mutated["result"]["outcome_proof"]["outcome"][
                "executor_id"
            ] = "wrong-contract.testnet"
        elif case == "modified_outcome_status":
            mutated["result"]["outcome_proof"]["outcome"]["status"] = {
                "Failure": {"ActionError": {"index": 0}}
            }
        elif case == "modified_merkle_branch":
            mutated["result"]["block_proof"][0]["hash"] = _base58_encode(
                hashlib.sha256(b"x3:wrong-merkle-sibling").digest()
            )
        elif case == "incorrect_merkle_direction":
            current = mutated["result"]["block_proof"][0]["direction"]
            mutated["result"]["block_proof"][0]["direction"] = (
                "Left" if current == "Right" else "Right"
            )
        elif case == "wrong_light_client_head":
            case_context["light_client_head"] = _base58_encode(
                hashlib.sha256(b"x3:wrong-light-client-head").digest()
            )
        elif case == "wrong_network":
            case_context["network"] = "mainnet"
        elif case == "incorrect_validator_information":
            mutated = copy.deepcopy(checkpoint)
            source_kind = "next_light_client_block_candidate"
            mutated["result"]["next_bps"][0]["stake"] = str(
                int(mutated["result"]["next_bps"][0]["stake"]) + 1
            )
        elif case == "truncated_or_malformed_proof":
            mutated["result"].pop("outcome_root_proof")
        elif case == "cross_swap_evidence_substitution":
            replacement = _base58_encode(
                hashlib.sha256(b"x3:different-swap-receipt").digest()
            )
            mutated["result"]["outcome_proof"]["id"] = replacement
            mutated["result"]["outcome_proof"]["outcome"]["receipt_ids"] = [
                replacement
            ]
            case_context["expected_receipt_id"] = source_receipt
        elif case == "wrong_epoch":
            mutated["result"]["block_header_lite"]["inner_lite"][
                "epoch_id"
            ] = _base58_encode(hashlib.sha256(b"x3:wrong-epoch").digest())
        source_artifact = (
            checkpoint_artifact
            if source_kind == "next_light_client_block_candidate"
            else proof_artifact
        )
        document = {
            "schema": NEGATIVE_SCHEMA,
            "label": "synthetic_negative_fixture",
            "case": case,
            "source_kind": source_kind,
            "source_artifact_sha256": source_artifact["sha256"],
            "authenticated": False,
            "authorized_to_sign": False,
            "mutation_description": (
                "Synthetic mutation of genuine RPC evidence; not chain data and "
                "not a claim about production-light-client acceptance."
            ),
            "context": case_context,
            "payload": mutated,
        }
        raw = _canonical(document) + b"\n"
        relative = f"{case}.json"
        path = mutation_dir / relative
        EvidenceStore._atomic_write(path, raw)
        cases[case] = {
            "path": relative,
            "sha256": _sha256(raw),
            "source_artifact_sha256": source_artifact["sha256"],
        }
    manifest = {
        "schema": MUTATION_INDEX_SCHEMA,
        "label": "synthetic_negative_corpus",
        "network": network,
        "authenticated": False,
        "cases": cases,
        "source_manifests": [
            proof_manifest["manifest_id"],
            checkpoint_manifest["manifest_id"],
        ],
        "limitation": (
            "These are deliberate mutations derived from real RPC artifacts. "
            "They do not assert that the independent production verifier was run."
        ),
    }
    manifest_path = mutation_dir / "manifest.json"
    EvidenceStore._atomic_write(manifest_path, _canonical(manifest) + b"\n")
    return manifest_path, manifest


def _print_result(value: dict) -> None:
    print(json.dumps(value, indent=2, sort_keys=True))


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)

    checkpoint = subparsers.add_parser(
        "collect-checkpoint", help="collect an untrusted checkpoint candidate"
    )
    checkpoint.add_argument("--rpc", dest="rpc_urls", action="append", required=True)
    checkpoint.add_argument("--network", choices=sorted(NETWORKS), default="testnet")
    checkpoint.add_argument("--head-hash", required=True)
    checkpoint.add_argument("--out", required=True)
    checkpoint.add_argument("--timeout-sec", type=int, default=20)

    receipt = subparsers.add_parser(
        "collect-receipt", help="collect an untrusted receipt light-client proof"
    )
    receipt.add_argument("--rpc", dest="rpc_urls", action="append", required=True)
    receipt.add_argument("--network", choices=sorted(NETWORKS), default="testnet")
    receipt.add_argument("--receipt-id", required=True)
    receipt.add_argument("--receiver-id", required=True)
    receipt.add_argument("--head-hash", required=True)
    receipt.add_argument("--transaction-hash")
    receipt.add_argument("--sender-id")
    receipt.add_argument("--out", required=True)
    receipt.add_argument("--timeout-sec", type=int, default=20)

    transaction = subparsers.add_parser(
        "collect-transaction", help="collect an untrusted transaction light-client proof"
    )
    transaction.add_argument("--rpc", dest="rpc_urls", action="append", required=True)
    transaction.add_argument("--network", choices=sorted(NETWORKS), default="testnet")
    transaction.add_argument("--transaction-hash", required=True)
    transaction.add_argument("--sender-id", required=True)
    transaction.add_argument("--head-hash", required=True)
    transaction.add_argument("--out", required=True)
    transaction.add_argument("--timeout-sec", type=int, default=20)

    mutations = subparsers.add_parser(
        "generate-negative-corpus",
        help="derive labelled adversarial copies from collected real evidence",
    )
    mutations.add_argument("--root", required=True)
    mutations.add_argument("--network", choices=sorted(NETWORKS), default="testnet")

    verify = subparsers.add_parser("verify", help="offline integrity verification")
    verify.add_argument("--root", required=True)
    verify.add_argument("--require-negative-corpus", action="store_true")

    args = parser.parse_args(argv)
    try:
        if args.command == "collect-checkpoint":
            store = EvidenceStore(args.out, args.network)
            manifest, path, duplicate = collect_checkpoint(
                args.rpc_urls,
                network=args.network,
                head_hash=args.head_hash,
                store=store,
                timeout_sec=args.timeout_sec,
            )
            _print_result(
                {
                    "manifest": str(path),
                    "manifest_id": manifest["manifest_id"],
                    "status": manifest["status"],
                    "duplicate": duplicate,
                    "authenticated": False,
                }
            )
            return {
                "complete_candidate": 0,
                "disagreement": 3,
                "incomplete": 4,
            }[manifest["status"]]
        if args.command == "collect-receipt":
            store = EvidenceStore(args.out, args.network)
            manifest, path, duplicate = collect_receipt(
                args.rpc_urls,
                network=args.network,
                receipt_id=args.receipt_id,
                receiver_id=args.receiver_id,
                head_hash=args.head_hash,
                transaction_hash=args.transaction_hash,
                sender_id=args.sender_id,
                store=store,
                timeout_sec=args.timeout_sec,
            )
            _print_result(
                {
                    "manifest": str(path),
                    "manifest_id": manifest["manifest_id"],
                    "status": manifest["status"],
                    "duplicate": duplicate,
                    "authenticated": False,
                }
            )
            return {
                "complete_candidate": 0,
                "disagreement": 3,
                "incomplete": 4,
            }[manifest["status"]]
        if args.command == "collect-transaction":
            store = EvidenceStore(args.out, args.network)
            manifest, path, duplicate = collect_transaction(
                args.rpc_urls,
                network=args.network,
                transaction_hash=args.transaction_hash,
                sender_id=args.sender_id,
                head_hash=args.head_hash,
                store=store,
                timeout_sec=args.timeout_sec,
            )
            _print_result(
                {
                    "manifest": str(path),
                    "manifest_id": manifest["manifest_id"],
                    "status": manifest["status"],
                    "duplicate": duplicate,
                    "authenticated": False,
                }
            )
            return {
                "complete_candidate": 0,
                "disagreement": 3,
                "incomplete": 4,
            }[manifest["status"]]
        if args.command == "generate-negative-corpus":
            path, manifest = generate_negative_corpus(
                args.root, network=args.network
            )
            _print_result(
                {
                    "manifest": str(path),
                    "cases": len(manifest["cases"]),
                    "authenticated": False,
                }
            )
            return 0
        if args.command == "verify":
            _print_result(
                verify_store(
                    args.root,
                    require_negative_corpus=args.require_negative_corpus,
                )
            )
            return 0
    except (EvidenceError, ReceiptProofError) as exc:
        print(f"evidence error: {exc}", file=sys.stderr)
        return 2
    raise AssertionError("unreachable")


if __name__ == "__main__":
    sys.exit(main())
