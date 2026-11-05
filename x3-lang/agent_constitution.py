"""Pure admission checks for X3 agent genesis.

This module only decides if an untrusted *candidate manifest* is admissible.
It does not authenticate an operator, mint identity, reserve budget, persist
state, revoke credentials or enforce execution. Those are trusted runtime
responsibilities. See docs/security/X3_AGENT_CONSTITUTION_V1.md.
"""
from dataclasses import dataclass
from typing import FrozenSet, Optional, Tuple


@dataclass(frozen=True)
class AgentManifest:
    agent_id: str
    parent_id: Optional[str]
    capabilities: FrozenSet[str]
    surfaces: FrozenSet[str]
    budget: int
    expires_at: int
    lineage: Tuple[str, ...]
    spawn_limit: int


@dataclass(frozen=True)
class AdmissionDecision:
    allowed: bool
    reasons: Tuple[str, ...]


def check_admission(
    candidate: AgentManifest,
    parent: Optional[AgentManifest],
    *,
    now: int,
    existing_ids: FrozenSet[str],
    parent_budget_available: int,
    parent_children: int,
    approval_verified: bool,
) -> AdmissionDecision:
    """Fail-closed deterministic evaluation of a proposed child agent.

    approval_verified MUST originate in a trusted signature/approval
    verifier, never from candidate-generated JSON. The caller must atomically
    reserve budget, bind the manifest to the signed approval and persist the
    identity before any runtime may execute it.
    """
    failures = []
    if not approval_verified:
        failures.append("approval")
    if not candidate.agent_id or candidate.agent_id in existing_ids:
        failures.append("identity")
    if candidate.budget <= 0 or candidate.spawn_limit < 0:
        failures.append("budget")
    if candidate.expires_at <= now:
        failures.append("expiry")
    if not candidate.capabilities or not candidate.surfaces:
        failures.append("capabilities_or_surfaces_empty")
    if parent is None:
        failures.append("root_requires_separate_genesis")
    else:
        if parent.parent_id == parent.agent_id or not parent.agent_id:
            failures.append("parent_invalid")
        if parent.expires_at <= now:
            failures.append("parent_expired")
        if candidate.parent_id != parent.agent_id or candidate.lineage != parent.lineage + (parent.agent_id,):
            failures.append("lineage")
        if not candidate.capabilities.issubset(parent.capabilities):
            failures.append("capabilities")
        if not candidate.surfaces.issubset(parent.surfaces):
            failures.append("surfaces")
        if candidate.budget > parent_budget_available or candidate.budget > parent.budget or parent_budget_available < 0:
            failures.append("budget")
        if candidate.expires_at > parent.expires_at:
            failures.append("expiry")
        if parent_children >= parent.spawn_limit or parent_children < 0 or candidate.spawn_limit > parent.spawn_limit:
            failures.append("spawn_limit")
    return AdmissionDecision(not failures, tuple(dict.fromkeys(failures)))
