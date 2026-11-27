"""Real Rust compiler -> owner signature -> real SQLite Agent Law runtime."""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from cryptography.hazmat.primitives import serialization
from agent_constitution import AgentManifest
from agent_admission_signed import SignedApproval, approval_payload
from agent_runtime import Runtime, Adapter, Action, StampGatePolicy, digest

SCOPE = "sandbox:pir05"
ARTIFACT = b"read-only fixture"
DOMAIN = b"X3/PIR/OWNER-GRANT/1\x00"
BIN = os.environ.get("X3_PIR_COMPILER")

def raw(key):
    return key.public_key().public_bytes(serialization.Encoding.Raw, serialization.PublicFormat.Raw)

def source(subject, parent, depth, expires=80, artifact=ARTIFACT):
    parent = json.dumps(parent) if parent is not None else "none"
    return (f'capability {{ subject "{subject}"; parent {parent}; scope "{SCOPE}"; '
            f'tool "reader" "{hashlib.sha256(artifact).hexdigest()}"; no_asset; '
            f'expires {expires}; depth {depth}; requirements []; }}')

def compiled(owner, src):
    out = subprocess.run([str(BIN), SCOPE, "10"], input=src.encode(), capture_output=True, check=True)
    record = json.loads(out.stdout)
    signature = owner.sign(DOMAIN + bytes.fromhex(record["source_sha256"]) + bytes.fromhex(record["encoded"]))
    return {**record, "signature": signature.hex()}

@unittest.skipUnless(BIN, "set X3_PIR_COMPILER to run Rust-to-runtime integration gate")
class PirRuntimeTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.db = Path(self.tmp.name) / "runtime.sqlite"
        self.owner = Ed25519PrivateKey.generate()
        self.root = Ed25519PrivateKey.generate()
        self.child = Ed25519PrivateKey.generate()
        self.calls = []
        self.adapters = {"reader": Adapter("read","local",1,False,lambda b: self.calls.append(b) or b"ok")}
        self.grants = [compiled(self.owner,source("root",None,2)),
                       compiled(self.owner,source("child","root",1,expires=70))]
        self.r = self.open()
        self.admit(self.r)

    def admit(self, runtime):
        self.parent = AgentManifest("root",None,frozenset({"read"}),frozenset({"local"}),20,100,(),2)
        runtime.genesis(self.parent,raw(self.root),"root-birth",
                       self.owner.sign(runtime.genesis_payload(self.parent,raw(self.root),"root-birth")))
        m=AgentManifest("child","root",frozenset({"read"}),frozenset({"local"}),5,90,("root",),0)
        a=SignedApproval("birth-child",60,self.root.sign(approval_payload(m,self.parent,nonce="birth-child",expires_at=60)))
        runtime.admit(m,raw(self.child),a,self.owner.sign(runtime.birth_payload(m,raw(self.child),a)))

    def open(self, **kw):
        opts=dict(clock=lambda:10,pir_scope=SCOPE,pir_grants=self.grants)
        opts.update(kw)
        return Runtime(self.db,raw(self.owner),self.adapters,**opts)

    def execute(self, artifact=ARTIFACT, expiry=50, nonce="one", key=None, runtime=None):
        r=runtime or self.r
        action=Action("child","reader",artifact,nonce,expiry)
        return r.execute(action,(key or self.child).sign(r.action_payload(action)))

    def test_success_replay_and_restart(self):
        self.assertEqual(self.execute(),b"ok")
        with self.assertRaises(ValueError): self.execute(runtime=self.open())
        self.assertEqual(self.calls,[ARTIFACT])
        self.assertEqual(self.r.snapshot("child")["remaining"],4)
        reserved = [json.loads(e["payload"]) for e in self.r.events() if json.loads(e["payload"])["kind"] == "reserved"]
        self.assertEqual(reserved[0]["details"]["pir_grant_sha256"], hashlib.sha256(bytes.fromhex(self.grants[1]["encoded"])).hexdigest())

    def test_artifact_and_expiry_gate(self):
        for kw in (dict(artifact=b"different"),dict(expiry=71)):
            with self.assertRaisesRegex(ValueError,"PIR"):
                self.execute(**kw)
        self.assertEqual(self.calls,[])
        self.assertEqual(self.r.snapshot("child")["remaining"],5)

    def test_owner_signature_and_config_downgrade(self):
        bad=[dict(g) for g in self.grants]
        bad[1]["signature"]="00"*64
        with self.assertRaises(ValueError): self.open(pir_grants=bad)
        with self.assertRaises(ValueError): self.open(pir_scope=None,pir_grants=None)
        with self.assertRaises(ValueError): self.open(pir_grants=self.grants[:1])

    def test_parent_revocation_and_wrong_signer(self):
        with self.assertRaises(ValueError): self.execute(key=self.root)
        self.r.revoke("root","operator stop")
        with self.assertRaises(ValueError): self.execute(runtime=self.open())
        self.assertEqual(self.calls,[])

    def test_missing_or_expanded_parent_grant(self):
        cases = [
            [self.grants[1]],
            [compiled(self.owner,source("root",None,1)),self.grants[1]],
            [compiled(self.owner,source("root",None,2,expires=60)),self.grants[1]],
            [compiled(self.owner,source("root",None,2,artifact=b"other")),self.grants[1]],
        ]
        for i, grants in enumerate(cases):
            self.db=Path(self.tmp.name)/f"case{i}.sqlite"
            r=self.open(pir_grants=grants)
            self.admit(r)
            with self.assertRaisesRegex(ValueError,"PIR"): self.execute(runtime=r)
            self.assertEqual(r.snapshot("child")["remaining"],5)
        self.assertEqual(self.calls,[])

    def test_strict_stamp_path_checks_same_grant(self):
        self.db=Path(self.tmp.name)/"stamped.sqlite"
        issuer=Ed25519PrivateKey.generate()
        policy=StampGatePolicy(raw(issuer),"a"*64,("signed_agent_identity","capability_scope","source_digest"))
        r=self.open(stamp_policies={"reader":policy},stamp_enforcement="all")
        self.admit(r)
        bad=Action("child","reader",b"other","bad",50)
        with self.assertRaisesRegex(ValueError,"PIR"):
            r.issue_stamp_for_action(bad,self.child.sign(r.action_payload(bad)),issuer)
        good=Action("child","reader",ARTIFACT,"good",50)
        sig=self.child.sign(r.action_payload(good))
        with self.assertRaises(ValueError): r.execute(good,sig)
        stamp=r.issue_stamp_for_action(good,sig,issuer)
        self.assertEqual(r.execute(good,sig,stamp=stamp),b"ok")
        self.assertEqual(self.calls,[ARTIFACT])
