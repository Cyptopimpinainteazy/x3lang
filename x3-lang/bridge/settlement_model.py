"""Exhaustive explicit-state model of BSC <-> NEAR settlement rules (issue #42).

This is a finite MODEL, not a proof about deployed code. It enumerates every
interleaving of party actions, NEAR finality, NEAR reorgs of non-final blocks
and an adversarial scheduler (which may delay any transaction past any
deadline), over a small discrete clock that covers every timelock boundary.

For each BSC withdrawal rule it checks, with one party adversarial and the
other following the protocol, the property

    AG EF honest_party_whole

i.e. from EVERY reachable state, the honest party can still end up with either
its own asset back or the counterparty's asset, using only its own protocol
actions plus actions anyone can take (relaying, NEAR finality, time passing).
A reachable state that fails this is a "free option" for the adversary: it got
the honest party's asset or permanently stranded it. The checker returns the
shortest action trace that reaches such a state.

Rules modelled:
  LEGACY_HASHLOCK  BNB to LP on SHA-256 preimage before BSC expiry (13054df).
  OWNER_RELEASE    BNB to LP only when the depositor releases (03a33a9, master).
  NEAR_SETTLED     BNB moves only on a verified FINAL NEAR terminal outcome
                   (NearSettledBnbEscrow + issue #42 NEAR contract).
Mutants of NEAR_SETTLED (each must be caught):
  ACCEPT_PENDING          verifier accepts executed-but-not-final NEAR state
  ACCEPT_FAILED_RECEIPT   verifier accepts a failed claim receipt's log
  PAY_LP_ON_REFUND        escrow pays the LP on a NEAR Refunded outcome
  TIMEOUT_REFUND          escrow adds an evidence-free BSC timeout refund
  CLAIM_WITHOUT_PROOF     escrow settles Claimed without any verification
  LOCK_WITHOUT_REGISTRATION  escrow lets BNB lock without a final NEAR window
  REFUND_INITIATOR_ONLY   NEAR refund callable only by the LP (pre-#42)

Clock: 0 = start, D = NEAR funding deadline, E = NEAR expiry, B = legacy BSC
expiry, with 0 < D < E < B < TMAX. Each boundary is reachable exactly, one
tick before and one tick after.
"""
from __future__ import annotations

from collections import deque
from dataclasses import dataclass, replace
from typing import Callable, Iterable

D, E, B, TMAX = 2, 4, 6, 7

RULES = ("LEGACY_HASHLOCK", "OWNER_RELEASE", "NEAR_SETTLED")
MUTANTS = (
    "ACCEPT_PENDING",
    "ACCEPT_FAILED_RECEIPT",
    "PAY_LP_ON_REFUND",
    "TIMEOUT_REFUND",
    "CLAIM_WITHOUT_PROOF",
    "LOCK_WITHOUT_REGISTRATION",
    "REFUND_INITIATOR_ONLY",
)


@dataclass(frozen=True)
class S:
    t: int = 0
    window_exists: bool = True  # NEAR window registered and final
    head_window: str = "P"  # P pending / F funded / N not-funded  ("-" absent)
    head_swap: str = "-"  # - none / L locked / C claimed / R refunded
    final_window: str = "P"
    final_swap: str = "-"
    claim_pending: bool = False  # Alice's claim tx submitted, not yet executed
    secret_public: bool = False  # preimage visible (mempool or failed receipt)
    failed_claim_receipt: bool = False  # a FAILED claim receipt exists
    bsc: str = "-"  # - none / L locked / P paid to LP / A refunded to Alice


@dataclass(frozen=True)
class Action:
    name: str
    actor: str  # "alice", "lp", "any" (anyone/relayer), "sched" (scheduler/adversary)
    honest_alice: bool = True  # allowed in an honest Alice's protocol
    honest_lp: bool = True  # allowed in an honest LP's protocol


def _actions(rule: str, mutant: str | None) -> list[tuple[Action, Callable[[S], S | None]]]:
    acts: list[tuple[Action, Callable[[S], S | None]]] = []

    def add(action: Action):
        def deco(fn):
            acts.append((action, fn))
            return fn

        return deco

    # ---- neutral: time, NEAR finality, NEAR reorg of non-final state ------
    @add(Action("tick", "sched"))
    def _tick(s):
        return replace(s, t=s.t + 1) if s.t < TMAX else None

    @add(Action("near_finalize", "any"))
    def _fin(s):
        if (s.final_window, s.final_swap) == (s.head_window, s.head_swap):
            return None
        return replace(s, final_window=s.head_window, final_swap=s.head_swap)

    @add(Action("near_reorg_nonfinal", "sched"))
    def _reorg(s):
        if (s.final_window, s.final_swap) == (s.head_window, s.head_swap):
            return None
        return replace(s, head_window=s.final_window, head_swap=s.final_swap)

    # ---- BSC lock -------------------------------------------------------
    @add(Action("alice_lock_bnb", "alice"))
    def _lock(s):
        if s.bsc != "-" or s.t >= D:
            return None
        needs_registration = rule == "NEAR_SETTLED" and mutant != "LOCK_WITHOUT_REGISTRATION"
        if needs_registration and not s.window_exists:
            return None
        return replace(s, bsc="L")

    # ---- NEAR ------------------------------------------------------------
    @add(Action("lp_fund_near", "lp", honest_lp=True))
    def _fund(s):
        if not s.window_exists or s.head_window != "P" or s.t >= D:
            return None
        return replace(s, head_window="F", head_swap="L")

    @add(Action("near_close_unfunded", "any"))
    def _close(s):
        if not s.window_exists or s.head_window != "P" or s.t < D:
            return None
        return replace(s, head_window="N")

    @add(Action("alice_submit_near_claim", "alice"))
    def _submit(s):
        if s.head_swap != "L" or s.claim_pending:
            return None
        return replace(s, claim_pending=True, secret_public=True)

    @add(Action("near_execute_claim", "sched"))
    def _execute(s):
        if not s.claim_pending:
            return None
        if s.head_swap == "L" and s.t < E:
            return replace(s, claim_pending=False, head_swap="C")
        return replace(s, claim_pending=False, failed_claim_receipt=True)

    refund_actor = "lp" if mutant == "REFUND_INITIATOR_ONLY" else "any"

    @add(Action("near_refund", refund_actor))
    def _near_refund(s):
        if s.head_swap != "L" or s.t < E:
            return None
        return replace(s, head_swap="R")

    # ---- BSC withdrawal rule --------------------------------------------
    if rule == "LEGACY_HASHLOCK":

        @add(Action("lp_claim_bnb_with_preimage", "lp"))
        def _lc(s):
            return replace(s, bsc="P") if s.bsc == "L" and s.secret_public and s.t < B else None

        @add(Action("alice_timeout_refund_bnb", "alice"))
        def _lr(s):
            return replace(s, bsc="A") if s.bsc == "L" and s.t >= B else None

    elif rule == "OWNER_RELEASE":

        @add(Action("alice_owner_release", "alice", honest_alice=False))
        def _or_any(s):
            return replace(s, bsc="P") if s.bsc == "L" and s.t < B else None

        @add(Action("alice_owner_release_after_final_credit", "alice"))
        def _or_honest(s):
            return replace(s, bsc="P") if s.bsc == "L" and s.t < B and s.final_swap == "C" else None

        @add(Action("alice_timeout_refund_bnb", "alice"))
        def _orr(s):
            return replace(s, bsc="A") if s.bsc == "L" and s.t >= B else None

    else:  # NEAR_SETTLED and its mutants

        def proves(s: S, outcome: str) -> bool:
            if outcome == "CLAIMED":
                if mutant == "CLAIM_WITHOUT_PROOF":
                    return True
                if mutant == "ACCEPT_PENDING" and s.head_swap == "C":
                    return True
                if mutant == "ACCEPT_FAILED_RECEIPT" and s.failed_claim_receipt:
                    return True
                return s.final_swap == "C"
            if outcome == "REFUNDED":
                return s.final_swap == "R"
            if outcome == "NOT_FUNDED":
                return s.final_window == "N"
            raise AssertionError(outcome)

        for outcome in ("CLAIMED", "REFUNDED", "NOT_FUNDED"):

            def _settle(s, outcome=outcome):
                if s.bsc != "L" or not proves(s, outcome):
                    return None
                to_lp = outcome == "CLAIMED" or (mutant == "PAY_LP_ON_REFUND" and outcome == "REFUNDED")
                return replace(s, bsc="P" if to_lp else "A")

            acts.append((Action(f"relay_settle_{outcome.lower()}", "any"), _settle))

        if mutant == "TIMEOUT_REFUND":

            @add(Action("alice_timeout_refund_bnb", "alice"))
            def _tr(s):
                return replace(s, bsc="A") if s.bsc == "L" and s.t >= B else None

    return acts


def alice_whole(s: S) -> bool:
    """Honest Alice has NEAR (final credit), or her BNB back, or never locked."""
    return s.final_swap == "C" or s.bsc in ("A", "-")


def lp_whole(s: S) -> bool:
    """Honest LP has BNB, or its NEAR back (final), or never funded."""
    return s.bsc == "P" or (s.final_swap in ("R", "-") and s.head_swap in ("R", "-"))


@dataclass(frozen=True)
class Result:
    rule: str
    mutant: str | None
    honest: str
    states: int
    violations: int
    trace: tuple[str, ...]  # shortest trace to a violating state, () if none
    final_state: S | None

    @property
    def safe(self) -> bool:
        return self.violations == 0


def _initial_states(mutant: str | None) -> Iterable[S]:
    yield S()
    # A depositor may face a window that was never registered (or squatted).
    yield S(window_exists=False, head_window="-", final_window="-")


def check(rule: str, honest: str, mutant: str | None = None) -> Result:
    """Exhaustively check AG EF whole(honest) with the other party adversarial."""
    if rule not in RULES or honest not in ("alice", "lp"):
        raise ValueError("unknown rule or party")
    if mutant is not None and (rule != "NEAR_SETTLED" or mutant not in MUTANTS):
        raise ValueError("mutants apply to NEAR_SETTLED only")
    acts = _actions(rule, mutant)
    adversary = "lp" if honest == "alice" else "alice"

    def allowed_any(a: Action) -> bool:
        # The honest party only takes protocol actions; everyone else may take
        # any action the contracts allow.
        if a.actor == honest:
            return a.honest_alice if honest == "alice" else a.honest_lp
        return True

    def allowed_honest_path(a: Action) -> bool:
        # Honest party's protocol, anyone's permissionless actions and the
        # passage of time / NEAR finality. Never the adversary, never reorgs.
        if a.actor == adversary or a.name == "near_reorg_nonfinal":
            return False
        if a.actor == honest:
            return a.honest_alice if honest == "alice" else a.honest_lp
        return True

    def lp_protocol_guard(a: Action, s: S) -> bool:
        # An honest LP funds NEAR only after the BSC lock exists. An honest
        # Alice reveals her preimage only against FINAL NEAR funding, so the
        # legacy counterexamples below are not artefacts of a careless Alice.
        if honest == "lp" and a.name == "lp_fund_near":
            return s.bsc == "L"
        if honest == "alice" and a.name == "alice_submit_near_claim":
            return s.final_swap == "L" and s.bsc != "-" and s.t < E
        return True

    whole = alice_whole if honest == "alice" else lp_whole
    parent: dict[S, tuple[S | None, str]] = {}
    queue: deque[S] = deque()
    for s0 in _initial_states(mutant):
        parent[s0] = (None, "init")
        queue.append(s0)
    edges: dict[S, list[S]] = {}
    while queue:
        s = queue.popleft()
        succ_honest: list[S] = []
        for a, fn in acts:
            if not allowed_any(a) or not lp_protocol_guard(a, s):
                continue
            n = fn(s)
            if n is None:
                continue
            if allowed_honest_path(a):
                succ_honest.append(n)
            if n not in parent:
                parent[n] = (s, a.name)
                queue.append(n)
        edges[s] = succ_honest

    # Backward fixpoint: states from which `whole` is reachable on honest edges.
    good = {s for s in parent if whole(s)}
    changed = True
    while changed:
        changed = False
        for s, succ in edges.items():
            if s not in good and any(n in good for n in succ):
                good.add(s)
                changed = True
    bad = [s for s in parent if s not in good]
    trace: tuple[str, ...] = ()
    worst: S | None = None
    if bad:
        # BFS order of discovery gives shortest traces; pick the first bad state.
        order = {s: i for i, s in enumerate(parent)}
        worst = min(bad, key=lambda s: order[s])
        steps = []
        cur: S | None = worst
        while cur is not None:
            prev, name = parent[cur]
            if prev is not None:
                steps.append(name)
            cur = prev
        trace = tuple(reversed(steps))
    return Result(rule, mutant, honest, len(parent), len(bad), trace, worst)


def main() -> int:
    ok = True
    for rule in RULES:
        for honest in ("alice", "lp"):
            r = check(rule, honest)
            verdict = "SAFE" if r.safe else "FREE OPTION"
            print(f"{rule:16} honest={honest:5} states={r.states:5} {verdict}" +
                  ("" if r.safe else f"  trace={' -> '.join(r.trace)}"))
            if rule == "NEAR_SETTLED" and not r.safe:
                ok = False
    for mutant in MUTANTS:
        caught = [h for h in ("alice", "lp") if not check("NEAR_SETTLED", h, mutant).safe]
        print(f"mutant {mutant:26} {'KILLED by ' + ','.join(caught) if caught else 'SURVIVED'}")
        ok = ok and bool(caught)
    return 0 if ok else 1


if __name__ == "__main__":
    raise SystemExit(main())
