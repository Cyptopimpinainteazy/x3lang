"""Security regression suite: tool ingress cannot skip signed stamped runtime."""
import base64
import json
import tempfile
import unittest
from pathlib import Path
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from cryptography.hazmat.primitives import serialization

from agent_constitution import AgentManifest
from agent_runtime import Action, Adapter, Runtime, StampGatePolicy, PolicyViolation
from agent_tool_gateway import LocalToolGateway


def public(key):
    return key.public_key().public_bytes(serialization.Encoding.Raw, serialization.PublicFormat.Raw)


class GatewayTests(unittest.TestCase):
    def setUp(self):
        folder = tempfile.TemporaryDirectory()
        self.addCleanup(folder.cleanup)
        self.db = Path(folder.name) / "gate.sqlite"
        self.operator = Ed25519PrivateKey.generate()
        self.agent = Ed25519PrivateKey.generate()
        self.issuer = Ed25519PrivateKey.generate()
        self.called = []
        def worker(source):
            if source == b"unauthorized":
                raise PolicyViolation("forbidden payload")
            self.called.append(source)
            return b"verified-result"
        self.adapters = {"compile-intent": Adapter("compile", "local-plan", 1, False,
                                                 worker, policy_id="v1")}
        self.policies = {"compile-intent": StampGatePolicy(public(self.issuer),
                           "a" * 64, ("signed_agent_identity", "capability_scope", "source_digest"))}
        self.runtime = Runtime(self.db, public(self.operator), self.adapters,
                               stamp_policies=self.policies, stamp_enforcement="all", clock=lambda: 100)
        manifest = AgentManifest("agent-one", None, frozenset({"compile"}),
                                 frozenset({"local-plan"}), 5, 500, (), 0)
        birth = self.runtime.genesis_payload(manifest, public(self.agent), "birth-one")
        self.runtime.genesis(manifest, public(self.agent), "birth-one", self.operator.sign(birth))
        self.gateway = LocalToolGateway(self.runtime, {"compile-intent": self.issuer})

    def request(self, source=b"good", nonce="task-1", agent_key=None):
        key = agent_key or self.agent
        action = Action("agent-one", "compile-intent", source, nonce, 150)
        return {
            "agent_id": action.agent_id, "adapter": action.adapter, "nonce": action.nonce,
            "expires_at": action.expires_at, "artifact_b64": base64.b64encode(source).decode(),
            "agent_signature_hex": key.sign(self.runtime.action_payload(action)).hex(),
        }

    def send(self, request):
        return self.gateway.submit(json.dumps(request, sort_keys=True).encode())

    def test_authorized_execution_and_one_use_nonce(self):
        request = self.request()
        self.assertEqual(self.send(request), b"verified-result")
        self.assertEqual(self.called, [b"good"])
        with self.assertRaises(ValueError):
            self.send(request)
        self.assertEqual(self.called, [b"good"])

    def test_unsigned_request_is_rejected(self):
        request = self.request()
        request["agent_signature_hex"] = "00" * 64
        with self.assertRaises(ValueError):
            self.send(request)
        self.assertEqual(self.called, [])

    def test_wrong_signer_is_rejected(self):
        with self.assertRaises(ValueError):
            self.send(self.request(agent_key=Ed25519PrivateKey.generate()))
        self.assertEqual(self.called, [])

    def test_no_untrusted_stamp_or_tool_callback_allowed(self):
        for field, value in (("stamp", {"passed_checks": ["source_digest"]}),
                             ("callback", "worker"),
                             ("issuer_private_key", "stolen"),
                             ("tool_endpoint", "http://127.0.0.1:9999")):
            with self.subTest(field=field), self.assertRaisesRegex(ValueError, "unexpected gateway"):
                self.send({**self.request(), field: value})
        self.assertEqual(self.called, [])

    def test_unknown_adapter_is_rejected(self):
        request = self.request()
        request["adapter"] = "direct-gpu-shell"
        with self.assertRaisesRegex(ValueError, "adapter not allowed"):
            self.send(request)
        self.assertEqual(self.called, [])

    def test_modified_artifact_is_rejected(self):
        request = self.request()
        request["artifact_b64"] = base64.b64encode(b"unauthorized").decode()
        with self.assertRaises(ValueError):
            self.send(request)
        self.assertEqual(self.called, [])

    def test_bad_envelope_fails_closed(self):
        for payload in (b"{}", b"[]", b"not json", b"",
                        b"\xff", b"{}" * 600000):
            with self.subTest(data=payload[:10]), self.assertRaises(ValueError):
                self.gateway.submit(payload)
        self.assertEqual(self.called, [])

    def test_gateway_requires_strict_runtime(self):
        with self.assertRaisesRegex(ValueError, "strictly stamped"):
            runtime = Runtime(Path(self.db.parent) / "loose.sqlite", public(self.operator),
                              self.adapters, stamp_policies=self.policies, clock=lambda: 100)
            LocalToolGateway(runtime, {"compile-intent": self.issuer})

    def test_gateway_rejects_wrong_or_missing_issuer(self):
        for issuers in ({}, {"compile-intent": Ed25519PrivateKey.generate()}):
            with self.assertRaises(ValueError):
                LocalToolGateway(self.runtime, issuers)

    def test_policy_violation_retires_agent(self):
        with self.assertRaises(PolicyViolation):
            self.send(self.request(source=b"unauthorized"))
        self.assertEqual(self.runtime.snapshot("agent-one")["status"], "retired")
        self.assertEqual(self.called, [])
        with self.assertRaises(ValueError):
            self.send(self.request(source=b"good", nonce="task-2"))
        self.assertEqual(self.called, [])

    def test_revocation_denies_future_gateway_requests(self):
        self.runtime.revoke("agent-one", "operator halt")
        with self.assertRaises(ValueError):
            self.send(self.request())
        self.assertEqual(self.called, [])


if __name__ == "__main__":
    unittest.main()
