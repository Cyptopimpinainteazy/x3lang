#!/usr/bin/env python3
"""Read-only evidence for the X3 agent vault on NEAR testnet.

`x3-lang/near/vault-testnet.sh` signs and sends; this module never does. It
reads NEAR testnet RPC, records what a run did and judges it, failing closed:

* the deployed code hash equals the locally built WASM;
* the agent account holds exactly one key, FunctionCall-only, scoped to
  ``<vault>::request_payment``;
* the complete bounded scenario locked the vault and it holds no access keys;
* every recorded transaction ended exactly as the run said it would: paid,
  refused with a specific ``X3_VAULT_DENIED`` reason, or plain success;
* the vault's own ledger (spent, reserved, revoked, per-nonce payments)
  matches the transactions;
* the vendor received exactly the paid amounts and the stranger received
  nothing.

Two attempted attacks are represented only by local CLI refusal reports. They
have no included transaction to re-query and are never classified as independently
verified attempts or protocol refusals. The separately queried access-key list
and balances are observations at a pinned block, not an audited custody guarantee.

It also locates the legacy-keychain key file that holds a given on-chain
public key. It reads only the ``public_key`` field and never prints, copies
or returns key material.
"""
from __future__ import annotations

import argparse
import base64
import hashlib
import json
import os
import re
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
if str(HERE) not in sys.path:
    sys.path.insert(0, str(HERE))

from testnet_guard_evidence import (  # noqa: E402
    DEFAULT_RPC_URL,
    EXPECTED_TESTNET_GENESIS,
    EvidenceError,
    _ACCOUNT_RE,
    _canonical_hash,
    _check_record_hash,
    _reject_sensitive,
    _require_testnet_identity,
    _rpc,
    _utc_now,
    _validate_url,
    _verify_manifest_record,
    _verify_transaction_record,
    record_manifest,
    record_transaction,
)

SCHEMA_RUN = "x3.shield.agent_vault.testnet.run.v1"
SCHEMA_EVIDENCE = "x3.shield.agent_vault.testnet.evidence.v2"
CLASSIFICATION = "PUBLIC_NEAR_TESTNET_OBSERVED_VAULT_SCENARIO_NOT_AUDITED"
AGENT_METHOD = "request_payment"
TRUE_B64 = base64.b64encode(b"true").decode("ascii")
_HASH_RE = re.compile(r"[1-9A-HJ-NP-Za-km-z]{43,44}\Z")
_NONCE_RE = re.compile(r"[A-Za-z0-9_-]{1,64}\Z")
_PUBKEY_RE = re.compile(r"ed25519:[1-9A-HJ-NP-Za-km-z]{43,44}\Z")
_ANSI_RE = re.compile(r"\x1b\[[0-9;]*[A-Za-z]")
_TX_ID_RE = re.compile(r"Transaction ID:\s*([1-9A-HJ-NP-Za-km-z]{43,44})")
_TX_URL_RE = re.compile(r"/(?:transactions|txns)/([1-9A-HJ-NP-Za-km-z]{43,44})")
_DENIED_PREFIX = "X3_VAULT_DENIED: "
# near-cli-rs's own wording for NEAR's InvalidAccessKeyError refusals (src/common.rs,
# convert_invalid_tx_error_to_cli_result). A refusal with any other text is not accepted.
RPC_REFUSALS = {
    "requires_full_access": "Transaction requires a full permission access key.",
    "method_not_allowed": "isn't allowed by the access key.",
}
ROLES = ("owner", "vault", "agent", "vendor", "stranger")


# ------------------------------------------------------------------------------------------
# Small validators


def _account(value: object, field: str) -> str:
    text = str(value)
    if not _ACCOUNT_RE.fullmatch(text) or not text.endswith(".testnet"):
        raise EvidenceError(f"{field} must be a testnet account id, got {value!r}")
    return text


def _yocto(value: object, field: str) -> int:
    text = str(value)
    if not re.fullmatch(r"[0-9]{1,40}", text):
        raise EvidenceError(f"{field} must be a non-negative integer of yoctoNEAR, got {value!r}")
    return int(text)


def parse_expect(text: str) -> tuple[str, str | None]:
    """`success`, `returns_true`, `denied:<reason>` or `rpc_refused:<refusal>`."""
    if text in ("success", "returns_true"):
        return text, None
    if text.startswith("rpc_refused:"):
        refusal = text[len("rpc_refused:"):]
        if refusal not in RPC_REFUSALS:
            raise EvidenceError(f"unknown RPC refusal {refusal!r}; expected one of {sorted(RPC_REFUSALS)}")
        return "rpc_refused", refusal
    if text.startswith("denied:") and len(text) > len("denied:"):
        reason = text[len("denied:"):]
        if not re.fullmatch(r"[a-z][a-z ,_-]{2,80}", reason):
            raise EvidenceError(f"denial reason must be plain lowercase words, got {reason!r}")
        return "denied", reason
    raise EvidenceError(f"unknown expectation {text!r}")


def extract_tx_hash(log_text: str) -> str | None:
    """Return the one transaction id near-cli printed, or None if it printed none.

    More than one distinct id in a single command's output is ambiguous and refused.
    """
    clean = _ANSI_RE.sub("", log_text)
    found = set(_TX_ID_RE.findall(clean)) | set(_TX_URL_RE.findall(clean))
    if not found:
        return None
    if len(found) > 1:
        raise EvidenceError(f"command output names several transactions: {sorted(found)}")
    return found.pop()


# ------------------------------------------------------------------------------------------
# RPC reads (all pinned to one block when a block hash is given)


def _block_ref(block_hash: str | None) -> dict:
    return {"block_id": block_hash} if block_hash else {"finality": "final"}


def final_block(rpc_url: str) -> dict:
    block = _rpc(rpc_url, "block", {"finality": "final"})
    header = block.get("header") if isinstance(block.get("header"), dict) else {}
    height, block_hash = header.get("height"), header.get("hash")
    if type(height) is not int or not isinstance(block_hash, str):
        raise EvidenceError("final block header missing height or hash")
    return {"block_hash": block_hash, "block_height": height}


def _normalize_permission(permission: object) -> object:
    if permission == "FullAccess":
        return "FullAccess"
    if isinstance(permission, dict) and isinstance(permission.get("FunctionCall"), dict):
        fc = permission["FunctionCall"]
        methods = fc.get("method_names")
        if not isinstance(methods, list) or not all(isinstance(m, str) for m in methods):
            raise EvidenceError("FunctionCall permission has malformed method names")
        return {"FunctionCall": {"receiver_id": str(fc.get("receiver_id")), "method_names": list(methods)}}
    raise EvidenceError(f"unrecognized access-key permission: {permission!r}")


def access_keys(rpc_url: str, account: str, block_hash: str | None = None) -> dict:
    """Every access key on `account`, without allowances (they move with use)."""
    result = _rpc(rpc_url, "query", {"request_type": "view_access_key_list", "account_id": account, **_block_ref(block_hash)})
    keys = result.get("keys")
    if not isinstance(keys, list):
        raise EvidenceError(f"access-key list missing for {account}")
    out = []
    for entry in keys:
        if not isinstance(entry, dict) or not isinstance(entry.get("access_key"), dict):
            raise EvidenceError(f"malformed access-key entry for {account}")
        public_key = entry.get("public_key")
        if not isinstance(public_key, str):
            raise EvidenceError(f"access key without public key on {account}")
        out.append({"public_key": public_key, "permission": _normalize_permission(entry["access_key"].get("permission"))})
    out.sort(key=lambda k: k["public_key"])
    return {"account_id": account, "block_hash": result.get("block_hash"), "block_height": result.get("block_height"), "keys": out}


def account_balance(rpc_url: str, account: str, block_hash: str | None = None) -> dict:
    result = _rpc(rpc_url, "query", {"request_type": "view_account", "account_id": account, **_block_ref(block_hash)})
    amount = result.get("amount")
    if not isinstance(amount, str) or not amount.isdigit():
        raise EvidenceError(f"balance missing for {account}")
    return {"account_id": account, "amount": amount, "block_hash": result.get("block_hash"), "block_height": result.get("block_height")}


def view_call(rpc_url: str, contract: str, method: str, args: dict, block_hash: str | None = None, block_height: int | None = None) -> object:
    encoded = base64.b64encode(json.dumps(args, separators=(",", ":"), sort_keys=True).encode("utf-8")).decode("ascii")
    result = _rpc(rpc_url, "query", {
        "request_type": "call_function", "account_id": contract, "method_name": method,
        "args_base64": encoded, **_block_ref(block_hash),
    })
    if block_hash is not None and (result.get("block_hash") != block_hash or (block_height is not None and result.get("block_height") != block_height)):
        raise EvidenceError(f"{contract}.{method}: returned block differs from pinned view")
    raw = result.get("result")
    if not isinstance(raw, list) or not all(isinstance(b, int) and 0 <= b <= 255 for b in raw):
        raise EvidenceError(f"{contract}.{method} returned no result bytes")
    try:
        return json.loads(bytes(raw).decode("utf-8"))
    except ValueError as exc:
        raise EvidenceError(f"{contract}.{method} returned non-JSON") from exc


def onchain_key(rpc_url: str, account: str, kind: str, receiver: str | None = None, method: str | None = None) -> str:
    """The single on-chain key of the requested kind; ambiguity is refused."""
    keys = access_keys(rpc_url, account)["keys"]
    if kind == "full":
        matches = [k for k in keys if k["permission"] == "FullAccess"]
    elif kind == "function-call":
        want = {"FunctionCall": {"receiver_id": receiver, "method_names": [method]}}
        matches = [k for k in keys if k["permission"] == want]
    else:
        raise EvidenceError(f"unknown key kind {kind!r}")
    if len(matches) != 1:
        raise EvidenceError(f"{account} has {len(matches)} {kind} keys matching; expected exactly one")
    return matches[0]["public_key"]


def key_file_public_key(path: Path) -> str:
    """The `public_key` field of a near-cli key file; nothing else is read out."""
    try:
        data = json.loads(Path(path).expanduser().read_text(encoding="utf-8"))
    except (OSError, ValueError) as exc:
        raise EvidenceError(f"cannot read key file {path}") from exc
    public_key = data.get("public_key") if isinstance(data, dict) else None
    if not isinstance(public_key, str) or not _PUBKEY_RE.fullmatch(public_key):
        raise EvidenceError(f"{path} has no ed25519 public_key")
    return public_key


def find_key_file(credentials_dir: Path, network: str, account: str, public_key: str) -> Path:
    """The legacy-keychain file for `public_key`. Reads only each file's `public_key`."""
    if not _PUBKEY_RE.fullmatch(public_key):
        raise EvidenceError(f"not an ed25519 public key: {public_key!r}")
    base = Path(credentials_dir).expanduser() / network
    candidates = [base / account / f"{public_key.replace(':', '_')}.json", base / f"{account}.json"]
    account_dir = base / account
    if account_dir.is_dir():
        candidates.extend(sorted(account_dir.glob("*.json")))
    seen: set[Path] = set()
    for path in candidates:
        if path in seen or not path.is_file():
            continue
        seen.add(path)
        try:
            data = json.loads(path.read_text(encoding="utf-8"))
        except (OSError, ValueError):
            continue
        if isinstance(data, dict) and data.get("public_key") == public_key:
            return path
    raise EvidenceError(f"no key file for {account} holds {public_key} under {base}")


# ------------------------------------------------------------------------------------------
# The run file written step by step by vault-testnet.sh


def _load_json(path: Path) -> dict:
    try:
        data = json.loads(Path(path).read_text(encoding="utf-8"))
    except FileNotFoundError as exc:
        raise EvidenceError(f"missing file: {path}") from exc
    except ValueError as exc:
        raise EvidenceError(f"malformed JSON: {path}") from exc
    if not isinstance(data, dict):
        raise EvidenceError(f"not a JSON object: {path}")
    _reject_sensitive(data)
    return data


def _write_json(path: Path, data: dict) -> None:
    _reject_sensitive(data)
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_suffix(path.suffix + ".tmp")
    fd = os.open(tmp, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(fd, "w", encoding="utf-8") as fh:
        fh.write(json.dumps(data, indent=2, sort_keys=True) + "\n")
    os.replace(tmp, path)


def init_run(path: Path, *, roles: dict, cap: str, budget: str, fund: str, deadline_ms: int, locked: bool) -> dict:
    if Path(path).exists():
        raise EvidenceError(f"run file already exists: {path}")
    accounts = {role: _account(roles[role], role) for role in ROLES}
    if len(set(accounts.values())) != len(ROLES):
        raise EvidenceError("owner, vault, agent, vendor and stranger must be five distinct accounts")
    cap_y, budget_y, fund_y = _yocto(cap, "cap"), _yocto(budget, "budget"), _yocto(fund, "fund")
    if not 0 < cap_y <= budget_y <= fund_y:
        raise EvidenceError("need 0 < cap <= budget <= fund")
    run = {
        "schema": SCHEMA_RUN, "network": "testnet", "accounts": accounts,
        "terms": {"cap": str(cap_y), "budget": str(budget_y), "fund": str(fund_y), "deadline_ms": int(deadline_ms)},
        "locked": bool(locked), "steps": [], "snapshots": {}, "created_at_utc": _utc_now(),
    }
    _write_json(path, run)
    return run


def add_step(path: Path, *, label: str, expect: str, signer: str, tx_hash: str | None,
             nonce: str | None = None, recipient: str | None = None, amount: str | None = None,
             cli_exit: int | None = None, log_text: str | None = None) -> dict:
    run = _load_json(path)
    if run.get("schema") != SCHEMA_RUN:
        raise EvidenceError("not a vault run file")
    if not re.fullmatch(r"[a-z][a-z0-9_]{1,40}", label):
        raise EvidenceError(f"bad step label {label!r}")
    if any(s["label"] == label for s in run["steps"]):
        raise EvidenceError(f"step {label} is already recorded; start a new run for a second attempt")
    kind, reason = parse_expect(expect)
    step: dict = {"label": label, "expect": kind, "signer": _account(signer, "signer")}
    if reason:
        step["reason"] = reason
    if kind == "rpc_refused":
        if tx_hash:
            raise EvidenceError(f"{label}: an rpc_refused step must not have a transaction hash; one was printed, so it reached the chain")
        if cli_exit is None or cli_exit == 0:
            raise EvidenceError(f"{label}: an rpc_refused step needs the CLI's non-zero exit code")
        clean = _ANSI_RE.sub("", log_text or "")
        if extract_tx_hash(clean) is not None:
            raise EvidenceError(f"{label}: the CLI output names a transaction, so it may have reached the chain")
        wanted = RPC_REFUSALS[reason]
        lines = [line.strip() for line in clean.splitlines() if wanted in line]
        if not lines:
            raise EvidenceError(f"{label}: the CLI output does not contain NEAR's refusal '{wanted}'")
        step["cli_exit"] = int(cli_exit)
        step["cli_message"] = lines[0][:300]
    else:
        if not tx_hash or not _HASH_RE.fullmatch(tx_hash):
            raise EvidenceError(f"{label}: a transaction hash is required")
        step["tx_hash"] = tx_hash
    if nonce is not None:
        if not _NONCE_RE.fullmatch(nonce):
            raise EvidenceError(f"bad nonce {nonce!r}")
        step["nonce"] = nonce
        step["recipient"] = _account(recipient, "recipient")
        step["amount"] = str(_yocto(amount, "amount"))
    run["steps"].append(step)
    _write_json(path, run)
    return step


def add_snapshot(path: Path, rpc_url: str, name: str, accounts: list[str]) -> dict:
    run = _load_json(path)
    if not re.fullmatch(r"[a-z][a-z0-9_]{1,40}", name) or name in run["snapshots"]:
        raise EvidenceError(f"snapshot {name!r} is invalid or already taken")
    _require_testnet_identity(rpc_url)
    block = final_block(rpc_url)
    run["snapshots"][name] = {
        "block_hash": block["block_hash"], "block_height": block["block_height"],
        "balances": {a: account_balance(rpc_url, _account(a, "account"), block["block_hash"])["amount"] for a in accounts},
    }
    _write_json(path, run)
    return run["snapshots"][name]


# ------------------------------------------------------------------------------------------
# Judging


def _status(record: dict) -> object:
    try:
        return json.loads(record["status"]) if record.get("status") else None
    except ValueError as exc:
        raise EvidenceError(f"unparseable status for {record.get('tx_hash')}") from exc


def chain_signer(rpc_url: str, tx_hash: str, signer: str) -> str:
    """The signer the chain itself reports for a transaction (not the one we asked with)."""
    result = _rpc(rpc_url, "tx", [tx_hash, signer])
    transaction = result.get("transaction") if isinstance(result.get("transaction"), dict) else {}
    actual = transaction.get("signer_id")
    if not isinstance(actual, str):
        raise EvidenceError(f"transaction {tx_hash} has no signer_id")
    return actual


# This verifier accepts the complete bounded operator scenario, not arbitrary
# caller-selected successes. Local CLI reports remain a separate evidence level.
SCENARIO = (
    ("create_vault", "owner", "success"),
    ("create_agent", "owner", "success"),
    ("create_vendor", "owner", "success"),
    ("create_stranger", "owner", "success"),
    ("vault_deployed", "vault", "success"),
    ("vault_locked", "vault", "success"),
    ("owner_funds", "owner", "success"),
    ("agent_key_scoped", "agent", "success"),
    ("agent_full_key_deleted", "agent", "success"),
    ("agent_pays_vendor", "agent", "returns_true"),
    ("agent_over_cap", "agent", "denied:per-payment cap exceeded"),
    ("agent_pays_stranger", "agent", "denied:recipient not allowlisted"),
    ("agent_replays_nonce", "agent", "denied:nonce already used"),
    ("agent_pays_vendor_again", "agent", "returns_true"),
    ("agent_over_budget", "agent", "denied:cumulative budget exceeded"),
    ("agent_raw_transfer", "agent", "rpc_refused:requires_full_access"),
    ("agent_calls_revoke", "agent", "rpc_refused:method_not_allowed"),
    ("owner_recovers_while_active", "owner", "denied:revoke or expire mandate before recovery"),
    ("owner_revokes", "owner", "success"),
    ("agent_after_revoke", "agent", "denied:owner revoked mandate"),
    ("owner_recovers", "owner", "returns_true"),
)
PAYMENTS = {
    "agent_pays_vendor": ("pay-1", "vendor", 10**23),
    "agent_over_cap": ("over-cap", "vendor", 3 * 10**23),
    "agent_pays_stranger": ("to-stranger", "stranger", 10**23),
    "agent_replays_nonce": ("pay-1", "vendor", 10**23),
    "agent_pays_vendor_again": ("pay-2", "vendor", 2 * 10**23),
    "agent_over_budget": ("pay-3", "vendor", 2 * 10**23),
    "agent_after_revoke": ("after-revoke", "vendor", 5 * 10**22),
}


def _validate_scenario(run: dict) -> dict[str, dict]:
    if run.get("schema") != SCHEMA_RUN or run.get("network") != "testnet":
        raise EvidenceError("not a testnet vault run")
    if run.get("locked") is not True:
        raise EvidenceError("observed vault scenario requires locked=true")
    roles = run.get("accounts", {})
    if set(roles) != set(ROLES) or len(set(roles.values())) != len(ROLES):
        raise EvidenceError("scenario requires five distinct account roles")
    for role in ROLES:
        _account(roles[role], role)
    terms = run.get("terms", {})
    if any(terms.get(k) != str(v) for k, v in (("cap", 2 * 10**23), ("budget", 4 * 10**23), ("fund", 10**24))):
        raise EvidenceError("scenario terms differ from bounded operator profile")
    if type(terms.get("deadline_ms")) is not int or terms["deadline_ms"] <= 0:
        raise EvidenceError("scenario deadline must be a positive integer")
    steps = run.get("steps")
    labels = [s.get("label") for s in steps if isinstance(s, dict)] if isinstance(steps, list) else []
    expected = [row[0] for row in SCENARIO]
    if labels != expected:
        raise EvidenceError("scenario steps must be complete, unique and in operator order")
    by_label = dict(zip(labels, steps))
    hashes = set()
    for label, role, expect in SCENARIO:
        step = by_label[label]
        kind, reason = parse_expect(expect)
        if step.get("signer") != roles[role] or step.get("expect") != kind or step.get("reason") != reason:
            raise EvidenceError(f"{label}: unexpected signer or expected outcome")
        if label in PAYMENTS:
            nonce, recipient, amount = PAYMENTS[label]
            if (step.get("nonce"), step.get("recipient"), step.get("amount")) != (nonce, roles[recipient], str(amount)):
                raise EvidenceError(f"{label}: payment terms differ from operator scenario")
        elif any(k in step for k in ("nonce", "recipient", "amount")):
            raise EvidenceError(f"{label}: unexpected payment fields")
        if kind == "rpc_refused":
            _local_refusal_verdict(step)
        else:
            tx_hash = step.get("tx_hash")
            if not isinstance(tx_hash, str) or not _HASH_RE.fullmatch(tx_hash) or tx_hash in hashes:
                raise EvidenceError(f"{label}: missing or duplicate transaction hash")
            hashes.add(tx_hash)
    return by_label


def _local_refusal_verdict(step: dict) -> dict:
    message = step.get("cli_message")
    if (step.get("tx_hash") is not None or type(step.get("cli_exit")) is not int or step["cli_exit"] == 0
            or not isinstance(message, str) or len(message) > 300
            or RPC_REFUSALS.get(step.get("reason"), "__invalid__") not in message
            or extract_tx_hash(message) is not None):
        raise EvidenceError(f"{step.get('label')}: invalid local CLI refusal report")
    return {"label": step["label"], "verdict": f"local CLI reported: {message} (exit {step['cli_exit']})",
            "tx_hash": None, "evidence_level": "local_cli_report_not_independently_verified"}


def _check_manifest_binding(manifest: dict, vault: str) -> None:
    if (manifest.get("account_id") != vault or manifest.get("code_hash") in (None, "", "1" * 32)
            or type(manifest.get("wasm_bytes")) is not int or manifest["wasm_bytes"] < 8):
        raise EvidenceError("manifest must identify the run vault and nonempty deployed WASM")


def _raw_transaction(step: dict, rpc_url: str) -> dict:
    raw = _rpc(rpc_url, "tx", [step["tx_hash"], step["signer"]])
    tx = raw.get("transaction")
    if not isinstance(tx, dict) or tx.get("hash") != step["tx_hash"] or tx.get("signer_id") != step["signer"]:
        raise EvidenceError(f"{step['label']}: raw transaction hash/signer mismatch")
    return tx


def _check_transaction_actions(step: dict, tx: dict, run: dict, manifest: dict,
                               raw_by_label: dict, scoped_key: str) -> None:
    label = step["label"]
    roles, terms = run["accounts"], run["terms"]
    actions = tx.get("actions")
    if not isinstance(actions, list):
        raise EvidenceError(f"{label}: missing transaction actions")
    receiver = roles[label.removeprefix("create_")] if label.startswith("create_") else (
        roles["agent"] if label in ("agent_key_scoped", "agent_full_key_deleted") else roles["vault"])
    if tx.get("receiver_id") != receiver:
        raise EvidenceError(f"{label}: wrong receiver")

    def key_action(action, permission):
        if not isinstance(action, dict) or set(action) != {"AddKey"}:
            raise EvidenceError(f"{label}: expected AddKey action")
        key = action["AddKey"]
        if (not isinstance(key, dict) or not _PUBKEY_RE.fullmatch(str(key.get("public_key", "")))
                or key.get("access_key", {}).get("permission") != permission):
            raise EvidenceError(f"{label}: wrong added-key permission")
        return key["public_key"]

    if label.startswith("create_"):
        role = label.removeprefix("create_")
        deposits = {"agent": 5 * 10**23, "vendor": 10**23, "stranger": 10**23,
                    "vault": ((2 * manifest["wasm_bytes"] + 499999) // 200000) * 10**24}
        if (len(actions) != 3 or actions[0] != "CreateAccount"
                or actions[1] != {"Transfer": {"deposit": str(deposits[role])}}):
            raise EvidenceError(f"{label}: wrong create-account actions or deposit")
        key_action(actions[2], "FullAccess")
        return
    if label == "agent_key_scoped":
        permission = {"FunctionCall": {"allowance": str(25 * 10**22), "receiver_id": roles["vault"],
                                      "method_names": [AGENT_METHOD]}}
        if len(actions) != 1 or key_action(actions[0], permission) != scoped_key:
            raise EvidenceError(f"{label}: wrong scoped public key")
        return
    if label in ("vault_locked", "agent_full_key_deleted"):
        role = "vault" if label == "vault_locked" else "agent"
        created = raw_by_label["create_" + role]["actions"][2]["AddKey"]["public_key"]
        if actions != [{"DeleteKey": {"public_key": created}}]:
            raise EvidenceError(f"{label}: wrong deleted key action")
        return
    if label == "vault_deployed":
        if len(actions) != 2 or not isinstance(actions[0], dict) or set(actions[0]) != {"DeployContract"}:
            raise EvidenceError("vault_deployed: expected atomic deploy and initialization")
        try:
            code = base64.b64decode(actions[0]["DeployContract"]["code"], validate=True)
        except (KeyError, TypeError, ValueError) as exc:
            raise EvidenceError("vault_deployed: invalid deployed code") from exc
        if len(code) != manifest["wasm_bytes"] or hashlib.sha256(code).hexdigest() != manifest["wasm_sha256"]:
            raise EvidenceError("vault_deployed: code differs from manifest")
        method = "new"
        args = {"owner": roles["owner"], "agent": roles["agent"], "recipient": roles["vendor"], **terms}
        args.pop("fund")
        call = actions[1]
    else:
        if len(actions) != 1:
            raise EvidenceError(f"{label}: expected exactly one function call")
        call = actions[0]
        if label in PAYMENTS:
            method, args = AGENT_METHOD, {k: step[k] for k in ("nonce", "recipient", "amount")}
            if tx.get("public_key") != scoped_key:
                raise EvidenceError(f"{label}: payment was not signed by the scoped key")
        elif label == "owner_funds":
            method, args = "fund", {}
        elif label == "owner_revokes":
            method, args = "revoke", {}
        else:
            method = "recover_available"
            args = {"amount": str(10**23 if label == "owner_recovers_while_active" else 7 * 10**23)}
    if not isinstance(call, dict) or set(call) != {"FunctionCall"} or not isinstance(call["FunctionCall"], dict):
        raise EvidenceError(f"{label}: expected function call")
    fc = call["FunctionCall"]
    try:
        actual_args = json.loads(base64.b64decode(fc["args"], validate=True))
    except (KeyError, ValueError, TypeError) as exc:
        raise EvidenceError(f"{label}: invalid function arguments") from exc
    deposit = terms["fund"] if label == "owner_funds" else "0"
    if fc.get("method_name") != method or actual_args != args or fc.get("deposit") != deposit:
        raise EvidenceError(f"{label}: method, arguments or attached deposit mismatch")


def judge_step(step: dict, record: dict, vault: str, signed_by: str) -> dict:
    """Does the recorded on-chain outcome match what the run said would happen?"""
    label = step["label"]
    if signed_by != step["signer"] or record.get("signer_id") != step["signer"]:
        raise EvidenceError(f"{label}: signed by {signed_by}, expected {step['signer']}")
    status = _status(record)
    text = json.dumps(status, sort_keys=True)
    if step.get("nonce") is not None:
        if record.get("receiver_id") != vault or record.get("method_names") != [AGENT_METHOD]:
            raise EvidenceError(f"{label}: not a single {vault}.{AGENT_METHOD} call")
        args = record["sanitized_args"][0].get("sanitized_args", {})
        if args != {"nonce": step["nonce"], "recipient": step["recipient"], "amount": step["amount"]}:
            raise EvidenceError(f"{label}: on-chain arguments differ from the run's")
    kind = step["expect"]
    if kind == "success":
        if not (isinstance(status, dict) and "SuccessValue" in status) or record.get("receipt_errors"):
            raise EvidenceError(f"{label}: expected success, got {text[:300]}")
        verdict = "succeeded"
    elif kind == "returns_true":
        if status != {"SuccessValue": TRUE_B64} or record.get("receipt_errors"):
            raise EvidenceError(f"{label}: expected the vault to return true, got {text[:300]}")
        verdict = "succeeded_and_vault_returned_true"
    elif kind == "denied":
        wanted = _DENIED_PREFIX + step["reason"]
        if not (isinstance(status, dict) and "Failure" in status) or wanted not in text:
            raise EvidenceError(f"{label}: expected refusal '{wanted}', got {text[:300]}")
        verdict = f"refused: {wanted}"
    else:
        raise EvidenceError(f"{label}: {kind} steps carry no transaction")
    return {"label": label, "verdict": verdict, "tx_hash": record["tx_hash"]}


def _check_ledger(run: dict, rpc_url: str, block_hash: str, block_height: int) -> dict:
    vault = run["accounts"]["vault"]
    steps = run["steps"]
    paid = [s for s in steps if s.get("nonce") and s["expect"] == "returns_true"]
    paid_nonces = {s["nonce"] for s in paid}
    paid_total = sum(int(s["amount"]) for s in paid)
    spent = int(view_call(rpc_url, vault, "spent", {}, block_hash, block_height))
    reserved = int(view_call(rpc_url, vault, "reserved", {}, block_hash, block_height))
    revoked = view_call(rpc_url, vault, "is_revoked", {}, block_hash, block_height)
    available = int(view_call(rpc_url, vault, "available", {}, block_hash, block_height))
    if spent != paid_total:
        raise EvidenceError(f"vault spent {spent} but the run paid {paid_total}")
    if available != 0:
        raise EvidenceError("vault retains available escrow after required recovery")
    if reserved != 0:
        raise EvidenceError(f"vault still reserves {reserved}: a payout has not settled")
    expect_revoked = any(s["label"] == "owner_revokes" for s in steps)
    if revoked is not expect_revoked:
        raise EvidenceError(f"vault is_revoked={revoked}, run expects {expect_revoked}")
    payments = {}
    for nonce in sorted({s["nonce"] for s in steps if s.get("nonce")}):
        record = view_call(rpc_url, vault, "payment", {"nonce": nonce}, block_hash, block_height)
        if nonce in paid_nonces:
            match = next(s for s in paid if s["nonce"] == nonce)
            want = {"amount": match["amount"], "recipient": match["recipient"], "state": "Paid"}
            if record != want:
                raise EvidenceError(f"payment {nonce}: vault records {record!r}, expected {want!r}")
        elif record is not None:
            raise EvidenceError(f"refused nonce {nonce} left a payment record: {record!r}")
        payments[nonce] = record
    return {"spent": str(spent), "reserved": str(reserved), "available": str(available),
            "is_revoked": revoked, "payments": payments}


def _check_keys(run: dict, rpc_url: str, block_hash: str, block_height: int) -> dict:
    vault, agent = run["accounts"]["vault"], run["accounts"]["agent"]
    agent_keys = access_keys(rpc_url, agent, block_hash)
    want = {"FunctionCall": {"receiver_id": vault, "method_names": [AGENT_METHOD]}}
    if len(agent_keys["keys"]) != 1 or agent_keys["keys"][0]["permission"] != want:
        raise EvidenceError(f"agent {agent} keys are not exactly one {vault}::{AGENT_METHOD} FunctionCall key: {agent_keys['keys']}")
    vault_keys = access_keys(rpc_url, vault, block_hash)
    for observed in (agent_keys, vault_keys):
        if observed["block_hash"] != block_hash or observed["block_height"] != block_height:
            raise EvidenceError("access-key response differs from pinned final block")
    if vault_keys["keys"]:
        raise EvidenceError(f"run says the vault is locked but {vault} still has keys: {vault_keys['keys']}")
    return {"agent": agent_keys["keys"], "vault": vault_keys["keys"], "vault_has_no_keys": not vault_keys["keys"]}


def _check_balances(run: dict, rpc_url: str, block_hash: str, block_height: int) -> dict:
    before = run["snapshots"].get("before_attacks")
    if not before:
        raise EvidenceError("missing the before_attacks balance snapshot")
    vendor, stranger = run["accounts"]["vendor"], run["accounts"]["stranger"]
    paid_total = sum(int(s["amount"]) for s in run["steps"] if s.get("nonce") and s["expect"] == "returns_true")
    final_balances = {}
    for account in (vendor, stranger):
        observed = account_balance(rpc_url, account, block_hash)
        if observed["block_hash"] != block_hash or observed["block_height"] != block_height:
            raise EvidenceError("balance response differs from pinned final block")
        final_balances[account] = int(observed["amount"])
    vendor_after, stranger_after = final_balances[vendor], final_balances[stranger]
    before_hash, before_height = before.get("block_hash"), before.get("block_height")
    header = _rpc(rpc_url, "block", {"block_id": before_hash}).get("header", {})
    if (not isinstance(before_hash, str) or type(before_height) is not int
            or header.get("hash") != before_hash or header.get("height") != before_height):
        raise EvidenceError("before_attacks block identity mismatch")
    fresh_before = {}
    for account in (vendor, stranger):
        observed = account_balance(rpc_url, account, before_hash)
        if observed["block_hash"] != before_hash or observed["block_height"] != before_height:
            raise EvidenceError("before_attacks balance block mismatch")
        fresh_before[account] = observed["amount"]
    if fresh_before != before.get("balances"):
        raise EvidenceError("before_attacks balances differ from historical RPC")
    vendor_before = int(fresh_before[vendor])
    stranger_before = int(fresh_before[stranger])
    if vendor_after != vendor_before + paid_total:
        raise EvidenceError(f"vendor balance moved by {vendor_after - vendor_before}, the vault paid {paid_total}")
    if stranger_after != stranger_before:
        raise EvidenceError(f"stranger balance moved by {stranger_after - stranger_before}; it must not move")
    return {"before": before, "vendor_after": str(vendor_after), "stranger_after": str(stranger_after),
            "vendor_received": str(paid_total), "stranger_received": "0"}


def _check_final_block(rpc_url: str, block: dict) -> None:
    if (not isinstance(block.get("block_hash"), str) or not _HASH_RE.fullmatch(block["block_hash"])
            or type(block.get("block_height")) is not int or block["block_height"] <= 0):
        raise EvidenceError("invalid recorded final block")
    header = _rpc(rpc_url, "block", {"block_id": block["block_hash"]}).get("header", {})
    if header.get("hash") != block["block_hash"] or header.get("height") != block["block_height"]:
        raise EvidenceError("recorded final block differs from RPC header")


def _check_transaction_blocks(run: dict, records: list[dict], final: dict) -> None:
    heights = [r.get("block_height") for r in records]
    before = run.get("snapshots", {}).get("before_attacks", {}).get("block_height")
    if (not heights or any(type(h) is not int or h <= 0 for h in heights)
            or type(final.get("block_height")) is not int or max(heights) > final["block_height"]
            or heights != sorted(heights) or type(before) is not int):
        raise EvidenceError("transaction/final block chronology is invalid")
    first_payment = next(r["block_height"] for r in records if r["label"] == "agent_pays_vendor")
    if before >= first_payment:
        raise EvidenceError("before_attacks must precede the first payment block")


def collect(run_path: Path, wasm: Path, rpc_url: str) -> dict:
    run = _load_json(run_path)
    by_label = _validate_scenario(run)
    chain_id, genesis, _ = _require_testnet_identity(rpc_url)
    vault = run["accounts"]["vault"]
    manifest = record_manifest(vault, wasm, rpc_url)
    _check_manifest_binding(manifest, vault)
    block = final_block(rpc_url)
    _check_final_block(rpc_url, block)
    keys = _check_keys(run, rpc_url, block["block_hash"], block["block_height"])
    raw = {label: _raw_transaction(step, rpc_url) for label, step in by_label.items() if step["expect"] != "rpc_refused"}
    transactions, verdicts = [], []
    for step in run["steps"]:
        if step["expect"] == "rpc_refused":
            verdicts.append(_local_refusal_verdict(step))
            continue
        record = record_transaction(step["tx_hash"], rpc_url, sender_id=step["signer"])
        _check_transaction_actions(step, raw[step["label"]], run, manifest, raw, keys["agent"][0]["public_key"])
        verdicts.append(judge_step(step, record, vault, raw[step["label"]]["signer_id"]))
        record["label"] = step["label"]
        record["record_hash"] = _canonical_hash({k: v for k, v in record.items() if k != "record_hash"})
        transactions.append(record)
    _check_transaction_blocks(run, transactions, block)
    evidence = {
        "schema": SCHEMA_EVIDENCE, "chain_id": chain_id, "genesis_hash": genesis,
        "classification": CLASSIFICATION,
        "run": run, "manifest": manifest, "transactions": transactions, "verdicts": verdicts,
        "final_block": block,
        "keys": keys,
        "ledger": _check_ledger(run, rpc_url, block["block_hash"], block["block_height"]),
        "balances": _check_balances(run, rpc_url, block["block_hash"], block["block_height"]),
        "limits": [
            "testnet only; the contract refuses to initialize on any other network",
            "native NEAR to one allowlisted recipient; it does not pay NEAR Intents deposit addresses",
            "the owner account's own full-access key is trusted and outside this vault",
            "CLI refusal reports are local evidence, not independently verified attempts or protocol refusals",
            "RPC observations do not authenticate finality or establish an audited custody/security guarantee",
            "not independently audited",
        ],
        "collected_at_utc": _utc_now(), "rpc_url": _validate_url(rpc_url),
    }
    _reject_sensitive(evidence)
    evidence["record_hash"] = _canonical_hash({k: v for k, v in evidence.items() if k != "record_hash"})
    return evidence


def verify(evidence_path: Path, rpc_url: str, wasm: Path | None = None) -> dict:
    """Re-read everything at the recorded blocks; any difference fails."""
    data = _load_json(evidence_path)
    if data.get("schema") != SCHEMA_EVIDENCE:
        raise EvidenceError("not vault testnet evidence")
    _check_record_hash(data, "vault evidence")
    if data.get("chain_id") != "testnet" or data.get("genesis_hash") != EXPECTED_TESTNET_GENESIS:
        raise EvidenceError("evidence is not pinned to NEAR testnet")
    _require_testnet_identity(rpc_url)
    run, vault = data["run"], data["run"]["accounts"]["vault"]
    by_label = _validate_scenario(run)
    if data.get("classification") != CLASSIFICATION:
        raise EvidenceError("unexpected evidence classification")
    _check_manifest_binding(data["manifest"], vault)
    _verify_manifest_record(data["manifest"], rpc_url, wasm)
    records = data.get("transactions")
    expected_labels = [s["label"] for s in run["steps"] if s["expect"] != "rpc_refused"]
    labels = [r.get("label") for r in records if isinstance(r, dict)] if isinstance(records, list) else []
    if labels != expected_labels:
        raise EvidenceError("transaction records must match every non-CLI step exactly once and in order")
    raw = {label: _raw_transaction(by_label[label], rpc_url) for label in expected_labels}
    by_record = dict(zip(labels, records))
    verdicts = []
    for step in run["steps"]:
        if step["expect"] == "rpc_refused":
            verdicts.append(_local_refusal_verdict(step))
            continue
        record = by_record[step["label"]]
        if record.get("tx_hash") != step["tx_hash"] or record.get("transaction_hash") != step["tx_hash"]:
            raise EvidenceError(f"{step['label']}: record transaction hash differs from run step")
        _check_record_hash(record, f"transaction {record.get('label')}")
        _verify_transaction_record(record, rpc_url)
        _check_transaction_actions(step, raw[step["label"]], run, data["manifest"], raw, data["keys"]["agent"][0]["public_key"])
        verdicts.append(judge_step(step, record, vault, raw[step["label"]]["signer_id"]))
    if verdicts != data.get("verdicts"):
        raise EvidenceError("verdicts differ from recomputed transaction/local-report outcomes")
    _check_final_block(rpc_url, data["final_block"])
    _check_transaction_blocks(run, records, data["final_block"])
    block_hash = data["final_block"]["block_hash"]
    try:
        keys = _check_keys(run, rpc_url, block_hash, data["final_block"]["block_height"])
        ledger = _check_ledger(run, rpc_url, block_hash, data["final_block"]["block_height"])
        balances = _check_balances(run, rpc_url, block_hash, data["final_block"]["block_height"])
    except EvidenceError as exc:
        raise EvidenceError(f"{exc} (re-reading block {block_hash}; older blocks need an archival testnet RPC)") from exc
    for name, fresh in (("keys", keys), ("ledger", ledger), ("balances", balances)):
        if fresh != data[name]:
            raise EvidenceError(f"{name} differ from the recorded evidence")
    return {"verified": True, "kind": "observed_agent_vault_testnet_scenario", "vault": vault,
            "classification": CLASSIFICATION, "local_cli_reports_independently_verified": False,
            "transactions": len(data["transactions"]), "record_hash": data["record_hash"]}


# ------------------------------------------------------------------------------------------
# CLI


def main(argv: list[str] | None = None) -> int:
    p = argparse.ArgumentParser(description="Read-only NEAR testnet evidence for the X3 agent vault.")
    p.add_argument("--rpc-url", default=os.environ.get("NEAR_TESTNET_RPC_URL", DEFAULT_RPC_URL))
    sub = p.add_subparsers(dest="cmd", required=True)

    s = sub.add_parser("key-file", help="path of the legacy-keychain file holding the account's on-chain key")
    s.add_argument("--account", required=True)
    s.add_argument("--kind", choices=["full", "function-call"], required=True)
    s.add_argument("--receiver")
    s.add_argument("--method")
    s.add_argument("--credentials-dir", type=Path, default=Path("~/.near-credentials"))

    s = sub.add_parser("public-key", help="the account's single on-chain key of a kind")
    s.add_argument("--account", required=True)
    s.add_argument("--kind", choices=["full", "function-call"], required=True)
    s.add_argument("--receiver")
    s.add_argument("--method")

    s = sub.add_parser("key-check", help="fail unless the key file's public key is a full-access key on the account")
    s.add_argument("--account", required=True)
    s.add_argument("--key-file", type=Path, required=True)

    s = sub.add_parser("manifest-check", help="fail unless the account's deployed code is exactly this WASM")
    s.add_argument("--account", required=True)
    s.add_argument("--wasm", type=Path, required=True)

    s = sub.add_parser("tx-hash", help="the transaction id in a near-cli log, or nothing")
    s.add_argument("log", type=Path)

    s = sub.add_parser("init")
    s.add_argument("--run", type=Path, required=True)
    for role in ROLES:
        s.add_argument(f"--{role}", required=True)
    for term in ("cap", "budget", "fund"):
        s.add_argument(f"--{term}", required=True)
    s.add_argument("--deadline-ms", type=int, required=True)
    s.add_argument("--locked", choices=["yes", "no"], required=True)

    s = sub.add_parser("step")
    s.add_argument("--run", type=Path, required=True)
    s.add_argument("--label", required=True)
    s.add_argument("--expect", required=True)
    s.add_argument("--signer", required=True)
    s.add_argument("--tx")
    s.add_argument("--nonce")
    s.add_argument("--recipient")
    s.add_argument("--amount")
    s.add_argument("--cli-exit", type=int)
    s.add_argument("--log", type=Path, help="near-cli output of an rpc_refused step")

    s = sub.add_parser("has-step", help="exit 0 if the run already recorded this step, 1 if not")
    s.add_argument("--run", type=Path, required=True)
    s.add_argument("--label", required=True)

    s = sub.add_parser("snapshot")
    s.add_argument("--run", type=Path, required=True)
    s.add_argument("--name", required=True)
    s.add_argument("--account", action="append", required=True)

    s = sub.add_parser("view")
    s.add_argument("--vault", required=True)

    s = sub.add_parser("collect")
    s.add_argument("--run", type=Path, required=True)
    s.add_argument("--wasm", type=Path, required=True)
    s.add_argument("--out", type=Path, required=True)

    s = sub.add_parser("verify")
    s.add_argument("--evidence", type=Path, required=True)
    s.add_argument("--wasm", type=Path)

    args = p.parse_args(argv)
    try:
        if args.cmd in ("key-file", "public-key"):
            account = _account(args.account, "account")
            if args.kind == "function-call" and not (args.receiver and args.method):
                p.error("--kind function-call needs --receiver and --method")
            public_key = onchain_key(args.rpc_url, account, args.kind, args.receiver, args.method)
            if args.cmd == "public-key":
                print(public_key)
            else:
                print(find_key_file(args.credentials_dir, "testnet", account, public_key))
        elif args.cmd == "key-check":
            account = _account(args.account, "account")
            public_key = key_file_public_key(args.key_file)
            full = [k["public_key"] for k in access_keys(args.rpc_url, account)["keys"] if k["permission"] == "FullAccess"]
            if public_key not in full:
                raise EvidenceError(f"{args.key_file} holds {public_key}, which is not a full-access key on {account}")
            print(public_key)
        elif args.cmd == "manifest-check":
            record = record_manifest(_account(args.account, "account"), args.wasm, args.rpc_url)
            print(f"{record['account_id']} runs code {record['code_hash']} = sha256 {record['wasm_sha256']} at block {record['block_height']}")
        elif args.cmd == "tx-hash":
            found = extract_tx_hash(args.log.read_text(encoding="utf-8", errors="replace"))
            if found:
                print(found)
        elif args.cmd == "init":
            init_run(args.run, roles={r: getattr(args, r) for r in ROLES}, cap=args.cap, budget=args.budget,
                     fund=args.fund, deadline_ms=args.deadline_ms, locked=args.locked == "yes")
        elif args.cmd == "step":
            print(json.dumps(add_step(args.run, label=args.label, expect=args.expect, signer=args.signer,
                                      tx_hash=args.tx, nonce=args.nonce, recipient=args.recipient,
                                      amount=args.amount, cli_exit=args.cli_exit,
                                      log_text=args.log.read_text(encoding="utf-8", errors="replace") if args.log else None),
                             sort_keys=True))
        elif args.cmd == "has-step":
            run = _load_json(args.run)
            return 0 if any(s.get("label") == args.label for s in run.get("steps", [])) else 1
        elif args.cmd == "snapshot":
            print(json.dumps(add_snapshot(args.run, args.rpc_url, args.name, args.account), indent=2, sort_keys=True))
        elif args.cmd == "view":
            vault = _account(args.vault, "vault")
            print(json.dumps({m: view_call(args.rpc_url, vault, m, {}) for m in ("available", "spent", "reserved", "is_revoked")},
                             indent=2, sort_keys=True))
        elif args.cmd == "collect":
            evidence = collect(args.run, args.wasm, args.rpc_url)
            _write_json(args.out, evidence)
            for v in evidence["verdicts"]:
                print(f"  {v['label']:<28} {v['verdict']}")
            print(f"wrote {args.out}")
        elif args.cmd == "verify":
            print(json.dumps(verify(args.evidence, args.rpc_url, args.wasm), indent=2, sort_keys=True))
        return 0
    except EvidenceError as exc:
        print(f"EVIDENCE ERROR: {exc}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
