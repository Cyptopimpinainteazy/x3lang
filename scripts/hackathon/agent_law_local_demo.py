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
from agent_runtime_handoff import (build_incident_payload, recheck_rejection, sign_incident_handoff,
                                   verify_incident_handoff)


def raw(key):
    return key.public_key().public_bytes(serialization.Encoding.Raw, serialization.PublicFormat.Raw)


DEMO_NOW = int(datetime(2026, 11, 13, tzinfo=timezone.utc).timestamp())


def demo_policy(now=DEMO_NOW):
    """The host envelope the demo's compiler adapter enforces."""
    return IntentCompilerPolicy(
        recipient="x3-demo.near", origin_chain="solana", destination_chain="near",
        asset="USDC", max_amount=25000000, min_receive=24800000,
        max_slippage_bps=50, max_fee_bps=100,
        refund_to="4Nd1mzi8Y1QYxJt9wZWBYZpG7S4pYkZs6YzD3Vt9aBcD",
        mandate_owner="x3-owner.near",
        clock=lambda: datetime.fromtimestamp(now, timezone.utc),
    )


def replay_regression(proposal):
    """Independently replay an x3.regression-proposal v1 against this compiler.

    Refuses unless the proposal names this demo's exact policy revision. The
    verdict says whether the source is still rejected with the expected error.
    """
    if proposal.get("format") != "x3.regression-proposal" or proposal.get("version") != 1:
        raise ValueError("unsupported regression proposal")
    case = proposal["case"]
    policy = demo_policy()
    if case.get("policy_id") != policy.policy_id():
        raise ValueError("proposal names a different policy revision; cannot replay it here")
    source = bytes.fromhex(case["source_hex"])
    if digest(source) != case["source_sha256"]:
        raise ValueError("regression source does not match its digest")
    try:
        policy.compile(source)
        observed = {"rejected": False, "error_type": None, "error": None}
    except PolicyViolation as exc:
        observed = {"rejected": True, "error_type": "PolicyViolation", "error": str(exc)}
    except (ValueError, TypeError) as exc:
        observed = {"rejected": True, "error_type": type(exc).__name__, "error": str(exc)}
    return {"replayed": True, "policy_id": policy.policy_id(), "source_sha256": case["source_sha256"],
            "expected": case["expected"], "expected_error": case.get("expected_error"),
            "observed": observed,
            "matches": case["expected"] == "reject" and observed["rejected"]
                       and observed["error"] == case.get("expected_error")}


def demonstrate(handoff_key=None):
    """Run the lifecycle; with handoff_key, also sign the incident for outside review."""
    now = DEMO_NOW
    operator, parent_key, child_key = (Ed25519PrivateKey.generate() for _ in range(3))
    jurors = {name: Ed25519PrivateKey.generate() for name in ("jury-a", "jury-b", "jury-c")}
    jury_keys = {i: raw(k) for i,k in jurors.items()}
    policy = demo_policy(now)
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
        probe_error = None
        later = Action("child", "compile-intent", source, "later", now+60)
        try:
            restarted.execute(later, child_key.sign(restarted.action_payload(later)), votes(later))
        except ValueError as exc:
            stopped_after_restart = True
            probe_error = str(exc)
        assert replay_blocked and stopped_after_restart
        assert restarted.snapshot("child")["status"] == "retired"
        handoff = {}
        if handoff_key is not None:
            # Exported from the reopened runtime: the evidence is what survived on disk.
            payload = build_incident_payload(
                restarted, case_id, issued_at=now,
                recheck=recheck_rejection(policy.compile, bad.artifact),
                restart={"status_after_restart": restarted.snapshot("child")["status"],
                         "probe_action_refused": stopped_after_restart, "probe_error": probe_error},
                court=proposal)
            envelope = sign_incident_handoff(payload, handoff_key)
            verify_incident_handoff(envelope, [raw(handoff_key)])
            handoff = {"incident_handoff": {"envelope_sha256": digest(envelope),
                                            "signer": raw(handoff_key).hex(),
                                            "format": "x3.incident-handoff", "version": 1},
                       "incident_handoff_envelope": envelope.decode("ascii")}
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
            **handoff,
        }


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--handoff", type=Path,
                        help="write the signed incident handoff (x3.incident-handoff v1) here")
    parser.add_argument("--handoff-signer", type=Path,
                        help="write the handoff signer's Ed25519 public key (hex) here")
    parser.add_argument("--replay-regression", type=Path,
                        help="replay an x3.regression-proposal v1 against this compiler and exit "
                             "(0 = still rejected as proposed, 1 = not)")
    args = parser.parse_args()
    if args.replay_regression:
        verdict = replay_regression(json.loads(args.replay_regression.read_text()))
        print(json.dumps(verdict, indent=2))
        sys.exit(0 if verdict["matches"] else 1)
    if (args.handoff is None) != (args.handoff_signer is None):
        parser.error("--handoff and --handoff-signer are used together")
    # Ephemeral per run; the private key is never written. Consumers pin the public key.
    result = demonstrate(Ed25519PrivateKey.generate() if args.handoff else None)
    envelope = result.pop("incident_handoff_envelope", None)
    if args.handoff:
        for path, text in ((args.handoff, envelope), (args.handoff_signer, result["incident_handoff"]["signer"] + "\n")):
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(text.encode("ascii"))
    encoded = json.dumps(result, indent=2)
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(encoded + "\n")
    print(json.dumps({k:v for k,v in result.items() if k not in ("court_proposal", "events")}, indent=2))
