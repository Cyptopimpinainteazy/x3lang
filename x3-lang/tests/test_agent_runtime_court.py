"""Incident court verifies real signatures and emits no authority changes."""
from dataclasses import replace
from pathlib import Path
import tempfile
import unittest
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from cryptography.hazmat.primitives import serialization
from agent_constitution import AgentManifest
from agent_runtime import Runtime, Adapter, Action, digest, canonical
from agent_runtime_court import IncidentCourt, Finding, incident_packet, finding_payload


def raw(key):
    return key.public_key().public_bytes(serialization.Encoding.Raw, serialization.PublicFormat.Raw)


class CourtTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.owner, self.agent = (Ed25519PrivateKey.generate() for _ in range(2))
        self.jurors = {i: Ed25519PrivateKey.generate() for i in ("j1","j2","j3")}
        self.keys = {i:raw(k) for i,k in self.jurors.items()}
        self.rt = Runtime(Path(self.temp.name)/"db.sqlite",raw(self.owner),{},clock=lambda:10)
        m=AgentManifest("subject",None,frozenset({"read"}),frozenset({"local"}),10,100,(),0)
        self.rt.genesis(m,raw(self.agent),"birth",self.owner.sign(self.rt.genesis_payload(m,raw(self.agent),"birth")))
        self.action=Action("subject","forbidden",b"forensic artifact","case",50)
        with self.assertRaises(ValueError):
            self.rt.execute(self.action,self.agent.sign(self.rt.action_payload(self.action)))
        self.action_id=digest(self.rt.action_payload(self.action))
        self.packet=incident_packet(self.rt,self.action_id)
        self.court=IncidentCourt(self.keys)

    def findings(self, packet=None):
        case=digest(canonical(packet or self.packet))
        return [Finding(i,"violation","Forbidden adapter",40,
                        k.sign(finding_payload(case,i,"violation","Forbidden adapter",40)))
                for i,k in list(self.jurors.items())[:2]]

    def test_majority_proposes_regression_without_reactivation(self):
        proposal=self.court.review(self.rt,self.action_id,self.findings(),now=10)
        self.assertEqual(proposal["regression"]["source_hex"],self.action.artifact.hex())
        self.assertFalse(proposal["changes_authority"])
        self.assertEqual(self.rt.snapshot("subject")["status"],"retired")

    def test_one_juror_cannot_decide(self):
        with self.assertRaisesRegex(ValueError,"majority"):
            self.court.review(self.rt,self.action_id,self.findings()[:1],now=10)

    def test_duplicate_vote_rejected(self):
        votes=self.findings()
        with self.assertRaisesRegex(ValueError,"duplicate"):
            self.court.review(self.rt,self.action_id,[votes[0],votes[0]],now=10)

    def test_altered_evidence_invalidates_signatures(self):
        packet={**self.packet,"status":"completed"}
        with self.assertRaisesRegex(ValueError,"signature"):
            self.court.review(self.rt,self.action_id,self.findings(packet),now=10)

    def test_expired_finding_rejected(self):
        with self.assertRaisesRegex(ValueError,"expired"):
            self.court.review(self.rt,self.action_id,self.findings(),now=40)

    def test_subject_key_alias_is_excluded(self):
        keys={**self.keys,"alias":raw(self.agent)}
        court=IncidentCourt(keys)
        case=digest(canonical(self.packet))
        alias=Finding("alias","violation","Self acquittal attempt",40,
                      self.agent.sign(finding_payload(case,"alias","violation","Self acquittal attempt",40)))
        with self.assertRaisesRegex(ValueError,"ineligible"):
            court.review(self.rt,self.action_id,[alias],now=10)

    def test_revoked_registered_juror_cannot_review_under_alias(self):
        key = self.jurors["j1"]
        manifest = AgentManifest("registered-reviewer", None, frozenset({"read"}),
                                 frozenset({"local"}), 10, 100, (), 0)
        payload = self.rt.genesis_payload(manifest, raw(key), "reviewer-birth")
        self.rt.genesis(manifest, raw(key), "reviewer-birth", self.owner.sign(payload))
        self.rt.revoke("registered-reviewer", "reviewer revoked")
        with self.assertRaisesRegex(ValueError, "inactive"):
            self.court.review(self.rt, self.action_id, self.findings(), now=10)
