"""Signed incident handoff: exact stored evidence, pinned signer, fail closed."""
from datetime import datetime, timezone
import json
from pathlib import Path
import tempfile
import unittest

from cryptography.hazmat.primitives import serialization
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey

from agent_constitution import AgentManifest
from agent_runtime import Action, Adapter, PolicyViolation, Runtime, canonical, digest
from agent_runtime_court import Finding, IncidentCourt, finding_payload, incident_packet
from agent_runtime_handoff import (EFFECTS, FORMAT, MAX_ENVELOPE_BYTES, VERSION, build_incident_payload,
                                   recheck_rejection, sign_incident_handoff, verify_incident_handoff)
from agent_runtime_intents import IntentCompilerPolicy

SOURCE = (Path(__file__).resolve().parents[1] / "examples/near_intents_agent_payment.x3").read_bytes()
NOW = int(datetime(2026, 11, 13, tzinfo=timezone.utc).timestamp())


def raw(key):
    return key.public_key().public_bytes(serialization.Encoding.Raw, serialization.PublicFormat.Raw)


class HandoffTests(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.path = Path(temp.name) / "runtime.sqlite"
        self.operator, self.agent_key, self.evidence_key = (Ed25519PrivateKey.generate() for _ in range(3))
        self.jurors = {i: Ed25519PrivateKey.generate() for i in ("j1", "j2", "j3")}
        self.jury_keys = {i: raw(k) for i, k in self.jurors.items()}
        self.policy = IntentCompilerPolicy(
            recipient="x3-demo.near", origin_chain="solana", destination_chain="near",
            asset="USDC", max_amount=25000000, min_receive=24800000,
            max_slippage_bps=50, max_fee_bps=100,
            refund_to="4Nd1mzi8Y1QYxJt9wZWBYZpG7S4pYkZs6YzD3Vt9aBcD",
            mandate_owner="x3-owner.near", clock=lambda: datetime.fromtimestamp(NOW, timezone.utc))
        self.adapters = {"compile-intent": Adapter("compile", "local-plan", 1, False, self.policy.compile,
                                                   policy_id=self.policy.policy_id())}
        self.rt = self.runtime()
        manifest = AgentManifest("payer", None, frozenset({"compile"}), frozenset({"local-plan"}),
                                 5, NOW + 1000, (), 0)
        self.rt.genesis(manifest, raw(self.agent_key), "birth",
                        self.operator.sign(self.rt.genesis_payload(manifest, raw(self.agent_key), "birth")))
        self.good = Action("payer", "compile-intent", SOURCE, "good", NOW + 60)
        self.rt.execute(self.good, self.agent_key.sign(self.rt.action_payload(self.good)))
        self.bad = Action("payer", "compile-intent",
                          SOURCE.replace(b"receiver x3-demo.near", b"receiver attacker.near"), "bad", NOW + 60)
        with self.assertRaises(PolicyViolation):
            self.rt.execute(self.bad, self.agent_key.sign(self.rt.action_payload(self.bad)))
        self.case = digest(self.rt.action_payload(self.bad))

    def runtime(self):
        return Runtime(self.path, raw(self.operator), self.adapters, jury_keys=self.jury_keys, clock=lambda: NOW)

    def proposal(self):
        case_hash = digest(canonical(incident_packet(self.rt, self.case)))
        reason = "Compiled recipient exceeds host envelope"
        findings = [Finding(i, "violation", reason, NOW + 50,
                            k.sign(finding_payload(case_hash, i, "violation", reason, NOW + 50)))
                    for i, k in list(self.jurors.items())[:2]]
        return IncidentCourt(self.jury_keys).review(self.rt, self.case, findings, now=NOW)

    def payload(self, runtime=None, **overrides):
        options = {"issued_at": NOW, "recheck": recheck_rejection(self.policy.compile, self.bad.artifact),
                   "restart": {"status_after_restart": "retired", "probe_action_refused": True,
                               "probe_error": "identity or ancestor inactive"},
                   "court": self.proposal()}
        options.update(overrides)
        return build_incident_payload(runtime or self.rt, self.case, **options)

    def envelope(self, payload=None, key=None):
        return sign_incident_handoff(payload or self.payload(), key or self.evidence_key)

    def resign(self, mutate):
        """Re-sign a mutated payload with a trusted key, so semantic checks are what fail."""
        payload = json.loads(json.dumps(self.payload()))
        mutate(payload)
        body = canonical(payload)
        return canonical({"format": FORMAT, "version": VERSION, "signer": raw(self.evidence_key).hex(),
                          "payload": body.decode("ascii"), "signature": self.evidence_key.sign(body).hex()})

    def verify(self, envelope):
        return verify_incident_handoff(envelope, [raw(self.evidence_key)])

    def test_round_trip_binds_exact_stored_evidence(self):
        payload = self.verify(self.envelope(self.payload(runtime=self.runtime())))
        incident = payload["incident"]
        self.assertEqual(incident["action_digest"], self.case)
        self.assertEqual(digest(bytes.fromhex(payload["request_hex"])), self.case)
        self.assertEqual(bytes.fromhex(incident["artifact_hex"]), self.bad.artifact)
        self.assertEqual(incident["status"], "failed")
        self.assertEqual(incident["subject"]["status"], "retired")
        self.assertEqual(payload["recheck"]["error"], "policy violation: recipient")
        self.assertEqual(payload["effects"], EFFECTS)
        self.assertFalse(payload["effects"]["os_process_terminated"])
        self.assertFalse(payload["effects"]["blockchain_key_revoked"])
        retired = json.loads(payload["event_log"][payload["retirement_event_seq"] - 1]["payload"])
        self.assertEqual((retired["kind"], retired["details"]["action"]), ("retired", self.case))

    def test_envelope_is_deterministic_canonical_json(self):
        payload = self.payload()
        self.assertEqual(self.envelope(payload), self.envelope(payload))
        self.assertEqual(canonical(json.loads(self.envelope(payload))), self.envelope(payload))

    def test_unpinned_signer_rejected(self):
        with self.assertRaisesRegex(ValueError, "untrusted signer"):
            verify_incident_handoff(self.envelope(), [raw(Ed25519PrivateKey.generate())])

    def test_tampered_payload_rejected(self):
        envelope = json.loads(self.envelope())
        envelope["payload"] = envelope["payload"].replace('"status":"failed"', '"status":"denied"', 1)
        with self.assertRaisesRegex(ValueError, "invalid signature"):
            self.verify(canonical(envelope))

    def test_signature_swapped_from_another_signer_rejected(self):
        envelope = json.loads(self.envelope())
        envelope["signature"] = json.loads(self.envelope(key=Ed25519PrivateKey.generate()))["signature"]
        with self.assertRaisesRegex(ValueError, "invalid signature"):
            self.verify(canonical(envelope))

    def test_other_domain_signature_cannot_pose_as_handoff(self):
        manifest = AgentManifest("other", None, frozenset({"compile"}), frozenset({"local-plan"}), 1, NOW + 9, (), 0)
        body = self.rt.genesis_payload(manifest, raw(self.agent_key), "x")
        envelope = canonical({"format": FORMAT, "version": VERSION, "signer": raw(self.evidence_key).hex(),
                              "payload": body.decode("ascii"), "signature": self.evidence_key.sign(body).hex()})
        with self.assertRaisesRegex(ValueError, "payload"):
            self.verify(envelope)

    def test_unknown_format_or_version_rejected(self):
        for field, value in (("version", 2), ("format", "x3.other")):
            envelope = json.loads(self.envelope())
            envelope[field] = value
            with self.assertRaisesRegex(ValueError, "unsupported"):
                self.verify(canonical(envelope))

    def test_non_canonical_or_oversized_envelope_rejected(self):
        with self.assertRaisesRegex(ValueError, "canonical"):
            self.verify(json.dumps(json.loads(self.envelope()), indent=1).encode())
        with self.assertRaisesRegex(ValueError, "size"):
            self.verify(b" " * (MAX_ENVELOPE_BYTES + 1))

    def test_duplicate_keys_in_signed_payload_rejected(self):
        body = canonical(self.payload())
        body = b'{"version":1,' + body[1:]
        envelope = canonical({"format": FORMAT, "version": VERSION, "signer": raw(self.evidence_key).hex(),
                              "payload": body.decode("ascii"), "signature": self.evidence_key.sign(body).hex()})
        with self.assertRaisesRegex(ValueError, "canonical"):
            self.verify(envelope)

    def test_semantic_violations_rejected_even_when_signed(self):
        def edit_event(p):
            row = p["event_log"][0]
            row["payload"] = row["payload"].replace('"born"', '"borN"')

        def claim(effect):
            def mutate(p):
                p["effects"][effect] = not p["effects"][effect]
            return mutate

        cases = {
            "audit chain": edit_event,
            "contiguous": lambda p: p["event_log"].pop(0),
            "retired": lambda p: p["incident"]["subject"].update(status="active"),
            "effects": claim("os_process_terminated"),
            "effects ": claim("blockchain_key_revoked"),
            "request": lambda p: p.update(request_hex=p["request_hex"][:-2] + "20"),
            "artifact": lambda p: p["incident"].update(artifact_hex=p["incident"]["artifact_hex"] + "0a"),
            "court": lambda p: p["court"].update(case_hash="0" * 64),
            "retirement event": lambda p: p.update(retirement_event_seq=1),
            "fields": lambda p: p.update(extra=True),
            "runtime": lambda p: p["producer"].update(runtime_id="f" * 64),
            "recheck": lambda p: p.update(recheck={"rejected": False}),
            "restart": lambda p: p["restart"].update(status_after_restart="active"),
            "restart ": lambda p: p["restart"].update(probe_action_refused="yes"),
            "invalid event log": lambda p: p["event_log"].__setitem__(0, ["not", "a", "row"]),
            "ineligible": lambda p: p["court"]["findings"][0].update(juror=["unhashable"]),
        }
        for reason, mutate in cases.items():
            with self.subTest(reason=reason), self.assertRaisesRegex(ValueError, reason.strip()):
                self.verify(self.resign(mutate))

    def test_forged_court_finding_rejected_even_when_signed(self):
        def forge(p):
            p["court"]["findings"][0]["reason"] = "rewritten after signing"
        with self.assertRaisesRegex(ValueError, "finding signature"):
            self.verify(self.resign(forge))

    def test_court_without_majority_rejected_even_when_signed(self):
        with self.assertRaisesRegex(ValueError, "majority"):
            self.verify(self.resign(lambda p: p["court"]["findings"].pop()))

    def test_export_refuses_completed_action_and_accepts_runtime_denials(self):
        with self.assertRaisesRegex(ValueError, "rejected or failed"):
            build_incident_payload(self.rt, digest(self.rt.action_payload(self.good)), issued_at=NOW)
        other = tempfile.TemporaryDirectory()
        self.addCleanup(other.cleanup)
        rt = Runtime(Path(other.name) / "db.sqlite", raw(self.operator), self.adapters, clock=lambda: NOW)
        manifest = AgentManifest("narrow", None, frozenset({"read"}), frozenset({"local-plan"}), 5, NOW + 99, (), 0)
        rt.genesis(manifest, raw(self.agent_key), "n", self.operator.sign(rt.genesis_payload(manifest, raw(self.agent_key), "n")))
        denied = Action("narrow", "compile-intent", SOURCE, "d", NOW + 60)
        with self.assertRaisesRegex(ValueError, "forbidden"):
            rt.execute(denied, self.agent_key.sign(rt.action_payload(denied)))
        payload = build_incident_payload(rt, digest(rt.action_payload(denied)), issued_at=NOW)
        self.assertEqual(payload["incident"]["status"], "denied")
        self.assertIsNone(payload["recheck"])
        self.verify(self.envelope(payload))

    def test_export_refuses_when_recheck_accepts_the_artifact(self):
        with self.assertRaisesRegex(ValueError, "accepted"):
            recheck_rejection(self.policy.compile, SOURCE)
        with self.assertRaisesRegex(ValueError, "does not match"):
            self.payload(court={**self.proposal(), "case_hash": "1" * 64})

    def test_revoked_subject_is_not_a_retirement(self):
        other = tempfile.TemporaryDirectory()
        self.addCleanup(other.cleanup)
        rt = Runtime(Path(other.name) / "db.sqlite", raw(self.operator), self.adapters, clock=lambda: NOW)
        manifest = AgentManifest("narrow", None, frozenset({"read"}), frozenset({"local-plan"}), 5, NOW + 99, (), 0)
        rt.genesis(manifest, raw(self.agent_key), "n", self.operator.sign(rt.genesis_payload(manifest, raw(self.agent_key), "n")))
        denied = Action("narrow", "compile-intent", SOURCE, "d", NOW + 60)
        with self.assertRaises(ValueError):
            rt.execute(denied, self.agent_key.sign(rt.action_payload(denied)))
        rt.revoke("narrow", "operator override")
        with self.assertRaisesRegex(ValueError, "not retired"):
            build_incident_payload(rt, digest(rt.action_payload(denied)), issued_at=NOW)


if __name__ == "__main__":
    unittest.main()
