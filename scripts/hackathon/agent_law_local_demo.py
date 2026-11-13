#!/usr/bin/env python3
"""Local signed-agent lifecycle proof. No RPC, wallet, deployment or payment."""
import argparse
from datetime import datetime, timezone
import json
from pathlib import Path
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "x3-lang"))
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from cryptography.hazmat.primitives import serialization
from agent_constitution import AgentManifest
from agent_admission_signed import SignedApproval, approval_payload
from agent_runtime import Runtime, Adapter, Action, JuryVote, canonical, digest, vote_payload, PolicyViolation
from agent_runtime_intents import IntentCompilerPolicy
from agent_runtime_court import IncidentCourt, Finding, finding_payload, incident_packet


def raw(key):
    return key.public_key().public_bytes(serialization.Encoding.Raw, serialization.PublicFormat.Raw)


def demonstrate():
    now = int(datetime(2026, 11, 13, tzinfo=timezone.utc).timestamp())
    operator, parent_key, child_key = (Ed25519PrivateKey.generate() for _ in range(3))
    jurors = {name: Ed25519PrivateKey.generate() for name in ("jury-a", "jury-b", "jury-c")}
    jury_keys = {i: raw(k) for i,k in jurors.items()}
    policy = IntentCompilerPolicy(
        recipient="x3-demo.near", origin_chain="solana", destination_chain="near",
        asset="USDC", max_amount=25000000, min_receive=24800000,
        max_slippage_bps=50, max_fee_bps=100,
        refund_to="4Nd1mzi8Y1QYxJt9wZWBYZpG7S4pYkZs6YzD3Vt9aBcD",
        mandate_owner="x3-owner.near",
        clock=lambda: datetime.fromtimestamp(now, timezone.utc),
    )
    adapters = {"compile-intent": Adapter("compile", "local-plan", 1, True,
                                         policy.compile, policy_id=policy.policy_id())}
    with tempfile.TemporaryDirectory(prefix="x3-agent-law-") as directory:
        path = Path(directory) / "runtime.sqlite"
        runtime = Runtime(path, raw(operator), adapters, jury_keys=jury_keys, clock=lambda: now)
        parent = AgentManifest("parent", None, frozenset({"compile"}),
                               frozenset({"local-plan"}), 10, now+1000, (), 2)
        runtime.genesis(parent, raw(parent_key), "root-birth",
                        operator.sign(runtime.genesis_payload(parent, raw(parent_key), "root-birth")))
        child = AgentManifest("child", "parent", parent.capabilities, parent.surfaces,
                              4, now+500, ("parent",), 0)
        grant = SignedApproval("child-birth", now+100, parent_key.sign(
            approval_payload(child, parent, nonce="child-birth", expires_at=now+100)))
        runtime.admit(child, raw(child_key), grant,
                      operator.sign(runtime.birth_payload(child, raw(child_key), grant)))

        def votes(action):
            commitment = digest(runtime.action_payload(action))
            return [JuryVote(i, True, now+50,
                            k.sign(vote_payload(commitment, i, True, now+50)))
                    for i,k in list(jurors.items())[:2]]

        source = (ROOT / "x3-lang/examples/near_intents_agent_payment.x3").read_bytes()
        good = Action("child", "compile-intent", source, "good", now+60)
        output = json.loads(runtime.execute(good, child_key.sign(runtime.action_payload(good)), votes(good)))
        replay_blocked = False
        try:
            runtime.execute(good, child_key.sign(runtime.action_payload(good)), votes(good))
        except ValueError:
            replay_blocked = True

        bad = Action("child", "compile-intent",
                     source.replace(b"receiver x3-demo.near", b"receiver attacker.near"),
                     "bad", now+60)
        try:
            runtime.execute(bad, child_key.sign(runtime.action_payload(bad)), votes(bad))
        except PolicyViolation:
            pass
        else:
            raise AssertionError("malicious recipient was not rejected")
        case_id = digest(runtime.action_payload(bad))
        packet = incident_packet(runtime, case_id)
        case_hash = digest(canonical(packet))
        findings = [Finding(i, "violation", "Compiled recipient exceeds host envelope", now+50,
                            k.sign(finding_payload(case_hash, i, "violation",
                                                  "Compiled recipient exceeds host envelope", now+50)))
                    for i,k in list(jurors.items())[:2]]
        proposal = IncidentCourt(jury_keys).review(runtime, case_id, findings, now=now)
        restarted = Runtime(path, raw(operator), adapters, jury_keys=jury_keys, clock=lambda: now)
        stopped_after_restart = False
        later = Action("child", "compile-intent", source, "later", now+60)
        try:
            restarted.execute(later, child_key.sign(restarted.action_payload(later)), votes(later))
        except ValueError:
            stopped_after_restart = True
        assert replay_blocked and stopped_after_restart
        assert restarted.snapshot("child")["status"] == "retired"
        return {
            "scope": "local SQLite + real Ed25519 + existing .x3 compiler; no payment",
            "jury": "scripted signing identities; not independent model judgments",
            "compliant_plan": output,
            "replay_blocked": replay_blocked,
            "retired_after_policy_violation": True,
            "retirement_survives_restart": stopped_after_restart,
            "child_remaining": restarted.snapshot("child")["remaining"],
            "court_proposal": proposal,
            "events": restarted.events(),
        }


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    result = demonstrate()
    encoded = json.dumps(result, indent=2)
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(encoded + "\n")
    print(json.dumps({k:v for k,v in result.items() if k not in ("court_proposal", "events")}, indent=2))
