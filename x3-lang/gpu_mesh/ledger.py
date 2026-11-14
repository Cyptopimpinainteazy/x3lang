"""The contribution ledger: what each contributor did in a period, signed by the coordinator.

This is the input to payouts, not a payout. Nothing here moves funds: the owner reviews the
ledger and pays from their own wallet, the same rule as every other value transfer in X3.
Canary jobs are counted for reputation but earn nothing, and only receipts whose chain and
signatures verify are counted.
"""

from __future__ import annotations

from typing import Any, Dict, Optional

from .protocol import PROTOCOL, Identity, canonical, now_ms, sha256_hex, verify_signature
from .store import Store


def build(store: Store, identity: Identity, *, since_ms: int, until_ms: Optional[int] = None) -> Dict[str, Any]:
    check = store.verify_receipts(identity.public_hex)
    if not check["ok"]:
        raise RuntimeError(f"receipt chain does not verify at seq {check['seq']}: {check['error']}")
    until = until_ms if until_ms is not None else now_ms()
    contributors: Dict[str, Dict[str, Any]] = {}
    for receipt in store.receipts(since_ms=since_ms, until_ms=until):
        entry = contributors.get(receipt["worker"])
        if entry is None:
            info = store.worker(receipt["worker"]) or {}
            entry = contributors[receipt["worker"]] = {
                "worker": receipt["worker"], "label": info.get("label", ""), "tier": info.get("tier", ""),
                "status": info.get("status", ""), "payout_account": info.get("payout_account"),
                "jobs": 0, "prompt_tokens": 0, "completion_tokens": 0, "busy_ms": 0,
                "canary_pass": 0, "canary_fail": 0, "by_app": {}, "first_receipt": receipt["seq"],
                "last_receipt": receipt["seq"],
            }
        entry["last_receipt"] = receipt["seq"]
        if receipt["kind"] == "canary":
            entry["canary_pass" if receipt["outcome"] == "canary_pass" else "canary_fail"] += 1
            continue
        entry["jobs"] += 1
        entry["prompt_tokens"] += receipt["prompt_tokens"]
        entry["completion_tokens"] += receipt["completion_tokens"]
        entry["busy_ms"] += max(0, receipt["finished_ms"] - receipt["started_ms"])
        entry["by_app"][receipt["app"]] = entry["by_app"].get(receipt["app"], 0) + 1
    body = {
        "format": f"{PROTOCOL}.ledger",
        "coordinator": identity.public_hex,
        "since_ms": since_ms,
        "until_ms": until,
        "receipt_chain_head": check["head"],
        "receipts_verified": check["receipts"],
        "contributors": sorted(contributors.values(), key=lambda e: -e["completion_tokens"]),
        "note": "Accounting only. Payouts are made by the owner from their own wallet after review.",
    }
    digest = sha256_hex(canonical(body))
    return {"ledger": body, "ledger_sha256": digest, "coordinator_sig": identity.sign(digest.encode())}


def verify(document: Dict[str, Any]) -> bool:
    body = document.get("ledger")
    if not isinstance(body, dict):
        return False
    digest = sha256_hex(canonical(body))
    return digest == document.get("ledger_sha256") and verify_signature(body.get("coordinator", ""),
                                                                      digest.encode(),
                                                                      document.get("coordinator_sig", ""))
