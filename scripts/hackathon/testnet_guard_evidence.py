#!/usr/bin/env python3
"""Testnet Guard evidence collector (read-only RPC, no signing).

Consumes a NEAR testnet RPC and a locally built Guard Wasm, and records
sanitized, content-hashed evidence that can be independently re-verified
with ``--verify``.  It never loads keys, never signs, and never moves value.

Chain identity: the brief requires ``EXPERIMENTAL_chain``. Public NEAR RPCs
currently answer ``METHOD_NOT_FOUND`` for that method; the collector tries
it first and, on that specific answer, falls back to ``status`` (which
carries the same ``chain_id``) so real testnet RPCs work.  Any other
error, or a chain id other than ``testnet``, is an ``EvidenceError``.
"""
from __future__ import annotations

import argparse
import base64
import hashlib
import json
import re
import sys
from datetime import datetime, timezone
from pathlib import Path
from urllib.error import HTTPError, URLError
from urllib.parse import urlparse
from urllib.request import Request, urlopen

SCHEMA_MANIFEST = "x3.guard.testnet.manifest.v1"
SCHEMA_TRANSACTION = "x3.guard.testnet.transaction.v1"
SCHEMA_COLLECTION = "x3.guard.testnet.collection.v1"
DEFAULT_RPC_URL = "https://test.rpc.fastnear.com"
MAX_RPC_BYTES = 8 * 1024 * 1024
BASE58_ALPHABET = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"
FORBIDDEN_KEYS = {
    "private_key", "privatekey", "secret_key", "secretkey", "seed", "seed_phrase",
    "mnemonic", "passphrase", "jwt", "access_token", "api_key", "apikey",
    "env", "environment", "private", "secret_key_base64",
}
_ACCOUNT_RE = re.compile(r"[a-z0-9][a-z0-9._-]{1,63}\Z")


class EvidenceError(ValueError):
    """Malformed or RPC-mismatched evidence."""


def _canonical(value: object) -> bytes:
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False, allow_nan=False).encode("utf-8")


def _canonical_hash(value: object) -> str:
    return hashlib.sha256(_canonical(value)).hexdigest()


def _utc_now() -> str:
    return datetime.now(timezone.utc).isoformat(timespec="seconds").replace("+00:00", "Z")


def _reject_sensitive(value: object, path: str = "") -> None:
    if isinstance(value, dict):
        for k, v in value.items():
            lk = str(k).lower()
            if lk in FORBIDDEN_KEYS or "private_key" in lk or lk == "seed_phrase":
                raise EvidenceError(f"private-key field not allowed in evidence at {path or k}: private")
            # Environment data: reject whole env dumps.
            if lk in ("env", "environment_variables", "process_env"):
                raise EvidenceError(f"environment data not allowed in evidence at {path or k}: private")
            _reject_sensitive(v, f"{path}.{k}" if path else str(k))
    elif isinstance(value, list):
        for i, item in enumerate(value):
            _reject_sensitive(item, f"{path}[{i}]")
    elif isinstance(value, str):
        # A raw ed25519 secret is never evidence; public keys are allowed.
        # Reject strings that look like unprefixed 64-byte base64 private keys
        # only when under a sensitive-looking key handled above.
        pass


def _validate_url(rpc_url: str) -> str:
    parsed = urlparse(str(rpc_url))
    host = parsed.hostname or ""
    is_local = host in ("127.0.0.1", "localhost", "::1")
    if parsed.scheme == "https" and host:
        return str(rpc_url).rstrip("/")
    if parsed.scheme == "http" and is_local:
        return str(rpc_url).rstrip("/")
    raise EvidenceError(f"RPC URL must be HTTPS for non-local execution, got {rpc_url!r}")


def _rpc(rpc_url: str, method: str, params: object) -> dict:
    url = _validate_url(rpc_url)
    payload = {"jsonrpc": "2.0", "id": "x3-guard-evidence", "method": method, "params": params}
    body = json.dumps(payload, separators=(",", ":"), sort_keys=True).encode("utf-8")
    req = Request(url, data=body, headers={"Content-Type": "application/json", "User-Agent": "X3GuardTestnetEvidence/1.0"}, method="POST")
    try:
        with urlopen(req, timeout=20) as resp:  # noqa: S310
            raw = resp.read(MAX_RPC_BYTES + 1)
    except HTTPError as exc:
        try:
            raw = exc.read(MAX_RPC_BYTES + 1)
        except Exception:
            raw = b""
        try:
            data = json.loads(raw)
            err = data.get("error") if isinstance(data, dict) else None
            detail = err.get("message", "") if isinstance(err, dict) else ""
            cause = ""
            if isinstance(err, dict):
                cause_obj = err.get("cause")
                if isinstance(cause_obj, dict):
                    cause = cause_obj.get("name", "")
            raise EvidenceError(f"RPC error for {method}: {cause} {detail}".strip()) from exc
        except EvidenceError:
            raise
        except Exception:
            raise EvidenceError(f"RPC unavailable for {method}: HTTP {exc.code}") from exc
    except (URLError, OSError, TimeoutError) as exc:
        raise EvidenceError(f"RPC unavailable for {method}: {exc}") from exc
    if len(raw) > MAX_RPC_BYTES:
        raise EvidenceError(f"RPC response too large for {method}")
    try:
        data = json.loads(raw)
    except ValueError as exc:
        raise EvidenceError(f"malformed RPC JSON for {method}") from exc
    if not isinstance(data, dict):
        raise EvidenceError(f"RPC response not an object for {method}")
    if data.get("error") is not None:
        err = data["error"]
        detail = err.get("message", "") if isinstance(err, dict) else str(err)
        cause = ""
        if isinstance(err, dict):
            cause_obj = err.get("cause")
            if isinstance(cause_obj, dict):
                cause = cause_obj.get("name", "")
        raise EvidenceError(f"RPC error for {method}: {cause} {detail}".strip())
    result = data.get("result")
    if not isinstance(result, dict):
        # Some methods return a string; normalize to dict for callers.
        if isinstance(result, str):
            return {"_string_result": result}
        raise EvidenceError(f"RPC result missing for {method}")
    return result


def _extract_chain_id(result: dict) -> str | None:
    if "chain_id" in result:
        return str(result["chain_id"])
    if "_string_result" in result:
        return str(result["_string_result"])
    return None


def _require_testnet(rpc_url: str) -> str:
    """Query EXPERIMENTAL_chain, require testnet; fall back to status on METHOD_NOT_FOUND."""
    # Try EXPERIMENTAL_chain first per brief.
    try:
        result = _rpc(rpc_url, "EXPERIMENTAL_chain", [])
        chain_id = _extract_chain_id(result)
        if chain_id is None:
            raise EvidenceError("chain identity missing from EXPERIMENTAL_chain response")
        if chain_id != "testnet":
            raise EvidenceError(f"chain id mismatch: expected testnet, got {chain_id!r}")
        return chain_id
    except EvidenceError as exc:
        msg = str(exc)
        # Real public RPCs answer METHOD_NOT_FOUND; fall back to status.
        if "METHOD_NOT_FOUND" in msg or "Method not found" in msg or "method not found" in msg.lower():
            status = _rpc(rpc_url, "status", [])
            chain_id = _extract_chain_id(status)
            if chain_id is None:
                raise EvidenceError("chain identity missing from status response")
            if chain_id != "testnet":
                raise EvidenceError(f"chain id mismatch: expected testnet, got {chain_id!r}")
            return chain_id
        raise


def _base58_decode(text: str) -> bytes:
    alphabet = BASE58_ALPHABET
    num = 0
    for ch in text:
        if ch not in alphabet:
            raise EvidenceError(f"invalid base58 in code hash: {text!r}")
        num = num * 58 + alphabet.index(ch)
    zeros = len(text) - len(text.lstrip("1"))
    body = num.to_bytes((num.bit_length() + 7) // 8, "big") if num else b""
    return b"\x00" * zeros + body


def _sanitize_args_value(key: str, value: object) -> object:
    lk = key.lower()
    if lk in FORBIDDEN_KEYS or "private_key" in lk:
        raise EvidenceError(f"private-key field not allowed in transaction args: {key}")
    if lk in ("signature", "signed_payload", "signedpayload"):
        raw = value if isinstance(value, str) else _canonical(value).decode("utf-8")
        return {"sha256": hashlib.sha256(raw.encode("utf-8")).hexdigest(), "preview": raw[:8], "len": len(raw)}
    if isinstance(value, dict):
        return {k: _sanitize_args_value(str(k), v) for k, v in value.items()}
    if isinstance(value, list):
        return [_sanitize_args_value(key, v) for v in value]
    return value


def _parse_actions(actions: object) -> tuple[list[str], list[dict], str]:
    methods: list[str] = []
    sanitized: list[dict] = []
    if not isinstance(actions, list):
        return methods, sanitized, hashlib.sha256(b"[]").hexdigest()
    for action in actions:
        method_name = None
        args_b64 = None
        if isinstance(action, str):
            method_name = action
        elif isinstance(action, dict):
            if "FunctionCall" in action and isinstance(action["FunctionCall"], dict):
                fc = action["FunctionCall"]
                method_name = str(fc.get("method_name") or "FunctionCall")
                args_b64 = fc.get("args")
            elif "method_name" in action:
                method_name = str(action["method_name"])
                args_b64 = action.get("args")
            elif len(action) == 1:
                k, v = next(iter(action.items()))
                method_name = str(k)
                if isinstance(v, dict):
                    args_b64 = v.get("args")
            else:
                method_name = ",".join(sorted(str(k) for k in action.keys()))
        else:
            method_name = str(action)
        if method_name:
            methods.append(method_name)
        entry: dict = {"method_name": method_name}
        if args_b64 is not None:
            try:
                raw = base64.b64decode(str(args_b64), validate=False) if isinstance(args_b64, str) else b""
            except Exception:
                raw = b""
            try:
                decoded = json.loads(raw) if raw else {}
            except Exception:
                decoded = {"_raw_len": len(raw)}
            if isinstance(decoded, dict):
                entry["args_keys"] = sorted(str(k) for k in decoded.keys())
                entry["sanitized_args"] = _sanitize_args_value("args", decoded)
            else:
                entry["args_keys"] = []
                entry["sanitized_args"] = decoded
            entry["args_sha256"] = _canonical_hash(entry["sanitized_args"])
        else:
            entry["args_keys"] = []
            entry["sanitized_args"] = {}
            entry["args_sha256"] = _canonical_hash({})
        sanitized.append(entry)
    combined = _canonical_hash(sanitized)
    return methods, sanitized, combined


def record_manifest(account_id: str, wasm: Path, rpc_url: str) -> dict:
    """Record contract account + code identity for the deployed Guard."""
    _validate_url(rpc_url)
    if not _ACCOUNT_RE.fullmatch(str(account_id)):
        raise EvidenceError(f"invalid account_id: {account_id!r}")
    wasm = Path(wasm)
    if not wasm.is_file():
        raise EvidenceError(f"wasm file missing: {wasm}")
    raw = wasm.read_bytes()
    if not raw.startswith(b"\x00asm"):
        raise EvidenceError("wasm file is not a WebAssembly binary")
    wasm_sha = hashlib.sha256(raw).hexdigest()
    wasm_bytes = len(raw)
    chain_id = _require_testnet(rpc_url)
    # Status for height/hash, view_account for code_hash at final.
    status = _rpc(rpc_url, "status", [])
    sync = status.get("sync_info") if isinstance(status.get("sync_info"), dict) else {}
    block_height = sync.get("latest_block_height") if isinstance(sync, dict) else status.get("latest_block_height")
    block_hash = sync.get("latest_block_hash") if isinstance(sync, dict) else status.get("latest_block_hash")
    if not isinstance(block_height, int) or block_height <= 0:
        # Fall back to EXPERIMENTAL_chain-style status height field.
        raise EvidenceError("missing block height from RPC status")
    view = _rpc(rpc_url, "query", {"request_type": "view_account", "account_id": str(account_id), "finality": "final"})
    code_hash = view.get("code_hash")
    if not isinstance(code_hash, str):
        raise EvidenceError("missing code_hash from view_account")
    # NEAR code-hash semantics: deployed code_hash is base58(sha256(code)).
    # Empty code uses the 111... sentinel and must not be treated as wasm.
    empty = code_hash == "11111111111111111111111111111111"
    if not empty:
        decoded = _base58_decode(code_hash)
        if len(decoded) != 32 or decoded.hex() != wasm_sha:
            raise EvidenceError("code_hash does not match wasm sha256 (chain code identity mismatch)")
    record: dict = {
        "schema": SCHEMA_MANIFEST,
        "chain_id": chain_id,
        "account_id": str(account_id),
        "code_hash": code_hash,
        "wasm_sha256": wasm_sha,
        "wasm_bytes": wasm_bytes,
        "block_height": block_height,
        "block_hash": block_hash,
        "observed_at_utc": _utc_now(),
        "rpc_url": _validate_url(rpc_url),
    }
    _reject_sensitive(record)
    record["record_hash"] = _canonical_hash({k: v for k, v in record.items() if k != "record_hash"})
    return record


def record_transaction(tx_hash: str, rpc_url: str, sender_id: str | None = None) -> dict:
    """Record one finalized (or failed) testnet transaction's sanitized facts."""
    _validate_url(rpc_url)
    tx_hash = str(tx_hash).strip()
    if not tx_hash:
        raise EvidenceError("transaction hash must not be empty")
    # Sender is the signer account; guard calls are self-signed by the guard account.
    signer = sender_id or "x3lang-guard-20261106.testnet"
    if not _ACCOUNT_RE.fullmatch(str(signer)):
        raise EvidenceError(f"invalid signer_id: {signer!r}")
    chain_id = _require_testnet(rpc_url)
    result = _rpc(rpc_url, "tx", [tx_hash, signer])
    final_status = result.get("final_execution_status")
    if final_status != "FINAL":
        raise EvidenceError(f"transaction not included in a final block (final_execution_status={final_status!r})")
    tx_outcome = result.get("transaction_outcome")
    if not isinstance(tx_outcome, dict):
        raise EvidenceError("missing transaction_outcome in RPC response")
    outcome_id = tx_outcome.get("id")
    if outcome_id != tx_hash:
        raise EvidenceError(f"transaction outcome id mismatch for {tx_hash!r}")
    block_hash = tx_outcome.get("block_hash")
    # Block height: prefer execution_info, else resolve block by hash.
    block_height = None
    exec_info = result.get("execution_info")
    if isinstance(exec_info, dict) and isinstance(exec_info.get("block_height"), int):
        block_height = exec_info["block_height"]
    else:
        if not isinstance(block_hash, str):
            raise EvidenceError("missing block hash in transaction outcome")
        block_result = _rpc(rpc_url, "block", {"block_id": block_hash})
        header = block_result.get("header") if isinstance(block_result.get("header"), dict) else {}
        if not isinstance(header.get("height"), int):
            raise EvidenceError("missing block height from block header")
        block_height = header["height"]
        if header.get("hash") != block_hash:
            raise EvidenceError("block hash mismatch between transaction and header")
    transaction = result.get("transaction") if isinstance(result.get("transaction"), dict) else {}
    receiver_id = transaction.get("receiver_id") if isinstance(transaction, dict) else None
    # Overall status (canonical string form).
    status_obj = result.get("status")
    status_str = _canonical(status_obj).decode("utf-8") if status_obj is not None else ""
    receipts = result.get("receipts_outcome") if isinstance(result.get("receipts_outcome"), list) else []
    receipt_statuses = []
    receipt_errors = []
    for r in receipts:
        if not isinstance(r, dict):
            continue
        outcome = r.get("outcome") if isinstance(r.get("outcome"), dict) else {}
        st = outcome.get("status") if isinstance(outcome, dict) else None
        receipt_statuses.append({"receipt_id": r.get("id"), "status": st})
        if isinstance(st, dict) and "Failure" in st:
            receipt_errors.append(str(st["Failure"])[:500])
    methods, sanitized, args_hash = _parse_actions(transaction.get("actions") if isinstance(transaction, dict) else [])
    record: dict = {
        "schema": SCHEMA_TRANSACTION,
        "chain_id": chain_id,
        "transaction_hash": tx_hash,
        "tx_hash": tx_hash,
        "signer_id": signer,
        "receiver_id": receiver_id,
        "block_height": block_height,
        "block_hash": block_hash,
        "final_execution_status": final_status,
        "status": status_str,
        "receipt_statuses": receipt_statuses,
        "receipt_errors": receipt_errors,
        "method_names": methods,
        "sanitized_args": sanitized,
        "args_sha256": args_hash,
        "observed_at_utc": _utc_now(),
        "rpc_url": _validate_url(rpc_url),
    }
    _reject_sensitive(record)
    record["record_hash"] = _canonical_hash({k: v for k, v in record.items() if k != "record_hash"})
    return record


def _check_record_hash(record: dict, label: str) -> None:
    stored = record.get("record_hash")
    if not isinstance(stored, str) or len(stored) != 64:
        raise EvidenceError(f"missing record_hash in {label}")
    expect = _canonical_hash({k: v for k, v in record.items() if k != "record_hash"})
    if stored != expect:
        raise EvidenceError(f"record_hash mismatch in {label}: evidence has been tampered")


def _verify_transaction_record(stored: dict, rpc_url: str) -> str:
    """Re-query one transaction and compare the full fresh record.

    Excludes observation time, record hash, collection label, and RPC URL
    (the verifying RPC may differ from the recording RPC). Every other field
    — chain_id, signer/receiver, receipt statuses/errors, sanitized args —
    must equal the fresh RPC record, and the stored ``args_sha256`` must
    equal the recomputed hash of the stored ``sanitized_args``.
    """
    tx_hash = stored.get("transaction_hash") or stored.get("tx_hash")
    if not isinstance(tx_hash, str):
        raise EvidenceError("transaction hash missing in evidence")
    signer = stored.get("signer_id")
    if stored.get("args_sha256") is not None and _canonical_hash(stored.get("sanitized_args")) != stored.get("args_sha256"):
        raise EvidenceError("args_sha256 mismatch: stored hash does not match sanitized args")
    fresh = record_transaction(tx_hash, rpc_url, sender_id=signer if isinstance(signer, str) else None)
    if fresh.get("args_sha256") is not None and _canonical_hash(fresh.get("sanitized_args")) != fresh.get("args_sha256"):
        raise EvidenceError("args_sha256 mismatch in fresh RPC record")
    excluded = {"observed_at_utc", "record_hash", "label", "rpc_url"}
    for key in sorted(set(stored) | set(fresh)):
        if key in excluded:
            continue
        if stored.get(key) != fresh.get(key):
            raise EvidenceError(f"transaction {key} mismatch versus RPC")
    return tx_hash


def verify_evidence(path: Path, rpc_url: str, wasm: Path | None = None) -> dict:
    """Re-query RPC and fail on any mismatch with the recorded evidence."""
    _validate_url(rpc_url)
    path = Path(path)
    if not path.is_file():
        raise EvidenceError(f"evidence file missing: {path}")
    raw = path.read_bytes()
    try:
        data = json.loads(raw)
    except ValueError as exc:
        raise EvidenceError("malformed evidence JSON") from exc
    _reject_sensitive(data)
    chain_id = _require_testnet(rpc_url)
    schema = data.get("schema") if isinstance(data, dict) else None
    if schema == SCHEMA_MANIFEST:
        _check_record_hash(data, "manifest")
        if data.get("chain_id") not in (None, "testnet"):
            raise EvidenceError(f"chain mismatch in manifest: {data.get('chain_id')!r}")
        # Historic block + code identity.
        block_hash = data.get("block_hash")
        block_height = data.get("block_height")
        if isinstance(block_hash, str) and isinstance(block_height, int):
            block_result = _rpc(rpc_url, "block", {"block_id": block_hash})
            header = block_result.get("header") if isinstance(block_result.get("header"), dict) else {}
            if header.get("height") != block_height or header.get("hash") != block_hash:
                raise EvidenceError("block height/hash mismatch in manifest versus RPC")
        # Code identity at the recorded height if available, else final.
        account_id = data.get("account_id")
        if not isinstance(account_id, str):
            raise EvidenceError("manifest account_id missing")
        params: dict = {"request_type": "view_account", "account_id": account_id, "finality": "final"}
        if isinstance(block_height, int):
            # Prefer historic query so a later redeploy cannot hide tampering.
            try:
                historic = _rpc(rpc_url, "query", {"request_type": "view_account", "account_id": account_id, "block_id": block_height})
                if isinstance(historic.get("code_hash"), str):
                    params = {"request_type": "view_account", "account_id": account_id, "block_id": block_height}
            except EvidenceError:
                pass  # fall back to final below
        view = _rpc(rpc_url, "query", params)
        if view.get("code_hash") != data.get("code_hash"):
            raise EvidenceError("code_hash mismatch in manifest versus RPC")
        wasm_sha = data.get("wasm_sha256")
        code_hash = data.get("code_hash")
        if isinstance(code_hash, str) and code_hash != "11111111111111111111111111111111" and isinstance(wasm_sha, str):
            decoded = _base58_decode(code_hash)
            if len(decoded) != 32 or decoded.hex() != wasm_sha:
                raise EvidenceError("code_hash does not match wasm sha256 in manifest")
        if wasm is not None:
            wasm_path = Path(wasm)
            if not wasm_path.is_file():
                raise EvidenceError("wasm file missing for manifest verification")
            actual = hashlib.sha256(wasm_path.read_bytes()).hexdigest()
            if actual != wasm_sha:
                raise EvidenceError("wasm sha256 mismatch in manifest versus local file")
        return {"verified": True, "kind": "manifest", "chain_id": chain_id, "account_id": account_id, "record_hash": data.get("record_hash")}
    if schema == SCHEMA_TRANSACTION:
        _check_record_hash(data, "transaction")
        tx_hash = _verify_transaction_record(data, rpc_url)
        return {"verified": True, "kind": "transaction", "chain_id": chain_id, "transaction_hash": tx_hash, "record_hash": data.get("record_hash")}
    if schema == SCHEMA_COLLECTION and isinstance(data.get("transactions"), list):
        _check_record_hash(data, "collection")
        for entry in data["transactions"]:
            if not isinstance(entry, dict):
                raise EvidenceError("transaction entry not an object in collection")
            _reject_sensitive(entry)
            _check_record_hash(entry, "transaction")
            _verify_transaction_record(entry, rpc_url)
        # Also verify embedded manifest if present.
        manifest = data.get("manifest")
        if isinstance(manifest, dict):
            _check_record_hash(manifest, "manifest")
            if manifest.get("code_hash") is not None:
                view = _rpc(rpc_url, "query", {"request_type": "view_account", "account_id": manifest.get("account_id"), "finality": "final"})
                if view.get("code_hash") != manifest.get("code_hash"):
                    raise EvidenceError("code_hash mismatch in collection manifest versus RPC")
        return {"verified": True, "kind": "collection", "chain_id": chain_id, "count": len(data["transactions"]), "record_hash": data.get("record_hash")}
    # A bare list file of transaction records (transactions.json without envelope).
    if isinstance(data, list) and data and all(isinstance(x, dict) for x in data):
        for entry in data:
            _verify_transaction_record(entry, rpc_url)
        return {"verified": True, "kind": "transaction-list", "chain_id": chain_id, "count": len(data)}
    raise EvidenceError(f"unsupported evidence schema: {schema!r}")


def main(argv: list[str] | None = None) -> int:
    p = argparse.ArgumentParser(description="Record or verify NEAR testnet Guard evidence (read-only).")
    p.add_argument("--rpc-url", default=DEFAULT_RPC_URL, help="NEAR testnet RPC URL (HTTPS required)")
    p.add_argument("--verify", type=Path, metavar="PATH", help="verify evidence JSON against RPC")
    p.add_argument("--manifest", action="store_true", help="record a manifest (requires --account and --wasm)")
    p.add_argument("--account", help="contract account id")
    p.add_argument("--wasm", type=Path, help="locally built Guard Wasm")
    p.add_argument("--tx", dest="tx_hash", help="record a single transaction by hash (requires --signer)")
    p.add_argument("--signer", help="transaction signer account id")
    p.add_argument("--out", type=Path, help="write JSON output here (0600)")
    args = p.parse_args(argv)
    try:
        if args.verify:
            result = verify_evidence(args.verify, args.rpc_url, wasm=args.wasm)
            print(json.dumps(result, indent=2, sort_keys=True))
            return 0
        if args.manifest:
            if not args.account or not args.wasm:
                p.error("--manifest requires --account and --wasm")
            record = record_manifest(args.account, args.wasm, args.rpc_url)
        elif args.tx_hash:
            record = record_transaction(args.tx_hash, args.rpc_url, sender_id=args.signer)
        else:
            p.error("specify --verify, --manifest, or --tx")
        text = json.dumps(record, indent=2, sort_keys=True) + "\n"
        if args.out:
            args.out.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
            with open(args.out, "w", encoding="utf-8") as fh:
                fh.write(text)
            try:
                args.out.chmod(0o600)
            except OSError:
                pass
            print(f"wrote {args.out}")
        else:
            print(text, end="")
        return 0
    except EvidenceError as exc:
        print(f"EVIDENCE ERROR: {exc}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
