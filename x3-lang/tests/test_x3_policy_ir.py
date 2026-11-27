import hashlib
import unittest

from agent_constitution import AgentManifest
from agent_admission_signed import approval_payload
from bridge.settlement import window_id
from x3_policy_ir import Profile, PolicyIRError, bind, verify, bind_agent_admission, bind_settlement_window

SOURCE = b"fixture policy bytes"
SCOPE = "sandbox:fixture"
NATIVE = bytes(range(32))

class PolicyIRTests(unittest.TestCase):
    def test_domain_and_exact_binding(self):
        results = [bind(p, SOURCE, SCOPE, NATIVE) for p in Profile]
        self.assertEqual(len({r.digest for r in results}), 3)
        self.assertEqual(bind(Profile.EVM_ACTION, SOURCE, SCOPE, NATIVE), results[0])
        self.assertTrue(verify(results[0].encoded, Profile.EVM_ACTION, SOURCE, SCOPE, NATIVE))
        for args in [(Profile.AGENT_ADMISSION, SOURCE, SCOPE, NATIVE),
                     (Profile.EVM_ACTION, b"changed", SCOPE, NATIVE),
                     (Profile.EVM_ACTION, SOURCE, "sandbox:other", NATIVE),
                     (Profile.EVM_ACTION, SOURCE, SCOPE, bytes(32))]:
            self.assertFalse(verify(results[0].encoded, *args))
        self.assertFalse(verify(results[0].encoded + b" ", Profile.EVM_ACTION, SOURCE, SCOPE, NATIVE))

    def test_fail_closed_inputs(self):
        for args in [("evm.action.v1", SOURCE, SCOPE, NATIVE),
                     (Profile.EVM_ACTION, b"", SCOPE, NATIVE),
                     (Profile.EVM_ACTION, SOURCE, "bad scope", NATIVE),
                     (Profile.EVM_ACTION, SOURCE, SCOPE, b"short")]:
            with self.assertRaises(PolicyIRError):
                bind(*args)

    def test_agent_native_payload_preserved(self):
        parent = AgentManifest("p", None, frozenset({"read"}), frozenset({"local"}), 20, 300, (), 2)
        child = AgentManifest("c", "p", frozenset({"read"}), frozenset({"local"}), 10, 200, ("p",), 0)
        args = dict(nonce="n1", expires_at=150)
        record = bind_agent_admission(SOURCE, SCOPE, child, parent, **args)
        native = hashlib.sha256(approval_payload(child, parent, **args)).digest()
        self.assertEqual(record, bind(Profile.AGENT_ADMISSION, SOURCE, SCOPE, native))
        self.assertNotEqual(record.digest, bind_agent_admission(SOURCE, SCOPE, child, parent, nonce="n2", expires_at=150).digest)

    def test_settlement_window_preserved(self):
        terms = dict(vault="vault.testnet", initiator="alice.testnet", recipient="bob.testnet",
                     hashlock=bytes(32), near_yocto=10, funding_deadline_ns=100,
                     expiry_ns=200, binding=bytes(range(32)))
        record = bind_settlement_window(SOURCE, SCOPE, **terms)
        self.assertEqual(record, bind(Profile.NEAR_BSC_WINDOW, SOURCE, SCOPE, window_id(**terms)))
        terms["near_yocto"] = 11
        self.assertNotEqual(record.digest, bind_settlement_window(SOURCE, SCOPE, **terms).digest)
