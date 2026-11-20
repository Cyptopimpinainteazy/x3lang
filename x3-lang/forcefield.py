"""Local forcefield checks beside the deployed x3-guard contract.

These rules run in this repository. They do not change MandateTerms, the
contract struct, or the live agent keys, and they do not sign or deploy.
Clauses this module understands (`deny recipients`, `allow programs`,
`agents`) are removed before the official mandate compiler runs, so the
terms object sent to the deployed Guard stays the terms object it already
stores.

A check marked deployed reads the repo source or the archived invoice_06
evidence. A check marked local is enforced here and is absent from the
deployed state.
"""

from __future__ import annotations

import base64
import hashlib
import json
import tempfile
from datetime import datetime, timedelta, timezone
from pathlib import Path
from typing import Any, Dict, List, Mapping, Optional, Sequence, Tuple

ROOT = Path(__file__).resolve().parents[1]
X3 = ROOT / "x3-lang"
GUARD_LIB = X3 / "near" / "x3-guard" / "src" / "lib.rs"
GUARD_MANDATE = X3 / "near" / "x3-guard" / "src" / "mandate.rs"
EVIDENCE = ROOT / "evidence" / "near-guard-mainnet-invoice06-2026-11-12"
PAYMENT = EVIDENCE / "source-recovery" / "payment-proposed.x3"
MANDATE = EVIDENCE / "source-recovery" / "mandate-proposed.x3"
OBSERVATION = EVIDENCE / "public-rpc-observation.json"
OUTCOME = EVIDENCE / "outcome-recording" / "finalized-outcome-verification.json"
TOKENS = X3 / "tests" / "fixtures" / "near_intents_tokens.json"
PROOF_PAGE = ROOT / "docs" / "hackathon" / "forcefield-proof.html"
SHA256_JS = ROOT / "docs" / "hackathon" / "forcefield-sha256.js"

WINDOW_MS = 86_400_000
ATTEMPT_CAP = 8
GENESIS_HASH = "0" * 64
HEX = set("0123456789abcdef")
AGENT_METHODS = (
    "agent_register_intent",
    "agent_accept_quote",
    "agent_record_outcome",
)
PURPOSE_FIELDS = ("reason", "purpose", "decision_hash")
BYPASS_METHODS = (
    "fn owner_execute",
    "fn approve_pending",
    "fn set_manager",
    "fn set_policy",
    "fn withdraw(",
)
OWNER_METHODS = (
    "pub fn revoke_mandate",
    "pub fn approve_intent",
    "pub fn withdraw_agent_storage",
)
MANAGER_KEY = "ed25519:reYaWhvwu8Jzo3WUM3zhn6VrhuMEF4eADL17qtRVifc"
NO_CONSUME_COMMENT = "No consumption occurs on a failed signature or rejected policy."
SETTER_COMMENT = "has no setter"
VAULT_SPEND = "vault_spend"
GUARD_ACCOUNT = "x3lang.near"
RECIPIENT = "x3lang.near"
REFUND_WALLET = "58tbMfadmi8HWzUndLqkX3prBbkiZMEaYMTDYSzhB3cQ"

CHECKS = (
    "refusal_certificate",
    "no_bypass_method",
    "outcome_authentication",
    "one_key",
    "timeout_releases_budget",
    "verifier_key_has_no_setter",
    "unenforceable_does_not_compile",
    "mandate_diff",
    "signature_is_not_permission",
    "approval_cannot_widen",
    "burned_quote_stays_burned",
    "no_purpose_string",
    "denylist_outranks_allowlist",
    "program_hash_cap",
    "mandate_hash_chain",
    "proof_recompute",
    "shared_daily_ceiling",
    "refund_wallet_pinned",
    "attempt_budget",
    "vault_gate",
)

TITLES = {
    "refusal_certificate": "Refusal certificate",
    "no_bypass_method": "No spend around the mandate",
    "outcome_authentication": "Outcome matches the transfers",
    "one_key": "One agent key",
    "timeout_releases_budget": "Unfunded accept releases the budget",
    "verifier_key_has_no_setter": "Verifier key has no setter",
    "unenforceable_does_not_compile": "Unenforceable rules do not compile",
    "mandate_diff": "Mandate diff",
    "signature_is_not_permission": "A signature is not permission",
    "approval_cannot_widen": "Approval cannot widen the mandate",
    "burned_quote_stays_burned": "A burned quote stays burned",
    "no_purpose_string": "Agent calls carry no purpose string",
    "denylist_outranks_allowlist": "Denylist outranks the allowlist",
    "program_hash_cap": "The cap is the program hash",
    "mandate_hash_chain": "Mandate versions form a hash chain",
    "proof_recompute": "The proof page recomputes the files",
    "shared_daily_ceiling": "One ceiling across agents",
    "refund_wallet_pinned": "The refund wallet is pinned",
    "attempt_budget": "Denials spend an attempt budget",
    "vault_gate": "The vault pays only through the mandate",
}

LIMITS = (
    "These checks run locally. The deployed Guard account and its state are unchanged.",
    "Extra mandate clauses stay off the terms object the deployed contract stores.",
    "Outcome authentication compares the archived Solana deposit and the archived NEAR payout.",
    "A Solana light-client proof is a separate system. The two transfers stay associated by the 1Click record.",
    "The 1Click ed25519 check stays on the deployed Guard. This command reads the archived receipt.",
    "The archived key view still has the FullAccess recovery key on the agent account.",
    "Solana USDC stays in the authorized deposit flow. The vault gate here is local.",
)

COMPILER_EDITS = (
    {
        "case": "no_slippage_bound",
        "old": "    require slippage <= 0.9%\n",
        "new": "",
        "code": "X3_NEAR_SLIPPAGE_REQUIRED",
    },
    {
        "case": "no_timeout",
        "old": "    timeout 74h refund Solana.USDC\n",
        "new": "",
        "code": "X3_NEAR_TIMEOUT_REQUIRED",
    },
    {
        "case": "no_nonce",
        "old": "    require nonce mainnet_demo_nov11_invoice_06\n",
        "new": "",
        "code": "X3_NEAR_NONCE",
    },
    {
        "case": "second_leg",
        "old": "        bridge NearIntents Solana.USDC -> Near.USDC min_receive 24750000;\n",
        "new": (
            "        bridge NearIntents Solana.USDC -> Near.USDC min_receive 24750000;\n"
            "        bridge NearIntents Near.USDC -> Near.wNEAR;\n"
        ),
        "code": "X3_NEAR_ROUTE_SHAPE",
    },
    {
        "case": "lookalike_recipient",
        "old": "receiver x3lang.near",
        "new": "receiver x3lang.ne\u0430r",
        "code": "X3_NEAR_TYPECHECK",
        "inner": "X3_INVALID_RECEIVER",
    },
    {
        "case": "zero_amount",
        "old": "amount 25000000",
        "new": "amount 0",
        "code": "X3_NEAR_TYPECHECK",
        "inner": "X3_INVALID_AMOUNT",
    },
    {
        "case": "finality_promise",
        "old": "    require slippage <= 0.9%\n",
        "new": "    require slippage <= 0.9%\n    require finality solana == finalized\n",
        "code": "X3_NEAR_TYPECHECK",
        "inner": "X3_INVALID_FINALITY",
    },
)


class ForcefieldError(Exception):
    def __init__(self, code: str, message: str) -> None:
        super().__init__(f"{code}: {message}")
        self.code = code
        self.message = message


def canonical_json(value: Any) -> str:
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False)


def sha256_text(text: str) -> str:
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


def sha256_file(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def empty_ledger() -> Dict[str, Dict[str, str]]:
    return {"quotes": {}, "deposits": {}}


def refusal_certificate(
    *,
    program_sha256: str,
    mandate_sha256: str,
    guard: str,
    quote_hash: str,
    detail: str = "",
) -> Dict[str, str]:
    """A denial names the program, the mandate, the guard, and the quote."""
    for field, value in (
        ("program_sha256", program_sha256),
        ("mandate_sha256", mandate_sha256),
        ("guard", guard),
        ("quote_hash", quote_hash),
    ):
        if not isinstance(value, str) or not value:
            raise ForcefieldError("X3_FORCEFIELD_CERTIFICATE", f"{field} is required")
    body = {
        "detail": detail,
        "guard": guard,
        "mandate_sha256": mandate_sha256,
        "program_sha256": program_sha256,
        "quote_hash": quote_hash,
    }
    encoded = canonical_json(body)
    return {**body, "certificate_sha256": sha256_text(encoded), "canonical": encoded}


def scan_contract_source(lib_source: str, mandate_source: str) -> Dict[str, Any]:
    """The repo source has owner recovery and has no method that spends around a mandate."""
    combined = lib_source + "\n" + mandate_source
    present = [name for name in BYPASS_METHODS if name in combined]
    if _has_token_withdraw(combined) and "fn withdraw(" not in present:
        present.append("fn withdraw")
    missing_owner = [name for name in OWNER_METHODS if name not in mandate_source]
    approve = function_signature(mandate_source, "approve_intent")
    widening = [word for word in ("recipient", "per_payment", "daily_cap", "refund_to") if word in approve]
    agent_calls = {
        name: function_signature(mandate_source, name)
        for name in ("agent_accept_quote", "agent_record_outcome")
    }
    purpose = {
        name: [word for word in PURPOSE_FIELDS if word in signature]
        for name, signature in agent_calls.items()
    }
    ok = (
        not present
        and not missing_owner
        and not widening
        and all(not hits for hits in purpose.values())
        and "pub const STATE_VERSION: u16 = 2;" in lib_source
    )
    return {
        "ok": ok,
        "bypass_present": present,
        "owner_methods_missing": missing_owner,
        "approve_widens": widening,
        "agent_purpose_fields": purpose,
        "state_version_2": "pub const STATE_VERSION: u16 = 2;" in lib_source,
        "where": "repo source",
    }


def _has_token_withdraw(source: str) -> bool:
    needle = "fn withdraw"
    start = 0
    while True:
        index = source.find(needle, start)
        if index < 0:
            return False
        after = source[index + len(needle):]
        if not after.startswith("_") and after.lstrip().startswith("("):
            return True
        start = index + len(needle)


def function_signature(source: str, name: str) -> str:
    marker = f"pub fn {name}"
    start = source.find(marker)
    if start < 0:
        raise ForcefieldError("X3_FORCEFIELD_SOURCE", f"missing {name}")
    paren = source.find("(", start)
    if paren < 0:
        raise ForcefieldError("X3_FORCEFIELD_SOURCE", f"{name} has no parameter list")
    depth = 0
    for index in range(paren, len(source)):
        character = source[index]
        if character == "(":
            depth += 1
        elif character == ")":
            depth -= 1
            if depth == 0:
                return source[paren:index + 1]
    raise ForcefieldError("X3_FORCEFIELD_SOURCE", f"{name} parameter list is unclosed")


def verifier_key_lock(lib_source: str) -> Dict[str, Any]:
    constant = f'pub const ONE_CLICK_MANAGER_PUBLIC_KEY: &str = "{MANAGER_KEY}";'
    has_setter = "fn set_manager" in lib_source or "fn set_policy" in lib_source
    ok = constant in lib_source and SETTER_COMMENT in lib_source and not has_setter
    return {
        "ok": ok,
        "public_key": MANAGER_KEY,
        "constant_present": constant in lib_source,
        "comment_present": SETTER_COMMENT in lib_source,
        "setter_present": has_setter,
        "replacement": "deploy new code",
        "where": "repo source",
    }


def authenticate_outcome(
    *,
    reported: Mapping[str, Any],
    solana: Mapping[str, Any],
    near: Mapping[str, Any],
    floors: Sequence[int],
    quote_deposit_address: str,
) -> Dict[str, Any]:
    """Settled requires the archived deposit and the archived payout to match.

    The reported status alone is the agent's text. Swapped transaction hashes
    fail. This comparison is the archived facts, and it is a Solana light-client
    proof's job to do more than that.
    """
    reasons: List[str] = []
    amount_out = _maybe_int(reported.get("amount_out"))
    near_amount = int(near["amount"])
    hashes = list(reported.get("tx_hashes") or [])
    expected_hashes = [solana["signature"], near["tx_hash"]]
    if reported.get("status") != "Settled":
        reasons.append("status")
    if amount_out != near_amount:
        reasons.append("amount_out")
    if amount_out is None or any(amount_out < int(floor) for floor in floors):
        reasons.append("floor")
    if hashes != expected_hashes:
        reasons.append("tx_hashes")
    if int(solana["deposit_address_delta"]) != int(solana["amount"]):
        reasons.append("solana_deposit")
    if int(solana["funder_delta"]) != -int(solana["amount"]):
        reasons.append("solana_funder")
    if solana["deposit_address"] != quote_deposit_address:
        reasons.append("deposit_address")
    if near.get("recipient") != RECIPIENT or not near.get("logs_ok"):
        reasons.append("near_payout")
    return {
        "ok": not reasons,
        "reasons": reasons,
        "cross_chain_proof": False,
        "where": "archived transfers",
    }


def budget_after_accept(
    *,
    settled: bool,
    funded: bool,
    accepted_at_ms: int,
    now_ms: int,
    window_ms: int,
    quote_consumed: bool,
) -> Dict[str, Any]:
    """An accepted quote that nobody funded returns the budget when the window ends.

    The quote hash stays consumed. A later retry needs a new quote.
    """
    if not quote_consumed:
        raise ForcefieldError("X3_FORCEFIELD_BUDGET", "release follows a successful accept")
    if window_ms <= 0:
        raise ForcefieldError("X3_FORCEFIELD_BUDGET", "the written window must be positive")
    if settled:
        state = "spent"
    elif funded:
        state = "reserved"
    elif now_ms >= accepted_at_ms + window_ms:
        state = "released"
    else:
        state = "reserved"
    return {"state": state, "quote_consumed": True, "where": "local"}


def consider_quote(
    ledger: Mapping[str, Mapping[str, str]],
    *,
    signature_ok: bool,
    policy_ok: bool,
    quote_hash: str,
    deposit_address: str,
    intent_key: str,
) -> Dict[str, Any]:
    """Consume a quote only after the signature and the policy both succeed.

    A bad signature and a rejected policy leave the hash available. A released
    budget does not put the hash back.
    """
    quotes = dict(ledger.get("quotes") or {})
    deposits = dict(ledger.get("deposits") or {})
    if not signature_ok:
        reason = "bad_signature"
        consumed = False
    elif not policy_ok:
        reason = "rejected_policy"
        consumed = False
    elif quote_hash in quotes or deposit_address in deposits:
        reason = "replay"
        consumed = False
    else:
        quotes[quote_hash] = intent_key
        deposits[deposit_address] = intent_key
        reason = "accepted"
        consumed = True
    return {
        "consumed": consumed,
        "reason": reason,
        "ledger": {"quotes": quotes, "deposits": deposits},
        "where": "local",
    }


def classify_keys(keys: Sequence[Mapping[str, Any]]) -> Dict[str, Any]:
    """One FunctionCall key on the agent. Recovery lives on the owner.

    The archived invoice_06 view still has the owner's FullAccess key on the
    agent. This classification reports that. It does not remove the key.
    """
    full: List[str] = []
    function: List[Dict[str, Any]] = []
    other: List[str] = []
    for key in keys:
        permission = (key.get("access_key") or {}).get("permission", key.get("permission"))
        public_key = key.get("public_key")
        if permission == "FullAccess":
            full.append(str(public_key))
        elif isinstance(permission, dict) and "FunctionCall" in permission:
            call = permission["FunctionCall"]
            function.append({
                "public_key": public_key,
                "methods": tuple(call.get("method_names") or ()),
                "receiver_id": call.get("receiver_id"),
            })
        else:
            other.append(str(public_key))
    separated = (
        not full
        and not other
        and len(function) == 1
        and function[0]["methods"] == AGENT_METHODS
        and function[0]["receiver_id"] == GUARD_ACCOUNT
    )
    if not keys or other:
        status = "fail"
        open_items: List[str] = []
    elif separated:
        status = "pass"
        open_items = []
    else:
        status = "action_required"
        open_items = [
            "Move the FullAccess recovery key off the agent account. Recovery stays with the owner."
        ]
    return {
        "separated": separated,
        "status": status,
        "full_access": full,
        "function_call": function,
        "open": open_items,
        "where": "archived keys",
    }


def compile_payment(text: str, *, now: Optional[datetime] = None) -> Dict[str, Any]:
    adapter = _near()
    with tempfile.TemporaryDirectory() as directory:
        path = Path(directory) / "payment.x3"
        path.write_text(text, encoding="utf-8")
        try:
            adapter.compile_program(str(path), now=now)
        except adapter.NearIntentsError as exc:
            detail = exc.detail if isinstance(exc.detail, list) else []
            inner = sorted({item.get("code") for item in detail if isinstance(item, dict) and item.get("code")})
            return {"compiled": False, "code": exc.code, "inner": inner}
    return {"compiled": True, "code": None, "inner": []}


def compiler_refusals(source: str, *, now: Optional[datetime] = None) -> Dict[str, Any]:
    baseline = compile_payment(source, now=now)
    rows = []
    for edit in COMPILER_EDITS:
        if source.count(edit["old"]) != 1:
            raise ForcefieldError("X3_FORCEFIELD_EDIT", f"{edit['case']} anchor is not unique")
        got = compile_payment(source.replace(edit["old"], edit["new"], 1), now=now)
        inner = edit.get("inner")
        passed = (
            not got["compiled"]
            and got["code"] == edit["code"]
            and (inner is None or inner in got["inner"])
        )
        rows.append({"case": edit["case"], "code": got["code"], "inner": got["inner"], "pass": passed})
    attacker = source.replace(
        "to Near.USDC receiver x3lang.near",
        "to Near.USDC receiver attacker.near",
        1,
    )
    attacker_result = compile_payment(attacker, now=now)
    ok = baseline["compiled"] and all(row["pass"] for row in rows) and attacker_result["compiled"]
    return {
        "ok": ok,
        "baseline_compiles": baseline["compiled"],
        "cases": rows,
        "attacker_compiles": attacker_result["compiled"],
        "where": "compiler",
    }


def split_forcefield(source: str) -> Tuple[str, Dict[str, List[str]]]:
    """Drop local clauses. The text that remains is the official mandate grammar."""
    extras: Dict[str, List[str]] = {"deny_recipients": [], "allow_programs": [], "agents": []}
    kept: List[str] = []
    for line in source.splitlines(keepends=True):
        parsed = _extension_line(line.strip())
        if parsed is None:
            kept.append(line)
            continue
        key, values = parsed
        if extras[key]:
            raise ForcefieldError("X3_FORCEFIELD_CLAUSE", f"{key} appears twice")
        extras[key] = values
    return "".join(kept), extras


def compile_mandate_source(source: str, tokens: Sequence[Mapping[str, Any]], *, now: Optional[datetime] = None) -> Dict[str, Any]:
    stripped, extras = split_forcefield(source)
    adapter = _near()
    mandates = adapter.mandates()
    with tempfile.TemporaryDirectory() as directory:
        path = Path(directory) / "mandate.x3"
        data = stripped.encode("utf-8")
        path.write_bytes(data)
        compiled = mandates.compile_mandate(str(path), tokens, now=now)
    chain_terms = compiled["terms"]
    spilled = {"deny_recipients", "allow_programs", "agents"} & set(chain_terms)
    if spilled:
        raise ForcefieldError("X3_FORCEFIELD_TERMS", "local clauses reached the chain terms")
    return {
        "compiled": compiled,
        "extras": extras,
        "stripped": stripped,
        "chain_terms": chain_terms,
        "full_sha256": sha256_text(source),
        "stripped_sha256": hashlib.sha256(data).hexdigest(),
    }


def mandate_diff(before: str, after: str) -> Dict[str, Any]:
    """A line diff and a clause diff of two mandate sources."""
    import difflib

    before_lines = before.splitlines()
    after_lines = after.splitlines()
    line_diff = list(difflib.unified_diff(before_lines, after_lines, fromfile="before", tofile="after", lineterm=""))
    before_clauses = _clauses(before)
    after_clauses = _clauses(after)
    keys = sorted(set(before_clauses) | set(after_clauses))
    changed = [key for key in keys if before_clauses.get(key) != after_clauses.get(key)]
    return {"line_diff": line_diff, "changed": changed, "identical": before == after}


def permission_after_signature(
    mandate: Mapping[str, Any],
    policy: Mapping[str, Any],
    now_ms: int,
    *,
    signature_valid: bool,
) -> Dict[str, Any]:
    """A manager signature that breaks the mandate still fails the named guard.

    `signature_valid` is the caller's archived receipt bit. This function does
    not run the ed25519 verification again.
    """
    checks = _near().mandates().check_mandate(mandate, policy, now_ms)
    failed = [row["guard"] for row in checks if not row["ok"]]
    return {
        "signature_valid": bool(signature_valid),
        "signature_reverified": False,
        "failed": failed,
        "permitted": bool(signature_valid) and not failed,
        "checks": checks,
    }


def apply_approval(
    terms: Mapping[str, Any],
    *,
    approver: str,
    program_sha256: str,
    widen: Optional[Mapping[str, Any]] = None,
) -> Dict[str, Any]:
    """Approval records the approver and the program hash. It leaves the terms."""
    proposed = dict(widen or {})
    refused: List[str] = []
    if "recipient" in proposed and proposed["recipient"] not in terms["recipients"]:
        refused.append("recipient")
    if "per_payment_cap" in proposed and int(proposed["per_payment_cap"]) != int(terms["per_payment_cap"]):
        refused.append("per_payment_cap")
    if "refund_to" in proposed and proposed["refund_to"] != terms["refund_to"]:
        refused.append("refund_to")
    snapshot = canonical_json(terms)
    if refused:
        if canonical_json(terms) != snapshot:
            raise ForcefieldError("X3_FORCEFIELD_APPROVAL", "a refused approval changed the terms")
        return {"ok": False, "refused": refused, "terms": terms, "approvals": []}
    if not approver or not program_sha256:
        raise ForcefieldError("X3_FORCEFIELD_APPROVAL", "an approval names an approver and a program hash")
    return {
        "ok": True,
        "refused": [],
        "terms": terms,
        "approvals": [{"approver": approver, "program_sha256": program_sha256}],
    }


def recipient_decision(recipient: str, *, deny: Sequence[str], allow: Sequence[str]) -> Dict[str, Any]:
    """A name on both lists is denied. The denylist is local and stays off the chain terms."""
    if recipient in deny:
        return {"ok": False, "guard": "mandate_deny_recipient", "where": "local"}
    if recipient not in allow:
        return {"ok": False, "guard": "mandate_recipient", "where": "deployed guard"}
    return {"ok": True, "guard": "mandate_recipient", "where": "deployed guard"}


def program_allowed(program_sha256: str, pins: Sequence[str], used: Mapping[str, int]) -> Dict[str, Any]:
    """An empty pin list keeps today's behavior: the program hash is recorded, and any program is eligible.

    A pin list allows that hash once. Another program needs a new mandate version.
    """
    if not pins:
        return {"ok": True, "pinned": False, "guard": None, "where": "deployed guard"}
    if program_sha256 not in pins:
        return {"ok": False, "pinned": True, "guard": "mandate_program", "where": "local"}
    if int(used.get(program_sha256, 0)) > 0:
        return {"ok": False, "pinned": True, "guard": "mandate_program_reuse", "where": "local"}
    return {"ok": True, "pinned": True, "guard": "mandate_program", "where": "local"}


def requires_new_version(pins: Sequence[str], program_sha256: str) -> bool:
    return bool(pins) and program_sha256 not in pins


def link(
    *,
    previous_hash: Optional[str],
    version: int,
    mandate_sha256: str,
    program_sha256: str,
    terms: Mapping[str, Any],
) -> Dict[str, Any]:
    """Each mandate version commits to the previous version's hash."""
    if version < 1:
        raise ForcefieldError("X3_FORCEFIELD_CHAIN", "version starts at 1")
    previous = GENESIS_HASH if previous_hash is None else previous_hash
    _hex64(previous, "previous_hash")
    _hex64(mandate_sha256, "mandate_sha256")
    _hex64(program_sha256, "program_sha256")
    body = {
        "mandate_sha256": mandate_sha256,
        "previous_hash": previous,
        "program_sha256": program_sha256,
        "terms": terms,
        "version": version,
    }
    encoded = canonical_json(body)
    return {**body, "chain_hash": sha256_text(encoded), "canonical": encoded}


def verify_chain(entries: Sequence[Mapping[str, Any]]) -> bool:
    previous = GENESIS_HASH
    for entry in entries:
        if entry["previous_hash"] != previous:
            return False
        recomputed = link(
            previous_hash=entry["previous_hash"],
            version=int(entry["version"]),
            mandate_sha256=entry["mandate_sha256"],
            program_sha256=entry["program_sha256"],
            terms=entry["terms"],
        )
        if recomputed["chain_hash"] != entry["chain_hash"]:
            return False
        previous = entry["chain_hash"]
    return True


def record_attempt(log: Sequence[Mapping[str, Any]], **certificate_fields: str) -> Dict[str, Any]:
    """Each denial stores a refusal certificate until the attempt cap is spent."""
    if len(log) >= ATTEMPT_CAP:
        return {
            "ok": False,
            "guard": "attempt_budget",
            "log": list(log),
            "certificate": None,
            "used": len(log),
            "where": "local",
        }
    certificate = refusal_certificate(**certificate_fields)
    stored = list(log) + [certificate]
    return {
        "ok": True,
        "guard": certificate["guard"],
        "log": stored,
        "certificate": certificate,
        "used": len(stored),
        "where": "local",
    }


def shared_spent(
    spends_by_agent: Mapping[str, Sequence[Mapping[str, Any]]],
    now_ms: int,
    agents: Sequence[str],
    window_ms: int = WINDOW_MS,
) -> Tuple[int, int]:
    """Sum the named agents' spends inside one rolling window."""
    since = max(now_ms - window_ms, 0)
    total = 0
    count = 0
    for agent in agents:
        for spend in spends_by_agent.get(agent, ()):
            if int(spend["at_ms"]) > since:
                total += int(spend["amount"])
                count += 1
    return total, count


def shared_ceiling_ok(spent: int, count: int, amount: int, daily_cap: int, max_payments: int = 64) -> bool:
    return spent + amount <= daily_cap and count < max_payments


def vault_may_transfer(
    *,
    guards_ok: bool,
    method: str,
    predecessor_is_agent: bool,
    withdraw: bool,
) -> Dict[str, Any]:
    """A transfer clears the same mandate checks. Withdraw is closed to the agent key."""
    ok = bool(guards_ok) and method == VAULT_SPEND and bool(predecessor_is_agent) and not withdraw
    return {
        "ok": ok,
        "method": method,
        "predecessor_is_agent": bool(predecessor_is_agent),
        "withdraw": bool(withdraw),
        "guards_ok": bool(guards_ok),
        "deployed": False,
        "where": "local",
    }


def proof_page(payment: str, mandate: str) -> str:
    payment_sha = sha256_text(payment)
    mandate_sha = sha256_text(mandate)
    return f"""<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>invoice_06 source hashes</title>
</head>
<body>
<h1>invoice_06 source hashes</h1>
<p>This page recomputes SHA-256 of the two embedded files. It uses the script next to it. The deployed Guard stored these hashes and compiled neither file.</p>
<pre id="out">computing</pre>
<script type="text/plain" id="payment-source">{payment}</script>
<script type="text/plain" id="mandate-source">{mandate}</script>
<script src="forcefield-sha256.js"></script>
<script>
var expected = {{
  payment: "{payment_sha}",
  mandate: "{mandate_sha}"
}};
var paymentText = document.getElementById("payment-source").textContent;
var mandateText = document.getElementById("mandate-source").textContent;
var paymentGot = sha256Hex(paymentText);
var mandateGot = sha256Hex(mandateText);
var ok = paymentGot === expected.payment && mandateGot === expected.mandate;
document.getElementById("out").textContent = [
  "payment " + paymentGot,
  "mandate " + mandateGot,
  ok ? "match" : "differ"
].join("\\n");
</script>
</body>
</html>
"""


def extract_source(page: str, element_id: str) -> str:
    start = f'<script type="text/plain" id="{element_id}">'
    begin = page.find(start)
    if begin < 0:
        raise ForcefieldError("X3_FORCEFIELD_PROOF", f"missing {element_id}")
    begin += len(start)
    end = page.find("</script>", begin)
    if end < 0:
        raise ForcefieldError("X3_FORCEFIELD_PROOF", f"{element_id} is unclosed")
    return page[begin:end]


def evaluate() -> Dict[str, Any]:
    """Run the twenty checks against the archived invoice_06 payment."""
    context = _load_invoice()
    rows = [
        _check_certificate(context),
        _check_bypass(),
        _check_outcome(context),
        _check_keys(context),
        _check_release(context),
        _check_verifier(),
        _check_compiler(context),
        _check_diff(context),
        _check_signature(context),
        _check_approval(context),
        _check_burn(context),
        _check_purpose(context),
        _check_denylist(),
        _check_program(context),
        _check_chain(context),
        _check_proof(),
        _check_shared(context),
        _check_refund(context),
        _check_attempts(context),
        _check_vault(context),
    ]
    if [row["name"] for row in rows] != list(CHECKS):
        raise ForcefieldError("X3_FORCEFIELD_REPORT", "the check list drifted")
    open_items: List[str] = []
    for row in rows:
        open_items.extend(row.get("open") or [])
    return {
        "ok": all(row["status"] != "fail" for row in rows),
        "open": open_items,
        "rows": rows,
        "limits": list(LIMITS),
    }


def render(report: Mapping[str, Any]) -> str:
    lines = [
        "X3 forcefield",
        "Local checks beside the deployed Guard. This command signs and deploys nothing.",
        f"ok: {str(report['ok']).lower()}",
    ]
    if report["open"]:
        lines.append("open:")
        lines.extend(f"  {item}" for item in report["open"])
    lines.append("")
    for row in report["rows"]:
        mark = {"pass": "PASS", "open": "OPEN", "fail": "FAIL"}[row["status"]]
        lines.append(f"{row['n']:2}  {mark:4}  {row['where']:22}  {row['title']}")
        lines.append(f"    {row['summary']}")
    lines.append("")
    lines.append("limits:")
    lines.extend(f"  {item}" for item in report["limits"])
    return "\n".join(lines) + "\n"


def _check_certificate(context: Mapping[str, Any]) -> Dict[str, Any]:
    certificate = refusal_certificate(
        program_sha256=context["attacker_sha256"],
        mandate_sha256=context["mandate_sha256"],
        guard="mandate_recipient",
        quote_hash=context["quote_hash"],
        detail="attacker.near",
    )
    again = refusal_certificate(
        program_sha256=context["attacker_sha256"],
        mandate_sha256=context["mandate_sha256"],
        guard="mandate_recipient",
        quote_hash=context["quote_hash"],
        detail="attacker.near",
    )
    ok = certificate == again and certificate["guard"] == "mandate_recipient"
    return _row(
        "refusal_certificate",
        "pass" if ok else "fail",
        "local",
        f"guard {certificate['guard']}  program {certificate['program_sha256'][:12]}  quote {certificate['quote_hash']}",
        {"certificate_sha256": certificate["certificate_sha256"]},
    )


def _check_bypass() -> Dict[str, Any]:
    found = scan_contract_source(GUARD_LIB.read_text(encoding="utf-8"), GUARD_MANDATE.read_text(encoding="utf-8"))
    summary = "revoke, approve, and storage recovery are present; bypass methods are absent"
    if not found["ok"]:
        summary = "source scan found a bypass or a missing owner method"
    return _row("no_bypass_method", "pass" if found["ok"] else "fail", found["where"], summary, found)


def _check_outcome(context: Mapping[str, Any]) -> Dict[str, Any]:
    live = authenticate_outcome(
        reported=context["reported_outcome"],
        solana=context["solana"],
        near=context["near"],
        floors=context["floors"],
        quote_deposit_address=context["deposit_address"],
    )
    swapped = dict(context["reported_outcome"])
    swapped["tx_hashes"] = list(reversed(swapped["tx_hashes"]))
    mutated = authenticate_outcome(
        reported=swapped,
        solana=context["solana"],
        near=context["near"],
        floors=context["floors"],
        quote_deposit_address=context["deposit_address"],
    )
    historical_null = context["historical_outcome"] is None
    ok = (
        live["ok"]
        and context["solana"]["check_passed"]
        and not mutated["ok"]
        and "tx_hashes" in mutated["reasons"]
        and historical_null
    )
    return _row(
        "outcome_authentication",
        "pass" if ok else "fail",
        live["where"],
        "archived deposit and payout match; swapped hashes are refused",
        {"live": live["reasons"], "swapped": mutated["reasons"], "historical_outcome": context["historical_outcome"]},
    )


def _check_keys(context: Mapping[str, Any]) -> Dict[str, Any]:
    classified = classify_keys(context["keys"])
    status = {"pass": "pass", "action_required": "open", "fail": "fail"}[classified["status"]]
    summary = "the agent key is the FunctionCall key alone"
    if classified["full_access"]:
        summary = f"FullAccess remains: {classified['full_access'][0]}"
    return _row("one_key", status, classified["where"], summary, classified, open_items=classified["open"])


def _check_release(context: Mapping[str, Any]) -> Dict[str, Any]:
    accepted = context["accepted_at_ms"]
    window = context["window_ms"]
    held = budget_after_accept(
        settled=False, funded=False, accepted_at_ms=accepted, now_ms=accepted + window - 1,
        window_ms=window, quote_consumed=True,
    )
    released = budget_after_accept(
        settled=False, funded=False, accepted_at_ms=accepted, now_ms=accepted + window,
        window_ms=window, quote_consumed=True,
    )
    funded = budget_after_accept(
        settled=False, funded=True, accepted_at_ms=accepted, now_ms=accepted + window,
        window_ms=window, quote_consumed=True,
    )
    spent = budget_after_accept(
        settled=True, funded=True, accepted_at_ms=accepted, now_ms=accepted + window,
        window_ms=window, quote_consumed=True,
    )
    ok = (
        held["state"] == "reserved"
        and released["state"] == "released"
        and released["quote_consumed"]
        and funded["state"] == "reserved"
        and spent["state"] == "spent"
    )
    return _row(
        "timeout_releases_budget",
        "pass" if ok else "fail",
        "local",
        "an unfunded accept returns the budget at the written window and keeps the quote consumed",
        {"held": held["state"], "released": released["state"], "funded": funded["state"], "spent": spent["state"]},
    )


def _check_verifier() -> Dict[str, Any]:
    found = verifier_key_lock(GUARD_LIB.read_text(encoding="utf-8"))
    summary = f"{MANAGER_KEY}  replacement is a new deployment"
    if not found["ok"]:
        summary = "the pinned verifier key or its no-setter comment moved"
    return _row("verifier_key_has_no_setter", "pass" if found["ok"] else "fail", found["where"], summary, found)


def _check_compiler(context: Mapping[str, Any]) -> Dict[str, Any]:
    found = compiler_refusals(context["payment_text"], now=context["compiled_at"])
    failed = [row["case"] for row in found["cases"] if not row["pass"]]
    summary = "slippage, timeout, nonce, a second leg, a look-alike, a zero amount, and a finality promise are refused"
    if failed or not found["ok"]:
        summary = "compiler cases failed: " + ", ".join(failed or ["baseline"])
    return _row(
        "unenforceable_does_not_compile",
        "pass" if found["ok"] else "fail",
        found["where"],
        summary,
        found,
    )


def _check_diff(context: Mapping[str, Any]) -> Dict[str, Any]:
    same = mandate_diff(context["mandate_text"], context["mandate_text"])
    changed = mandate_diff(
        context["mandate_text"],
        context["mandate_text"].replace(REFUND_WALLET, "DifferentRefundWallet1111111111111111111111111", 1),
    )
    ok = same["identical"] and same["changed"] == [] and changed["changed"] == ["refund_to"]
    return _row(
        "mandate_diff",
        "pass" if ok else "fail",
        "local",
        "the invoice_06 mandate diffs clean, and a refund-wallet edit shows refund_to",
        {"self": same["changed"], "refund_edit": changed["changed"]},
    )


def _check_signature(context: Mapping[str, Any]) -> Dict[str, Any]:
    allowed = permission_after_signature(
        context["mandate"], context["policy"], context["accepted_at_ms"], signature_valid=True,
    )
    unsigned = permission_after_signature(
        context["mandate"], context["policy"], context["accepted_at_ms"], signature_valid=False,
    )
    attacker = permission_after_signature(
        context["mandate"], context["attacker_policy"], context["accepted_at_ms"], signature_valid=True,
    )
    ok = (
        allowed["permitted"]
        and not unsigned["permitted"]
        and not attacker["permitted"]
        and attacker["failed"] == ["mandate_recipient"]
        and attacker["signature_reverified"] is False
    )
    return _row(
        "signature_is_not_permission",
        "pass" if ok else "fail",
        "deployed guard",
        "a passing receipt bit still fails mandate_recipient for attacker.near",
        {"attacker_failed": attacker["failed"], "signature_reverified": False},
    )


def _check_approval(context: Mapping[str, Any]) -> Dict[str, Any]:
    terms = context["terms"]
    before = canonical_json(terms)
    widened = apply_approval(
        terms,
        approver=GUARD_ACCOUNT,
        program_sha256=context["payment_sha256"],
        widen={"recipient": "attacker.near", "per_payment_cap": "25000001", "refund_to": "elsewhere"},
    )
    signed = apply_approval(terms, approver=GUARD_ACCOUNT, program_sha256=context["payment_sha256"])
    ok = (
        not widened["ok"]
        and widened["refused"] == ["recipient", "per_payment_cap", "refund_to"]
        and signed["ok"]
        and signed["approvals"] == [{"approver": GUARD_ACCOUNT, "program_sha256": context["payment_sha256"]}]
        and canonical_json(terms) == before
        and canonical_json(signed["terms"]) == before
    )
    return _row(
        "approval_cannot_widen",
        "pass" if ok else "fail",
        "local",
        "approval stores the program hash and leaves the recipient, cap, and refund wallet",
        {"refused": widened["refused"]},
    )


def _check_burn(context: Mapping[str, Any]) -> Dict[str, Any]:
    comment = NO_CONSUME_COMMENT in GUARD_LIB.read_text(encoding="utf-8")
    bad_signature = consider_quote(
        empty_ledger(), signature_ok=False, policy_ok=True, quote_hash="quote-a",
        deposit_address="deposit-a", intent_key="intent-a",
    )
    bad_policy = consider_quote(
        empty_ledger(), signature_ok=True, policy_ok=False, quote_hash="quote-a",
        deposit_address="deposit-a", intent_key="intent-a",
    )
    accepted = consider_quote(
        empty_ledger(), signature_ok=True, policy_ok=True, quote_hash=context["quote_hash"],
        deposit_address=context["deposit_address"], intent_key="invoice_06",
    )
    released = budget_after_accept(
        settled=False, funded=False, accepted_at_ms=context["accepted_at_ms"],
        now_ms=context["accepted_at_ms"] + context["window_ms"],
        window_ms=context["window_ms"], quote_consumed=accepted["consumed"],
    )
    replay = consider_quote(
        accepted["ledger"], signature_ok=True, policy_ok=True, quote_hash=context["quote_hash"],
        deposit_address=context["deposit_address"], intent_key="invoice_06-again",
    )
    ok = (
        comment
        and not bad_signature["consumed"]
        and bad_signature["reason"] == "bad_signature"
        and not bad_policy["consumed"]
        and bad_policy["reason"] == "rejected_policy"
        and accepted["consumed"]
        and released["state"] == "released"
        and released["quote_consumed"]
        and not replay["consumed"]
        and replay["reason"] == "replay"
        and context["quote_hash"] in accepted["ledger"]["quotes"]
    )
    return _row(
        "burned_quote_stays_burned",
        "pass" if ok else "fail",
        "local",
        "a rejected call leaves the quote, and a released budget keeps it consumed",
        {"comment": comment, "replay": replay["reason"]},
    )


def _check_purpose(context: Mapping[str, Any]) -> Dict[str, Any]:
    source = GUARD_MANDATE.read_text(encoding="utf-8")
    signatures = {
        name: function_signature(source, name)
        for name in ("agent_accept_quote", "agent_record_outcome")
    }
    purpose_in_source = {
        name: [word for word in PURPOSE_FIELDS if word in signature]
        for name, signature in signatures.items()
    }
    calls = context["agent_calls"]
    ok = (
        all(not hits for hits in purpose_in_source.values())
        and len(calls) == 2
        and all(call["deposit"] == "0" and not call["purpose_fields"] and call["actions"] == 1 for call in calls)
    )
    methods = ", ".join(call["method_name"] for call in calls)
    return _row(
        "no_purpose_string",
        "pass" if ok else "fail",
        "repo source",
        f"{methods} attach zero NEAR and carry no purpose field",
        {"calls": calls, "source": purpose_in_source},
    )


def _check_denylist() -> Dict[str, Any]:
    allowed = recipient_decision("x3lang.near", deny=("attacker.near",), allow=("x3lang.near",))
    missing = recipient_decision("attacker.near", deny=(), allow=("x3lang.near",))
    both = recipient_decision("x3lang.near", deny=("x3lang.near",), allow=("x3lang.near",))
    ok = (
        allowed["ok"]
        and allowed["guard"] == "mandate_recipient"
        and not missing["ok"]
        and missing["guard"] == "mandate_recipient"
        and not both["ok"]
        and both["guard"] == "mandate_deny_recipient"
    )
    return _row(
        "denylist_outranks_allowlist",
        "pass" if ok else "fail",
        "local",
        "a name written on both lists is denied before the allowlist is consulted",
        {"both": both["guard"]},
    )


def _check_program(context: Mapping[str, Any]) -> Dict[str, Any]:
    payment = context["payment_sha256"]
    other = context["attacker_sha256"]
    unpinned = program_allowed(other, (), {})
    first = program_allowed(payment, (payment,), {})
    second = program_allowed(payment, (payment,), {payment: 1})
    foreign = program_allowed(other, (payment,), {})
    ok = (
        unpinned["ok"] and unpinned["pinned"] is False
        and first["ok"]
        and not second["ok"] and second["guard"] == "mandate_program_reuse"
        and not foreign["ok"] and foreign["guard"] == "mandate_program"
        and requires_new_version((payment,), other)
        and requires_new_version((), other) is False
    )
    return _row(
        "program_hash_cap",
        "pass" if ok else "fail",
        "local",
        "with no pin, any program stays eligible; pinning the invoice_06 hash allows it once",
        {"second": second["guard"], "foreign": foreign["guard"]},
    )


def _check_chain(context: Mapping[str, Any]) -> Dict[str, Any]:
    terms = context["terms"]
    first = link(
        previous_hash=None,
        version=1,
        mandate_sha256=context["mandate_sha256"],
        program_sha256=terms["program_sha256"],
        terms=terms,
    )
    second = link(
        previous_hash=first["chain_hash"],
        version=2,
        mandate_sha256=context["mandate_sha256"],
        program_sha256=terms["program_sha256"],
        terms=terms,
    )
    tampered = dict(second)
    tampered_terms = dict(terms)
    tampered_terms["per_payment_cap"] = "1"
    tampered["terms"] = tampered_terms
    broken_previous = dict(second)
    broken_previous["previous_hash"] = GENESIS_HASH
    ok = (
        first["previous_hash"] == GENESIS_HASH
        and verify_chain([first, second])
        and verify_chain([first, tampered]) is False
        and verify_chain([broken_previous]) is False
    )
    return _row(
        "mandate_hash_chain",
        "pass" if ok else "fail",
        "local",
        "a local chain starts at 64 zeros; a changed term breaks the link",
        {"version_1": first["chain_hash"], "version_2": second["chain_hash"]},
    )


def _check_proof() -> Dict[str, Any]:
    payment = PAYMENT.read_text(encoding="utf-8")
    mandate = MANDATE.read_text(encoding="utf-8")
    page = PROOF_PAGE.read_text(encoding="utf-8")
    expected = proof_page(payment, mandate)
    extracted_payment = extract_source(page, "payment-source")
    extracted_mandate = extract_source(page, "mandate-source")
    ok = (
        page == expected
        and extracted_payment == payment
        and extracted_mandate == mandate
        and sha256_text(extracted_payment) == sha256_file(PAYMENT)
        and sha256_text(extracted_mandate) == sha256_file(MANDATE)
        and SHA256_JS.is_file()
    )
    return _row(
        "proof_recompute",
        "pass" if ok else "fail",
        "local",
        f"payment {sha256_file(PAYMENT)}  mandate {sha256_file(MANDATE)}",
        {"page": str(PROOF_PAGE.relative_to(ROOT))},
    )


def _check_shared(context: Mapping[str, Any]) -> Dict[str, Any]:
    agent = context["compiled"]["agent"]
    other = "shopper.x3lang.near"
    cap = int(context["terms"]["daily_cap"])
    amount = int(context["policy"]["amount"])
    now_ms = context["accepted_at_ms"]
    spends = {agent: [{"at_ms": now_ms, "amount": str(cap)}]}
    spent, count = shared_spent(spends, now_ms + 1, (agent, other))
    blocked = shared_ceiling_ok(spent, count, 1, cap)
    old = {agent: [{"at_ms": now_ms - WINDOW_MS, "amount": str(cap)}]}
    aged, aged_count = shared_spent(old, now_ms, (agent, other))
    aged_ok = shared_ceiling_ok(aged, aged_count, amount, cap)
    one_agent, one_count = shared_spent(spends, now_ms + 1, (agent,))
    ok = (
        not blocked
        and aged_ok
        and one_agent == cap
        and one_count == 1
        and context["mandate_text"].count("\n    agent ") == 1
    )
    return _row(
        "shared_daily_ceiling",
        "pass" if ok else "fail",
        "local",
        "the invoice_06 mandate names one agent; a second agent's spend shares per_24h",
        {"shared_spent": spent, "second_agent_allowed": blocked, "aged_window_allows": aged_ok},
    )


def _check_refund(context: Mapping[str, Any]) -> Dict[str, Any]:
    moved = dict(context["policy"])
    moved["refund_to"] = "DifferentRefundWallet1111111111111111111111111"
    decision = permission_after_signature(
        context["mandate"], moved, context["accepted_at_ms"], signature_valid=True,
    )
    pinned = permission_after_signature(
        context["mandate"], context["policy"], context["accepted_at_ms"], signature_valid=True,
    )
    ok = (
        pinned["permitted"]
        and not decision["permitted"]
        and "mandate_refund_to" in decision["failed"]
        and context["terms"]["refund_to"] == REFUND_WALLET
        and decision["signature_reverified"] is False
    )
    return _row(
        "refund_wallet_pinned",
        "pass" if ok else "fail",
        "deployed guard",
        "a signed quote whose refund_to leaves the mandate fails mandate_refund_to",
        {"failed": decision["failed"]},
    )


def _check_attempts(context: Mapping[str, Any]) -> Dict[str, Any]:
    log: List[Mapping[str, Any]] = []
    last: Dict[str, Any] = {"log": log}
    for _ in range(ATTEMPT_CAP):
        last = record_attempt(
            last["log"],
            program_sha256=context["attacker_sha256"],
            mandate_sha256=context["mandate_sha256"],
            guard="mandate_recipient",
            quote_hash=context["quote_hash"],
            detail="attacker.near",
        )
        if not last["ok"]:
            break
    overflow = record_attempt(
        last["log"],
        program_sha256=context["attacker_sha256"],
        mandate_sha256=context["mandate_sha256"],
        guard="mandate_recipient",
        quote_hash=context["quote_hash"],
    )
    ok = (
        last["ok"]
        and last["used"] == ATTEMPT_CAP
        and len(last["log"]) == ATTEMPT_CAP
        and all(item["guard"] == "mandate_recipient" and item["certificate_sha256"] for item in last["log"])
        and not overflow["ok"]
        and overflow["guard"] == "attempt_budget"
        and overflow["certificate"] is None
        and len(overflow["log"]) == ATTEMPT_CAP
    )
    return _row(
        "attempt_budget",
        "pass" if ok else "fail",
        "local",
        f"{ATTEMPT_CAP} denials store certificates; the next denial is attempt_budget",
        {"used": last["used"], "overflow": overflow["guard"]},
    )


def _check_vault(context: Mapping[str, Any]) -> Dict[str, Any]:
    allowed = vault_may_transfer(
        guards_ok=True, method=VAULT_SPEND, predecessor_is_agent=True, withdraw=False,
    )
    withdraw = vault_may_transfer(
        guards_ok=True, method="withdraw", predecessor_is_agent=True, withdraw=True,
    )
    stranger = vault_may_transfer(
        guards_ok=True, method=VAULT_SPEND, predecessor_is_agent=False, withdraw=False,
    )
    unguarded = vault_may_transfer(
        guards_ok=False, method=VAULT_SPEND, predecessor_is_agent=True, withdraw=False,
    )
    methods = context["keys_classified"]["function_call"][0]["methods"]
    ok = (
        allowed["ok"]
        and not withdraw["ok"]
        and not stranger["ok"]
        and not unguarded["ok"]
        and "withdraw" not in methods
        and methods == AGENT_METHODS
    )
    return _row(
        "vault_gate",
        "pass" if ok else "fail",
        "local",
        "vault_spend follows the mandate checks; the agent key has no withdraw method",
        {"allowed": allowed["ok"], "withdraw": withdraw["ok"]},
    )


def _row(
    name: str,
    status: str,
    where: str,
    summary: str,
    detail: Mapping[str, Any],
    open_items: Optional[Sequence[str]] = None,
) -> Dict[str, Any]:
    return {
        "n": CHECKS.index(name) + 1,
        "name": name,
        "title": TITLES[name],
        "status": status,
        "where": where,
        "summary": summary,
        "detail": detail,
        "open": list(open_items or []),
    }


def _load_invoice() -> Dict[str, Any]:
    payment_text = PAYMENT.read_text(encoding="utf-8")
    mandate_text = MANDATE.read_text(encoding="utf-8")
    observation = json.loads(OBSERVATION.read_text(encoding="utf-8"))
    outcome = json.loads(OUTCOME.read_text(encoding="utf-8"))
    tokens = json.loads(TOKENS.read_text(encoding="utf-8"))
    adapter = _near()
    if payment_text.count("timeout 74h") != 1 or mandate_text.count("expires 74h") != 1:
        raise ForcefieldError("X3_FORCEFIELD_WINDOW", "the written window is 74h on both files")
    deadline_ms = int(observation["intent"]["value"]["policy"]["deadline_ms"])
    window = adapter.parse_duration("74h")
    window_ms = int(window.total_seconds() * 1000)
    compiled_at = datetime(1970, 1, 1, tzinfo=timezone.utc) + timedelta(milliseconds=deadline_ms - window_ms)
    mandates = adapter.mandates()
    compiled = compile_mandate_source(mandate_text, tokens, now=compiled_at)
    if compiled["extras"]["deny_recipients"] or compiled["extras"]["allow_programs"] or compiled["extras"]["agents"]:
        raise ForcefieldError("X3_FORCEFIELD_INVOICE", "invoice_06 mandate gained local clauses")
    _, leg = adapter.compile_program(str(PAYMENT), now=compiled_at)
    policy = _policy(adapter, leg, tokens, sha256_file(PAYMENT))
    attacker_text = payment_text.replace(
        "to Near.USDC receiver x3lang.near",
        "to Near.USDC receiver attacker.near",
        1,
    )
    with tempfile.TemporaryDirectory() as directory:
        attacker_path = Path(directory) / "attacker.x3"
        attacker_path.write_text(attacker_text, encoding="utf-8")
        attacker_sha = sha256_file(attacker_path)
        _, attacker_leg = adapter.compile_program(str(attacker_path), now=compiled_at)
    attacker_policy = _policy(adapter, attacker_leg, tokens, attacker_sha)
    intent = outcome["intent_after"]["value"]
    quote = intent["quote"]["facts"]
    reported = intent["outcome"]
    solana_check = _one_check(observation, "sol_25_usdc_funder_to_deposit_address")
    payout_check = _one_check(observation, "payout_logs_name_recipient_and_amount")
    logs = payout_check["detail"]
    logs_text = "".join(logs) if isinstance(logs, list) else str(logs)
    near_tx = observation["near_transactions"]["payout"]
    accept_tx = observation["near_transactions"]["agent_accept_quote"]
    outcome_tx = outcome["rpc_transaction"]["transaction"]
    return {
        "payment_text": payment_text,
        "mandate_text": mandate_text,
        "payment_sha256": sha256_file(PAYMENT),
        "mandate_sha256": sha256_file(MANDATE),
        "attacker_sha256": attacker_sha,
        "compiled_at": compiled_at,
        "compiled": compiled["compiled"],
        "terms": compiled["chain_terms"],
        "mandate": {
            "owner": compiled["compiled"]["owner"],
            "agent": compiled["compiled"]["agent"],
            "terms": compiled["chain_terms"],
            "version": observation["mandate"]["value"]["version"],
            "revoked": False,
            "spends": [],
        },
        "policy": policy,
        "attacker_policy": attacker_policy,
        "accepted_at_ms": int(intent["quote"]["accepted_at_ms"]),
        "window_ms": window_ms,
        "quote_hash": quote["quote_hash"],
        "deposit_address": quote["deposit_address"],
        "floors": [int(quote["min_amount_out"]), int(intent["policy"]["min_receive"])],
        "reported_outcome": reported,
        "historical_outcome": observation["intent"]["value"]["outcome"],
        "solana": {
            "signature": observation["solana_deposit"]["signature"],
            "amount": 25_000_000,
            "deposit_address": quote["deposit_address"],
            "deposit_address_delta": solana_check["detail"]["deposit_address_delta"],
            "funder_delta": solana_check["detail"]["funder_delta"],
            "check_passed": solana_check["result"] == "pass",
        },
        "near": {
            "tx_hash": near_tx["hash"],
            "recipient": RECIPIENT,
            "amount": 24_995_200,
            "logs_ok": payout_check["result"] == "pass" and "24995200" in logs_text and RECIPIENT in logs_text,
        },
        "keys": observation["agent_access_keys"]["result"]["keys"],
        "keys_classified": classify_keys(observation["agent_access_keys"]["result"]["keys"]),
        "agent_calls": [
            _call_shape(accept_tx),
            _call_shape(outcome_tx),
        ],
        "observation_solana_passed": solana_check["result"] == "pass",
    }


def _call_shape(transaction: Mapping[str, Any]) -> Dict[str, Any]:
    actions = transaction["actions"]
    if len(actions) != 1 or "FunctionCall" not in actions[0]:
        raise ForcefieldError("X3_FORCEFIELD_CALL", "expected one function call")
    call = actions[0]["FunctionCall"]
    raw_args = call["args"]
    if isinstance(raw_args, str):
        args = json.loads(base64.b64decode(raw_args))
    else:
        args = raw_args
    return {
        "method_name": call["method_name"],
        "deposit": call["deposit"],
        "actions": len(actions),
        "purpose_fields": sorted(set(args) & set(PURPOSE_FIELDS)),
        "arg_names": sorted(args),
    }


def _policy(adapter: Any, leg: Mapping[str, Any], tokens: Sequence[Mapping[str, Any]], program_sha256: str) -> Dict[str, Any]:
    origin = adapter.resolve_asset(tokens, leg["origin"]["chain"], leg["origin"]["asset"])["assetId"]
    destination = adapter.resolve_asset(tokens, leg["destination"]["chain"], leg["destination"]["asset"])["assetId"]
    return adapter.policy_for(leg, {"originAsset": origin, "destinationAsset": destination}, program_sha256)


def _one_check(observation: Mapping[str, Any], name: str) -> Mapping[str, Any]:
    rows = [row for row in observation["checks"] if row.get("check") == name]
    if len(rows) != 1:
        raise ForcefieldError("X3_FORCEFIELD_EVIDENCE", f"{name} appears {len(rows)} times")
    return rows[0]


def _extension_line(body: str) -> Optional[Tuple[str, List[str]]]:
    if body.startswith("deny recipients"):
        return "deny_recipients", _name_list(body[len("deny recipients"):], "X3_FORCEFIELD_DENY")
    if body.startswith("allow programs"):
        values = _name_list(body[len("allow programs"):], "X3_FORCEFIELD_PROGRAM")
        if any(len(item) != 64 or any(character not in HEX for character in item) for item in values):
            raise ForcefieldError("X3_FORCEFIELD_PROGRAM", "allow programs lists 64 hex characters")
        return "allow_programs", values
    if body == "agents" or body.startswith("agents "):
        return "agents", _name_list(body[len("agents"):], "X3_FORCEFIELD_AGENTS")
    return None


def _name_list(rest: str, code: str) -> List[str]:
    text = rest.strip()
    if not text:
        raise ForcefieldError(code, "the clause names nobody")
    names = [item.strip() for item in text.split(",")]
    if any(not name or any(character.isspace() for character in name) for name in names):
        raise ForcefieldError(code, "each name is one token")
    if len(set(names)) != len(names):
        raise ForcefieldError(code, "names repeat")
    return names


def _clauses(source: str) -> Dict[str, List[str]]:
    found: Dict[str, List[str]] = {}
    for line in source.splitlines():
        text = line.strip()
        if not text or text.startswith("mandate ") or text == "}":
            continue
        parts = text.split()
        if parts[0] in ("allow", "deny", "require") and len(parts) >= 2:
            key = f"{parts[0]} {parts[1]}"
        elif parts[0] == "approve":
            key = "approve"
        else:
            key = parts[0]
        found.setdefault(key, []).append(text)
    return found


def _hex64(value: str, field: str) -> None:
    if len(value) != 64 or any(character not in HEX for character in value):
        raise ForcefieldError("X3_FORCEFIELD_CHAIN", f"{field} must be 64 hex characters")


def _maybe_int(value: Any) -> Optional[int]:
    if value is None:
        return None
    return int(value)


def _near() -> Any:
    import near_intents
    return near_intents
