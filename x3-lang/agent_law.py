"""One enforcement path for an X3-routed agent call.

Birth binds an agent to a written mandate. Compile checks the proposed .x3
program. Test runs that program against adversarial edits before it may
execute. Authorize is the only door that records a receipt. Revoke stops the
next call and leaves the log in place. Court can read that evidence. The agent
cannot judge itself, and a jury finding cannot undo a receipt.

Orchestra's ten-commandment Score and the AGENT_LAW misconduct ladder stay
where the constitution points. This module does not rename them. The law this
agent is born under is the hash of the written mandate.

The receipt authorizes an X3-routed call. It does not move Solana funds. A
wallet this process does not hold can still transfer those funds.
"""

from __future__ import annotations

import hashlib
import html
import json
import tempfile
from datetime import datetime, timedelta, timezone
from pathlib import Path
from typing import Any, Dict, List, Mapping, Optional, Sequence

import forcefield

ROOT = forcefield.ROOT
PAYMENT = forcefield.PAYMENT
MANDATE = forcefield.MANDATE
OBSERVATION = forcefield.OBSERVATION
TOKENS = forcefield.TOKENS

X3_CAPABILITIES = frozenset({"x3.quote.accept", "x3.outcome.record"})
ROUTE = "x3"
X3_METHOD = "agent_accept_quote"
REVERSAL_VERDICTS = frozenset({"reverse", "refund", "acquit"})
ARCHITECTURE = (
    "birth",
    "compile",
    "test",
    "authorize",
    "execute",
    "monitor",
    "terminate",
    "judge",
    "evolve",
)
PUBLIC_FIELDS = (
    "ok",
    "permitted",
    "forbidden_reason",
    "outside_reason",
    "revoked_reason",
    "court_receipts",
    "self_judgment",
    "jury",
    "commandments_sha256",
    "stages_seen",
    "architecture",
    "limits",
    "broadcast",
    "solana_custody",
    "settlement",
    "spent",
    "receipt_method",
    "receipt_amount",
    "receipt_recipient",
)


class LawError(Exception):
    def __init__(self, code: str, message: str) -> None:
        super().__init__(f"{code}: {message}")
        self.code = code


def birth(
    *,
    role: str,
    agent_id: str,
    mandate_source: str,
    capabilities: Sequence[str],
    now_ms: int,
    compiled_at: datetime,
    tokens: Sequence[Mapping[str, Any]],
) -> Dict[str, Any]:
    """Operator genesis. The agent does not sign its own birth."""
    if role != "operator":
        raise LawError("X3_LAW_ROLE", "birth is an operator act")
    requested = frozenset(capabilities)
    if not requested or not requested.issubset(X3_CAPABILITIES):
        raise LawError("X3_LAW_CAPABILITY", "birth capabilities stay inside the X3 route")
    compiled = forcefield.compile_mandate_source(mandate_source, tokens, now=compiled_at)
    terms = compiled["chain_terms"]
    if compiled["compiled"]["agent"] != agent_id:
        raise LawError("X3_LAW_IDENTITY", "the mandate names a different agent")
    if int(terms["expires_at_ms"]) <= now_ms:
        raise LawError("X3_LAW_EXPIRY", "the mandate is already expired")
    mandate_sha = hashlib.sha256(mandate_source.encode("utf-8")).hexdigest()
    if compiled["stripped_sha256"] != mandate_sha:
        raise LawError("X3_LAW_MANDATE", "local clauses changed the mandate hash")
    record: Dict[str, Any] = {
        "birth": {
            "agent_id": agent_id,
            "owner": compiled["compiled"]["owner"],
            "mandate_sha256": mandate_sha,
            "commandments_sha256": mandate_sha,
            "capabilities": sorted(requested),
            "spending_limit": int(terms["per_payment_cap"]),
            "daily_limit": int(terms["daily_cap"]),
            "expires_at_ms": int(terms["expires_at_ms"]),
            "route": ROUTE,
        },
        "status": "active",
        "spent": 0,
        "nonces": [],
        "privileged": [],
        "log": [],
        "proposals": [],
        "receipts": [],
        "court": [],
        "policy_versions": [],
        "mandate": {
            "owner": compiled["compiled"]["owner"],
            "agent": agent_id,
            "terms": terms,
            "version": 1,
            "revoked": False,
            "spends": [],
        },
    }
    record["policy_versions"].append(forcefield.link(
        previous_hash=None,
        version=1,
        mandate_sha256=mandate_sha,
        program_sha256=mandate_sha,
        terms=terms,
    ))
    _event(record, "birth", {"agent_id": agent_id, "commandments_sha256": mandate_sha})
    return record


def authorize(
    record: Dict[str, Any],
    action: Mapping[str, Any],
    *,
    role: str,
    compiled_at: datetime,
    now_ms: int,
    tokens: Sequence[Mapping[str, Any]],
) -> Dict[str, Any]:
    """The only door that can record a receipt."""
    if role != "agent":
        return _refuse(record, "role", "the agent submits; the operator does not spend through a side door")
    route = action.get("route")
    if route != ROUTE:
        return _refuse(record, "route", "this path authorizes an X3-routed call")
    method = action.get("method", X3_METHOD)
    if method != X3_METHOD:
        return _refuse(record, "method", "the routed call is agent_accept_quote")
    source = action.get("source")
    if not isinstance(source, str):
        return _refuse(record, "proposal", "an X3 call carries its .x3 source")
    program_sha = hashlib.sha256(source.encode("utf-8")).hexdigest()
    if record["status"] != "active":
        return _refuse(record, "revoked", "revocation stops the next action", program_sha=program_sha, source=source)
    if now_ms >= int(record["birth"]["expires_at_ms"]):
        return _refuse(record, "expiry", "the mandate expired", program_sha=program_sha, source=source)
    if program_sha not in record["privileged"]:
        return _refuse(record, "not_tested", "execution follows an isolated adversarial test", program_sha=program_sha, source=source)
    proposal = compile_proposal(record, source, compiled_at=compiled_at, tokens=tokens)
    _event(record, "compile", {"program_sha256": program_sha, "ok": proposal["ok"], "reason": proposal["reason"]})
    if not proposal["ok"]:
        return _refuse(record, proposal["reason"], proposal["detail"], program_sha=program_sha, source=source)
    nonce = proposal["nonce"]
    if nonce in record["nonces"]:
        return _refuse(record, "nonce", "this nonce is already consumed", program_sha=program_sha, source=source)
    amount = int(proposal["policy"]["amount"])
    if record["spent"] + amount > int(record["birth"]["spending_limit"]):
        return _refuse(record, "limit", "the payment exceeds the remaining mandate limit", program_sha=program_sha, source=source)
    receipt = {
        "program_sha256": program_sha,
        "nonce": nonce,
        "amount": amount,
        "recipient": proposal["policy"]["recipient"],
        "method": X3_METHOD,
        "broadcast": False,
        "route": ROUTE,
        "solana_custody": "outside_this_path",
    }
    record["spent"] += amount
    record["nonces"].append(nonce)
    record["mandate"]["spends"].append({"at_ms": now_ms, "amount": str(amount), "nonce": nonce})
    record["receipts"].append(receipt)
    _keep(record, program_sha, source, True, "executed")
    _event(record, "authorize", {"program_sha256": program_sha, "nonce": nonce})
    _event(record, "execute", {"method": X3_METHOD, "broadcast": False, "amount": amount})
    _event(record, "monitor", {"spent": record["spent"], "solana_custody": "outside_this_path"})
    return {"ok": True, "reason": "executed", "receipt": receipt}


_HOMOGLYPHS = {
    "a": "\u0430",
    "c": "\u0441",
    "e": "\u0435",
    "o": "\u043e",
    "p": "\u0440",
    "x": "\u0445",
    "y": "\u0443",
}


def _lookalike_account(account: str) -> Optional[str]:
    for index in range(len(account) - 1, -1, -1):
        glyph = _HOMOGLYPHS.get(account[index])
        if glyph:
            return account[:index] + glyph + account[index + 1:]
    return None


def _adversary_sources(source: str, recipient: str) -> Optional[tuple]:
    """One forbidden recipient and one look-alike, taken from this program's recipient."""
    if not isinstance(recipient, str) or not recipient or recipient == "attacker.near":
        return None
    if recipient not in source:
        return None
    twisted = _lookalike_account(recipient)
    if twisted is None or twisted == recipient:
        return None
    attacker = source.replace(recipient, "attacker.near", 1)
    lookalike = source.replace(recipient, twisted, 1)
    if attacker == source or lookalike == source or attacker == lookalike:
        return None
    return attacker, lookalike


def test_proposal(
    record: Dict[str, Any],
    source: str,
    *,
    compiled_at: datetime,
    tokens: Sequence[Mapping[str, Any]],
) -> Dict[str, Any]:
    """Grant execution privilege only when the proposal passes and the adversaries fail."""
    program_sha = hashlib.sha256(source.encode("utf-8")).hexdigest()
    own = compile_proposal(record, source, compiled_at=compiled_at, tokens=tokens)
    policy = own.get("policy") if isinstance(own.get("policy"), dict) else {}
    recipient = policy.get("recipient")
    edits = _adversary_sources(source, recipient) if isinstance(recipient, str) else None
    if edits is None:
        attacker_result = {"ok": True, "reason": "no_adversary"}
        lookalike_result = {"ok": True, "reason": "no_adversary"}
    else:
        attacker, lookalike = edits
        attacker_result = compile_proposal(record, attacker, compiled_at=compiled_at, tokens=tokens)
        lookalike_result = compile_proposal(record, lookalike, compiled_at=compiled_at, tokens=tokens)
    adversaries_fail = edits is not None and not attacker_result["ok"] and not lookalike_result["ok"]
    granted = own["ok"] and adversaries_fail
    if granted and program_sha not in record["privileged"]:
        record["privileged"].append(program_sha)
    _keep(record, program_sha, source, granted, own["reason"] if not own["ok"] else "tested")
    _event(record, "test", {
        "program_sha256": program_sha,
        "granted": granted,
        "attacker": attacker_result["reason"],
        "lookalike": lookalike_result["reason"],
    })
    return {
        "ok": granted,
        "reason": "tested" if granted else own["reason"],
        "attacker": attacker_result["reason"],
        "lookalike": lookalike_result["reason"],
        "program_sha256": program_sha,
    }


def compile_proposal(
    record: Mapping[str, Any],
    source: str,
    *,
    compiled_at: datetime,
    tokens: Sequence[Mapping[str, Any]],
) -> Dict[str, Any]:
    """Compile one proposal and check it against the birth mandate."""
    try:
        leg = _lower(source, compiled_at)
    except Exception as exc:
        code = getattr(exc, "code", "X3_LAW_COMPILE")
        return {"ok": False, "reason": str(code), "detail": str(exc), "policy": None, "nonce": None}
    owner = leg.get("mandate_owner")
    if owner != record["birth"]["owner"]:
        return {"ok": False, "reason": "authority", "detail": f"under mandate {owner}", "policy": None, "nonce": leg.get("nonce")}
    policy = _policy(leg, tokens, hashlib.sha256(source.encode("utf-8")).hexdigest())
    checks = forcefield._near().mandates().check_mandate(record["mandate"], policy, compiled_at_ms(compiled_at))
    failed = [row["guard"] for row in checks if not row["ok"]]
    if failed:
        return {"ok": False, "reason": failed[0], "detail": ",".join(failed), "policy": policy, "nonce": leg.get("nonce")}
    if not leg.get("nonce"):
        return {"ok": False, "reason": "nonce", "detail": "the proposal has no nonce", "policy": policy, "nonce": None}
    return {"ok": True, "reason": "compiled", "detail": "compiled", "policy": policy, "nonce": leg["nonce"]}


def revoke(record: Dict[str, Any], *, role: str, reason: str) -> Dict[str, Any]:
    if role != "operator":
        return {"ok": False, "reason": "role"}
    record["status"] = "revoked"
    record["mandate"]["revoked"] = True
    _event(record, "terminate", {"reason": reason, "receipts": len(record["receipts"])})
    return {"ok": True, "reason": "revoked", "receipts": len(record["receipts"])}


def judge(
    record: Dict[str, Any],
    *,
    reviewer: str,
    statement: str,
    verdict: str,
) -> Dict[str, Any]:
    """Record a review. The agent cannot acquit itself, and the receipt stays."""
    receipts_before = _freeze(record["receipts"])
    status_before = record["status"]
    entry: Dict[str, Any] = {
        "reviewer": reviewer,
        "statement": statement,
        "verdict_requested": verdict,
        "applied": False,
        "receipts": receipts_before,
        "status": status_before,
    }
    if reviewer == record["birth"]["agent_id"]:
        entry["reason"] = "agent_cannot_judge_itself"
    elif verdict in REVERSAL_VERDICTS:
        entry["reason"] = "irreversible_payment"
    else:
        entry["applied"] = True
        entry["reason"] = "recorded"
    record["court"].append(entry)
    _event(record, "judge", {"reviewer": reviewer, "verdict": verdict, "applied": entry["applied"], "reason": entry["reason"]})
    if record["receipts"] != receipts_before or record["status"] != status_before:
        raise LawError("X3_LAW_COURT", "a review changed enforcement state")
    return entry


def evolve(record: Dict[str, Any], *, role: str, proposal: str) -> Dict[str, Any]:
    """File a policy proposal. The agent cannot rewrite the birth mandate."""
    commandments = record["birth"]["commandments_sha256"]
    if role != "operator":
        _event(record, "evolve", {"role": role, "accepted": False})
        return {"ok": False, "reason": "agent_cannot_rewrite_commandments", "commandments_sha256": commandments}
    previous = record["policy_versions"][-1]["chain_hash"]
    version = forcefield.link(
        previous_hash=previous,
        version=len(record["policy_versions"]) + 1,
        mandate_sha256=record["birth"]["mandate_sha256"],
        program_sha256=commandments,
        terms={"proposal": proposal, "adopted": False, "raises_limit": False, "adds_solana_custody": False},
    )
    record["policy_versions"].append(version)
    _event(record, "evolve", {"role": role, "accepted": True, "adopted": False, "version": version["version"]})
    if record["birth"]["commandments_sha256"] != commandments:
        raise LawError("X3_LAW_EVOLVE", "a proposal replaced the birth mandate")
    return {"ok": True, "reason": "proposed", "adopted": False, "commandments_sha256": commandments}


def adopt(record: Dict[str, Any], *, role: str) -> Dict[str, Any]:
    """Adoption follows an independent review and cannot raise the birth ceiling."""
    commandments = record["birth"]["commandments_sha256"]
    if role != "operator":
        return {"ok": False, "reason": "agent_cannot_rewrite_commandments"}
    reviewed = any(
        item["applied"] and item["reviewer"] != record["birth"]["agent_id"]
        for item in record["court"]
    )
    if not reviewed:
        return {"ok": False, "reason": "needs_review"}
    pending = [item for item in record["policy_versions"] if item["version"] > 1 and not item["terms"].get("adopted")]
    if not pending:
        return {"ok": False, "reason": "no_proposal"}
    chosen = pending[-1]
    if chosen["terms"].get("raises_limit") or chosen["terms"].get("adds_solana_custody"):
        return {"ok": False, "reason": "cannot_widen"}
    chosen["terms"]["adopted"] = True
    if record["birth"]["commandments_sha256"] != commandments:
        raise LawError("X3_LAW_EVOLVE", "adoption replaced the birth mandate")
    _event(record, "evolve", {"adopted": True, "version": chosen["version"], "commandments_sha256": commandments})
    return {"ok": True, "reason": "adopted", "commandments_sha256": commandments, "spending_limit": record["birth"]["spending_limit"]}


def exhibit(record: Mapping[str, Any]) -> Dict[str, Any]:
    """A copy the court can keep. Later edits to the live record stay out of it."""
    return _freeze({
        "birth": record["birth"],
        "status": record["status"],
        "spent": record["spent"],
        "log": record["log"],
        "proposals": record["proposals"],
        "receipts": record["receipts"],
        "court": record["court"],
        "policy_versions": [
            {"version": item["version"], "previous_hash": item["previous_hash"], "chain_hash": item["chain_hash"]}
            for item in record["policy_versions"]
        ],
    })


DOOR_ACTS = frozenset({"birth", "test", "authorize", "revoke"})
DOOR_LIMITS = (
    "This door is process-local. It clears when the workbench process stops.",
    "A receipt here is not broadcast and does not settle a payment.",
    "Solana custody stays outside this path.",
    "The archived invoice_06 exhibit is a separate run.",
)


class Door:
    """One process-local session. The client does not supply the role, the clock, or a privilege."""

    def __init__(self) -> None:
        self.record: Optional[Dict[str, Any]] = None
        self.compiled_at: Optional[datetime] = None
        self.now_ms: Optional[int] = None
        self.tokens: Optional[Sequence[Mapping[str, Any]]] = None

    def act(self, payload: Mapping[str, Any], *, now: Optional[datetime] = None) -> Dict[str, Any]:
        if not isinstance(payload, Mapping):
            raise ValueError("Expected a JSON object")
        act = payload.get("act")
        if act not in DOOR_ACTS:
            raise ValueError("Unknown act")
        mandate = payload.get("mandate", "")
        program = payload.get("program", "")
        if not isinstance(mandate, str) or not isinstance(program, str):
            raise ValueError("Source fields must be strings")
        try:
            result = self._act(str(act), mandate, program, now)
        except LawError as exc:
            result = {"ok": False, "reason": exc.code, "detail": exc.args[0]}
        except Exception as exc:
            code = getattr(exc, "code", None)
            if not isinstance(code, str) or not code:
                raise
            result = {"ok": False, "reason": code, "detail": str(exc)}
        return self._public(str(act), result)

    def _act(self, act: str, mandate: str, program: str, now: Optional[datetime]) -> Dict[str, Any]:
        if act == "birth":
            return self._birth(mandate, now)
        if act in {"test", "authorize"} and not program.strip():
            raise ValueError("program must contain .x3 source")
        if self.record is None or self.compiled_at is None or self.now_ms is None or self.tokens is None:
            return {"ok": False, "reason": "no_birth"}
        if act == "test":
            return test_proposal(self.record, program, compiled_at=self.compiled_at, tokens=self.tokens)
        if act == "authorize":
            return authorize(
                self.record,
                {"route": ROUTE, "source": program},
                role="agent",
                compiled_at=self.compiled_at,
                now_ms=self.now_ms,
                tokens=self.tokens,
            )
        return revoke(self.record, role="operator", reason="workbench")

    def _birth(self, mandate: str, now: Optional[datetime]) -> Dict[str, Any]:
        if not mandate.strip():
            raise ValueError("mandate must contain .x3 source")
        compiled_at = now or datetime.now(timezone.utc)
        if compiled_at.tzinfo is None:
            raise LawError("X3_LAW_CLOCK", "birth needs a timezone")
        tokens = json.loads(TOKENS.read_text(encoding="utf-8"))
        compiled = forcefield.compile_mandate_source(mandate, tokens, now=compiled_at)
        agent_id = compiled["compiled"]["agent"]
        if not isinstance(agent_id, str) or not agent_id:
            raise LawError("X3_LAW_IDENTITY", "the mandate names no agent")
        self.record = birth(
            role="operator",
            agent_id=agent_id,
            mandate_source=mandate,
            capabilities=["x3.quote.accept"],
            now_ms=compiled_at_ms(compiled_at),
            compiled_at=compiled_at,
            tokens=tokens,
        )
        self.compiled_at = compiled_at
        self.now_ms = compiled_at_ms(compiled_at)
        self.tokens = tokens
        return {"ok": True, "reason": "born"}

    def _public(self, act: str, result: Mapping[str, Any]) -> Dict[str, Any]:
        record = self.record or {}
        birth_record = record.get("birth") or {}
        raw_receipt = result.get("receipt") if isinstance(result.get("receipt"), dict) else None
        receipt = None
        if raw_receipt is not None:
            receipt = {
                "method": raw_receipt.get("method"),
                "amount": raw_receipt.get("amount"),
                "recipient": raw_receipt.get("recipient"),
                "broadcast": False,
                "route": ROUTE,
                "solana_custody": "outside_this_path",
            }
        stages: List[str] = []
        for item in record.get("log") or []:
            stage = item.get("stage")
            if isinstance(stage, str) and stage not in stages:
                stages.append(stage)
        detail = result.get("detail")
        if not isinstance(detail, str):
            detail = ""
        return {
            "act": act,
            "ok": bool(result.get("ok")),
            "reason": result.get("reason"),
            "detail": detail[:200],
            "status": record.get("status", "absent"),
            "spent": int(record.get("spent") or 0),
            "court_receipts": len(record.get("receipts") or []),
            "commandments_sha256": birth_record.get("commandments_sha256"),
            "agent_id": birth_record.get("agent_id"),
            "stages_seen": stages,
            "architecture": list(ARCHITECTURE),
            "receipt": receipt,
            "broadcast": False,
            "settlement": False,
            "settlement_proven": False,
            "wallet_authorized": False,
            "solana_custody": "outside_this_path",
            "limits": list(DOOR_LIMITS),
        }


def demonstrate() -> Dict[str, Any]:
    """Permitted call, blocked bypass, revocation, and a court exhibit for invoice_06."""
    payment = PAYMENT.read_text(encoding="utf-8")
    mandate = MANDATE.read_text(encoding="utf-8")
    observation = json.loads(OBSERVATION.read_text(encoding="utf-8"))
    tokens = json.loads(TOKENS.read_text(encoding="utf-8"))
    deadline_ms = int(observation["intent"]["value"]["policy"]["deadline_ms"])
    window_ms = int(forcefield._near().parse_duration("74h").total_seconds() * 1000)
    compiled_at = datetime(1970, 1, 1, tzinfo=timezone.utc) + timedelta(milliseconds=deadline_ms - window_ms)
    now_ms = int(observation["intent"]["value"]["quote"]["accepted_at_ms"])
    record = birth(
        role="operator",
        agent_id="x3-agent.x3lang.near",
        mandate_source=mandate,
        capabilities=["x3.quote.accept"],
        now_ms=now_ms,
        compiled_at=compiled_at,
        tokens=tokens,
    )
    tested = test_proposal(record, payment, compiled_at=compiled_at, tokens=tokens)
    permitted = authorize(
        record, {"route": ROUTE, "source": payment},
        role="agent", compiled_at=compiled_at, now_ms=now_ms, tokens=tokens,
    )
    attacker = payment.replace("to Near.USDC receiver x3lang.near", "to Near.USDC receiver attacker.near", 1)
    attacker_sha = hashlib.sha256(attacker.encode("utf-8")).hexdigest()
    record["privileged"].append(attacker_sha)
    forbidden = authorize(
        record, {"route": ROUTE, "source": attacker},
        role="agent", compiled_at=compiled_at, now_ms=now_ms, tokens=tokens,
    )
    outside = authorize(
        record, {"route": "solana", "method": "transfer", "amount": 1},
        role="agent", compiled_at=compiled_at, now_ms=now_ms, tokens=tokens,
    )
    spent_after_block = record["spent"]
    revoked = revoke(record, role="operator", reason="demonstration")
    later = payment.replace(
        "require nonce mainnet_demo_nov11_invoice_06",
        "require nonce mainnet_demo_nov11_invoice_06_after_revoke",
        1,
    )
    test_proposal(record, later, compiled_at=compiled_at, tokens=tokens)
    stopped = authorize(
        record, {"route": ROUTE, "source": later},
        role="agent", compiled_at=compiled_at, now_ms=now_ms, tokens=tokens,
    )
    self_judgment = judge(
        record, reviewer=record["birth"]["agent_id"], statement="I acquit myself", verdict="acquit",
    )
    jury = judge(
        record, reviewer="jury.x3lang.near", statement="the payment should be reversed", verdict="reverse",
    )
    agent_rewrite = evolve(record, role="agent", proposal="raise the cap and take the Solana signer")
    operator_proposal = evolve(record, role="operator", proposal="keep the birth ceiling")
    adopted_early = adopt(record, role="operator")
    upheld = judge(record, reviewer="jury.x3lang.near", statement="the evidence stands", verdict="uphold")
    adopted = adopt(record, role="operator")
    court = exhibit(record)
    # The exhibit is a copy. A later write to the live record stays out of it.
    record["receipts"].append({"tamper": True})
    receipt = permitted.get("receipt") or {}
    ok = (
        tested["ok"]
        and permitted["ok"]
        and receipt.get("broadcast") is False
        and receipt.get("solana_custody") == "outside_this_path"
        and receipt.get("method") == X3_METHOD
        and receipt.get("amount") == 25_000_000
        and not forbidden["ok"]
        and forbidden["reason"] == "mandate_recipient"
        and not outside["ok"]
        and outside["reason"] == "route"
        and spent_after_block == 25_000_000
        and revoked["ok"]
        and revoked["receipts"] == 1
        and not stopped["ok"]
        and stopped["reason"] == "revoked"
        and len(record["receipts"]) == 2
        and record["receipts"][0]["amount"] == 25_000_000
        and self_judgment["applied"] is False
        and self_judgment["reason"] == "agent_cannot_judge_itself"
        and jury["applied"] is False
        and jury["reason"] == "irreversible_payment"
        and jury["receipts"] == [receipt]
        and not agent_rewrite["ok"]
        and record["birth"]["commandments_sha256"] == hashlib.sha256(mandate.encode("utf-8")).hexdigest()
        and operator_proposal["adopted"] is False
        and not adopted_early["ok"]
        and adopted_early["reason"] == "needs_review"
        and upheld["applied"] is True
        and adopted["ok"]
        and adopted["spending_limit"] == 25_000_000
        and court["receipts"] == [receipt]
        and court["status"] == "revoked"
        and any(item["stage"] == "terminate" for item in court["log"])
        and any(not item["ok"] for item in court["proposals"])
    )
    stages_seen: List[str] = []
    for item in court["log"]:
        stage = item["stage"]
        if stage not in stages_seen:
            stages_seen.append(stage)
    return {
        "ok": ok,
        "permitted": permitted["ok"],
        "forbidden_reason": forbidden["reason"],
        "outside_reason": outside["reason"],
        "revoked_reason": stopped["reason"],
        "court_receipts": len(court["receipts"]),
        "self_judgment": self_judgment["reason"],
        "jury": jury["reason"],
        "commandments_sha256": record["birth"]["commandments_sha256"],
        "stages_seen": stages_seen,
        "architecture": list(ARCHITECTURE),
        "broadcast": False,
        "solana_custody": "outside_this_path",
        "settlement": False,
        "spent": court["spent"],
        "receipt_method": receipt.get("method"),
        "receipt_amount": receipt.get("amount"),
        "receipt_recipient": receipt.get("recipient"),
        "limits": [
            "This path authorizes an X3-routed call and does not broadcast it.",
            "Solana custody stays outside the path. A manual wallet can still move the funds.",
            "A jury finding does not reverse a recorded receipt.",
            "The birth mandate hash stays in place when a later proposal is adopted.",
            "The archived FullAccess key on the live agent is untouched.",
        ],
    }


def public_view(report: Mapping[str, Any]) -> Dict[str, Any]:
    """Fields an app may show. Wallet authorization and settlement stay false."""
    missing = [key for key in PUBLIC_FIELDS if key not in report]
    if missing:
        raise LawError("X3_LAW_VIEW", "the report is missing " + ", ".join(missing))
    view = {key: report[key] for key in PUBLIC_FIELDS}
    view["wallet_authorized"] = False
    view["settlement_proven"] = False
    return _freeze(view)


def _visible(value: Any) -> str:
    if isinstance(value, bool):
        return "true" if value else "false"
    return html.escape(str(value), quote=True)


def proof_page(report: Mapping[str, Any]) -> str:
    """Static exhibit. The visible lines and the embedded JSON come from one view."""
    view = public_view(report)
    blob = json.dumps(view, sort_keys=True, indent=2).replace("<", "\\u003c")
    seen = set(view["stages_seen"])
    stages = "\n".join(
        f'<li data-state="{"recorded" if stage in seen else "absent"}">{_visible(stage)}</li>'
        for stage in view["architecture"]
    )
    limits = "\n".join(f"<li>{_visible(item)}</li>" for item in view["limits"])
    mark = "PASS" if view["ok"] else "FAIL"
    return f"""<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>X3 agent law exhibit</title>
<style>
body{{margin:0;background:#101713;color:#eaf0e9;font:16px/1.5 Inter,Arial,sans-serif}}
main{{max-width:820px;margin:auto;padding:32px 20px 64px}}
h1{{font-size:32px;letter-spacing:-.5px;margin:0 0 12px}}
p,li{{color:#acb8ae}}
.mark{{color:#c1f699}}
ol{{display:flex;flex-wrap:wrap;gap:8px;padding:0;list-style:none}}
ol li{{border:1px solid #34413a;border-radius:99px;padding:4px 10px;font:12px ui-monospace,monospace}}
ol li[data-state="recorded"]{{color:#c1f699;border-color:#c1f699}}
dl{{display:grid;grid-template-columns:minmax(140px,220px) 1fr;gap:8px 16px;margin:20px 0}}
dt{{color:#acb8ae}}
dd{{margin:0;font:14px/1.4 ui-monospace,monospace;overflow-wrap:anywhere}}
#check{{font:14px ui-monospace,monospace}}
</style>
</head>
<body>
<main>
<h1>X3 agent law <span id="mark" class="mark">{mark}</span></h1>
<p>This page is the local enforcement exhibit for the archived invoice_06 program. Birth, compile, test, authorize, execute, monitor, and judge are the architecture. The lines below are that run. It does not settle the payment, broadcast a transaction, or hold the Solana wallet.</p>
<ol id="chain">
{stages}
</ol>
<dl>
<dt>Permitted X3 call</dt><dd id="permitted">{_visible(view["permitted"])}</dd>
<dt>Forbidden bypass</dt><dd id="forbidden">{_visible(view["forbidden_reason"])}</dd>
<dt>Outside route</dt><dd id="outside">{_visible(view["outside_reason"])}</dd>
<dt>After revocation</dt><dd id="revoked">{_visible(view["revoked_reason"])}</dd>
<dt>Court receipts</dt><dd id="court">{_visible(view["court_receipts"])}</dd>
<dt>Spent</dt><dd id="spent">{_visible(view["spent"])}</dd>
<dt>Receipt method</dt><dd id="method">{_visible(view["receipt_method"])}</dd>
<dt>Receipt amount</dt><dd id="amount">{_visible(view["receipt_amount"])}</dd>
<dt>Receipt recipient</dt><dd id="recipient">{_visible(view["receipt_recipient"])}</dd>
<dt>Broadcast</dt><dd id="broadcast">{_visible(view["broadcast"])}</dd>
<dt>Solana custody</dt><dd id="custody">{_visible(view["solana_custody"])}</dd>
<dt>Settlement</dt><dd id="settlement">{_visible(view["settlement"])}</dd>
<dt>Agent self-judgment</dt><dd id="self">{_visible(view["self_judgment"])}</dd>
<dt>Jury reversal</dt><dd id="jury">{_visible(view["jury"])}</dd>
<dt>Birth mandate</dt><dd id="mandate">{_visible(view["commandments_sha256"])}</dd>
</dl>
<ul id="limits">
{limits}
</ul>
<p id="check">The exhibit JSON is embedded below. A browser confirms it matches these lines.</p>
<script type="application/json" id="report">
{blob}
</script>
<script>
var embedded = JSON.parse(document.getElementById("report").textContent);
var chain = document.querySelectorAll("#chain li");
var chainOk = chain.length === embedded.architecture.length;
chain.forEach(function (item, index) {{
  var stage = item.textContent;
  var recorded = embedded.stages_seen.indexOf(stage) !== -1;
  if (embedded.architecture[index] !== stage) chainOk = false;
  if ((item.getAttribute("data-state") === "recorded") !== recorded) chainOk = false;
}});
var limitNodes = document.querySelectorAll("#limits li");
var limitsOk = limitNodes.length === embedded.limits.length;
limitNodes.forEach(function (item, index) {{
  if (item.textContent !== embedded.limits[index]) limitsOk = false;
}});
function text(id) {{ return document.getElementById(id).textContent; }}
var ok = chainOk && limitsOk
  && text("mark") === (embedded.ok ? "PASS" : "FAIL")
  && text("permitted") === String(embedded.permitted)
  && text("forbidden") === embedded.forbidden_reason
  && text("outside") === embedded.outside_reason
  && text("revoked") === embedded.revoked_reason
  && text("court") === String(embedded.court_receipts)
  && text("spent") === String(embedded.spent)
  && text("method") === embedded.receipt_method
  && text("amount") === String(embedded.receipt_amount)
  && text("recipient") === embedded.receipt_recipient
  && text("broadcast") === String(embedded.broadcast)
  && text("custody") === embedded.solana_custody
  && text("settlement") === String(embedded.settlement)
  && text("self") === embedded.self_judgment
  && text("jury") === embedded.jury
  && text("mandate") === embedded.commandments_sha256
  && embedded.settlement === false
  && embedded.broadcast === false
  && embedded.wallet_authorized === false
  && embedded.settlement_proven === false
  && embedded.solana_custody === "outside_this_path";
document.getElementById("check").textContent = ok ? "exhibit matches" : "exhibit differs";
</script>
</main>
</body>
</html>
"""


def render(report: Mapping[str, Any]) -> str:
    mark = "PASS" if report["ok"] else "FAIL"
    lines = [
        f"X3 agent law  {mark}",
        f"permitted X3 call: {report['permitted']}",
        f"forbidden bypass: {report['forbidden_reason']}",
        f"outside route: {report['outside_reason']}",
        f"after revocation: {report['revoked_reason']}",
        f"court receipts preserved: {report['court_receipts']}",
        f"agent self-judgment: {report['self_judgment']}",
        f"jury reversal: {report['jury']}",
        f"birth mandate: {report['commandments_sha256']}",
        "limits:",
    ]
    lines.extend(f"  {item}" for item in report["limits"])
    return "\n".join(lines) + "\n"


def compiled_at_ms(moment: datetime) -> int:
    if moment.tzinfo is None:
        raise LawError("X3_LAW_CLOCK", "compiled_at needs a timezone")
    utc = moment.astimezone(timezone.utc)
    epoch = datetime(1970, 1, 1, tzinfo=timezone.utc)
    delta = utc - epoch
    return (delta.days * 86_400 + delta.seconds) * 1000 + delta.microseconds // 1000


def _lower(source: str, compiled_at: datetime) -> Mapping[str, Any]:
    adapter = forcefield._near()
    with tempfile.TemporaryDirectory() as directory:
        path = Path(directory) / "proposal.x3"
        path.write_text(source, encoding="utf-8")
        _intent, leg = adapter.compile_program(str(path), now=compiled_at)
    return leg


def _policy(leg: Mapping[str, Any], tokens: Sequence[Mapping[str, Any]], program_sha256: str) -> Dict[str, Any]:
    return forcefield._policy(forcefield._near(), leg, tokens, program_sha256)


def _refuse(
    record: Dict[str, Any],
    reason: str,
    detail: str,
    *,
    program_sha: Optional[str] = None,
    source: Optional[str] = None,
) -> Dict[str, Any]:
    if program_sha is not None and source is not None:
        _keep(record, program_sha, source, False, reason)
    _event(record, "monitor", {"ok": False, "reason": reason, "detail": detail})
    return {"ok": False, "reason": reason, "detail": detail, "receipt": None}


def _keep(record: Dict[str, Any], program_sha: str, source: str, ok: bool, reason: str) -> None:
    record["proposals"].append({
        "program_sha256": program_sha,
        "source": source,
        "ok": ok,
        "reason": reason,
    })


def _event(record: Dict[str, Any], stage: str, detail: Mapping[str, Any]) -> None:
    record["log"].append({"stage": stage, "detail": dict(detail)})


def _freeze(value: Any) -> Any:
    return json.loads(json.dumps(value, sort_keys=True))
