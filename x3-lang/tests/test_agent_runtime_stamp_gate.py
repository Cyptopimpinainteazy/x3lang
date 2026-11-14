"""Exact execution-boundary tests: a protected adapter cannot run without scoped stamp."""
import json
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from cryptography.hazmat.primitives import serialization
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from agent_constitution import AgentManifest
from agent_runtime import Runtime, Adapter, Action, StampGatePolicy, PolicyViolation, digest
from agent_scoped_stamp import StampRequest, issue_scoped_stamp


def raw(key):
    return key.public_key().public_bytes(serialization.Encoding.Raw, serialization.PublicFormat.Raw)


class ProtectedExecutionTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.db = Path(self.temp.name) / "runtime.sqlite"
        self.operator = Ed25519PrivateKey.generate()
        self.agent = Ed25519PrivateKey.generate()
        self.issuer = Ed25519PrivateKey.generate()
        self.executed = []
        def record_task(source):
            if source == b"attacker-source":
                raise PolicyViolation("deterministic recipient policy rejected")
            self.executed.append(source)
            return b"local-verified-output"
        self.adapter = Adapter("compile", "local-plan", 1, False, record_task,
                               policy_id="v1")
        self.policies = {"compile-intent": StampGatePolicy(raw(self.issuer), "a" * 64,
                          ("signed_agent_identity", "capability_scope", "source_digest"))}
        self.runtime = self.make_runtime()
        manifest = AgentManifest("test-agent", None, frozenset({"compile"}),
                                 frozenset({"local-plan"}), 5, 500, (), 0)
        birth = self.runtime.genesis_payload(manifest, raw(self.agent), "root-birth")
        self.runtime.genesis(manifest, raw(self.agent), "root-birth", self.operator.sign(birth))

    def make_runtime(self, stamp_policies=None):
        return Runtime(self.db, raw(self.operator), {"compile-intent": self.adapter},
                       clock=lambda: 100, stamp_policies=self.policies if stamp_policies is None else stamp_policies)

    def attempt(self, nonce="act-1", source=b"valid-source", stamp=None):
        action = Action("test-agent", "compile-intent", source, nonce, 150)
        signed = self.agent.sign(self.runtime.action_payload(action))
        return self.runtime.execute(action, signed, stamp=stamp)

    def stamp(self, nonce="act-1", source=b"valid-source", checks=None):
        action = Action("test-agent", "compile-intent", source, nonce, 150)
        return issue_scoped_stamp(StampRequest(
            "test-agent", "compile-intent", source, "a" * 64,
            90, 120, nonce,
            checks or self.policies["compile-intent"].required_checks,
            digest(self.runtime.action_payload(action))), self.issuer)

    def test_trusted_issuer_preflight_to_actual_dispatch(self):
        action = Action("test-agent", "compile-intent", b"valid-source", "fresh", 150)
        signature = self.agent.sign(self.runtime.action_payload(action))
        stamped = self.runtime.issue_stamp_for_action(action, signature, self.issuer)
        output = self.runtime.execute(action, signature, stamp=stamped)
        self.assertEqual(output, b"local-verified-output")
        self.assertEqual(self.executed, [b"valid-source"])

    def test_stamp_issue_requires_authentic_agent_signature(self):
        action = Action("test-agent", "compile-intent", b"valid-source", "fresh", 150)
        forged = Ed25519PrivateKey.generate().sign(self.runtime.action_payload(action))
        with self.assertRaises(ValueError):
            self.runtime.issue_stamp_for_action(action, forged, self.issuer)
        self.assertEqual(self.executed, [])

    def test_stamp_issue_requires_correct_issuer(self):
        action = Action("test-agent", "compile-intent", b"valid-source", "fresh", 150)
        signature = self.agent.sign(self.runtime.action_payload(action))
        with self.assertRaises(ValueError):
            self.runtime.issue_stamp_for_action(action, signature, Ed25519PrivateKey.generate())
        self.assertEqual(self.executed, [])

    def test_stamp_issue_fails_after_agent_revoked(self):
        action = Action("test-agent", "compile-intent", b"valid-source", "fresh", 150)
        signature = self.agent.sign(self.runtime.action_payload(action))
        self.runtime.revoke("test-agent", "owner termination")
        with self.assertRaises(ValueError):
            self.runtime.issue_stamp_for_action(action, signature, self.issuer)
        self.assertEqual(self.executed, [])

    def test_issuer_cannot_claim_unverified_ten_commandments(self):
        guarded = {"compile-intent": StampGatePolicy(
            raw(self.issuer), "a" * 64, ("major_action_jury",))}
        # Protected trust config is permanent: cannot quietly weaken/change it.
        with self.assertRaises(ValueError):
            self.make_runtime(stamp_policies=guarded)
        self.assertEqual(self.executed, [])

    def test_full_local_lifecycle_stamp_violation_retirement_and_evidence(self):
        # Real signatures, live source-binding, trusted adapter and SQLite state.
        good = Action("test-agent", "compile-intent", b"valid-source", "good-1", 150)
        signed_good = self.agent.sign(self.runtime.action_payload(good))
        good_stamp = self.runtime.issue_stamp_for_action(good, signed_good, self.issuer)
        self.assertEqual(self.runtime.execute(good, signed_good, stamp=good_stamp),
                         b"local-verified-output")

        # A valid scope stamp is NOT proof of semantic safety: the actual
        # adapter still rejects the malformed program at execution time.
        bad = Action("test-agent", "compile-intent", b"attacker-source", "bad-1", 150)
        signed_bad = self.agent.sign(self.runtime.action_payload(bad))
        bad_stamp = self.runtime.issue_stamp_for_action(bad, signed_bad, self.issuer)
        with self.assertRaises(PolicyViolation):
            self.runtime.execute(bad, signed_bad, stamp=bad_stamp)
        self.assertEqual(self.executed, [b"valid-source"])
        self.assertEqual(self.runtime.snapshot("test-agent")["status"], "retired")
        # The operator can additionally revoke, and the entire hash-checked
        # event sequence (including failure) survives a database reopen.
        self.runtime.revoke("test-agent", "confirmed policy violation")
        before = self.runtime.events()
        self.assertTrue(any(json.loads(e["payload"])["kind"] == "retired" for e in before))
        self.runtime = self.make_runtime()
        self.assertEqual(self.runtime.snapshot("test-agent")["status"], "revoked")
        self.assertEqual(before, self.runtime.events())
        later = Action("test-agent", "compile-intent", b"valid-source", "later-1", 150)
        signed_later = self.agent.sign(self.runtime.action_payload(later))
        with self.assertRaises(ValueError):
            self.runtime.issue_stamp_for_action(later, signed_later, self.issuer)
        self.assertEqual(len(self.executed), 1)

    def strict_runtime(self):
        strict_db = Path(self.temp.name) / "strict-runtime.sqlite"
        runtime = Runtime(strict_db, raw(self.operator), {"compile-intent": self.adapter},
                          clock=lambda: 100, stamp_policies=self.policies,
                          stamp_enforcement="all")
        manifest = AgentManifest("test-agent", None, frozenset({"compile"}),
                                 frozenset({"local-plan"}), 5, 500, (), 0)
        birth = runtime.genesis_payload(manifest, raw(self.agent), "strict-birth")
        runtime.genesis(manifest, raw(self.agent), "strict-birth", self.operator.sign(birth))
        return runtime, strict_db

    def test_strict_mode_protected_execution(self):
        runtime, _ = self.strict_runtime()
        action = Action("test-agent", "compile-intent", b"valid-source", "strict-1", 150)
        signed = self.agent.sign(runtime.action_payload(action))
        stamp = runtime.issue_stamp_for_action(action, signed, self.issuer)
        self.assertEqual(runtime.execute(action, signed, stamp=stamp),
                         b"local-verified-output")
        self.assertEqual(self.executed, [b"valid-source"])

    def test_strict_mode_blocks_unstamped_adapter(self):
        runtime, _ = self.strict_runtime()
        action = Action("test-agent", "compile-intent", b"valid-source", "strict-1", 150)
        signed = self.agent.sign(runtime.action_payload(action))
        with self.assertRaisesRegex(ValueError, "requires a scoped X3 stamp"):
            runtime.execute(action, signed)
        self.assertEqual(self.executed, [])

    def test_strict_mode_rejects_missing_coverage_at_start(self):
        additional = Adapter("compile", "local-plan", 1, False,
                             lambda source: b"unused", policy_id="v1")
        with self.assertRaisesRegex(ValueError, "every registered adapter"):
            Runtime(Path(self.temp.name) / "coverage.sqlite", raw(self.operator),
                    {"compile-intent": self.adapter, "new-adapter": additional},
                    clock=lambda: 100, stamp_policies=self.policies,
                    stamp_enforcement="all")

    def test_strict_mode_refuses_downgrade_after_reopen(self):
        runtime, db = self.strict_runtime()
        with self.assertRaisesRegex(ValueError, "runtime trust configuration changed"):
            Runtime(db, raw(self.operator), {"compile-intent": self.adapter},
                    clock=lambda: 100, stamp_policies=self.policies,
                    stamp_enforcement="configured")

    def test_strict_mode_fails_if_policy_removed_at_runtime(self):
        runtime, _ = self.strict_runtime()
        runtime.stamp_policies.clear()
        action = Action("test-agent", "compile-intent", b"valid-source", "strict-1", 150)
        signed = self.agent.sign(runtime.action_payload(action))
        with self.assertRaisesRegex(ValueError, "live stamp policy differs"):
            runtime.execute(action, signed)
        self.assertEqual(self.executed, [])

    def test_strict_mode_cannot_swap_valid_policy_to_attacker_issuer(self):
        runtime, _ = self.strict_runtime()
        attacker_issuer = Ed25519PrivateKey.generate()
        runtime.stamp_policies["compile-intent"] = StampGatePolicy(
            raw(attacker_issuer), "a" * 64, self.policies["compile-intent"].required_checks)
        action = Action("test-agent", "compile-intent", b"valid-source", "swapped-1", 150)
        signed = self.agent.sign(runtime.action_payload(action))
        with self.assertRaisesRegex(ValueError, "live stamp policy differs"):
            runtime.issue_stamp_for_action(action, signed, attacker_issuer)
        forged = issue_scoped_stamp(StampRequest(
            "test-agent", "compile-intent", b"valid-source", "a" * 64,
            90, 120, "swapped-1", self.policies["compile-intent"].required_checks,
            digest(runtime.action_payload(action))), attacker_issuer)
        with self.assertRaisesRegex(ValueError, "live stamp policy differs"):
            runtime.execute(action, signed, stamp=forged)
        self.assertEqual(self.executed, [])

    def test_strict_mode_none_policy_value_never_bypasses(self):
        runtime, _ = self.strict_runtime()
        runtime.stamp_policies["compile-intent"] = None
        action = Action("test-agent", "compile-intent", b"valid-source", "probe-1", 150)
        signed = self.agent.sign(runtime.action_payload(action))
        with self.assertRaisesRegex(ValueError, "invalid live stamp configuration"):
            runtime.execute(action, signed)
        self.assertEqual(self.executed, [])

    def test_separate_agents_can_use_same_nonce_without_collision(self):
        second = Ed25519PrivateKey.generate()
        manifest = AgentManifest("second-agent", None, frozenset({"compile"}),
                                 frozenset({"local-plan"}), 5, 500, (), 0)
        birth = self.runtime.genesis_payload(manifest, raw(second), "second-birth")
        self.runtime.genesis(manifest, raw(second), "second-birth", self.operator.sign(birth))
        for agent_id, key in (("test-agent", self.agent), ("second-agent", second)):
            action = Action(agent_id, "compile-intent", b"valid-source", "shared", 150)
            signed = key.sign(self.runtime.action_payload(action))
            stamp = self.runtime.issue_stamp_for_action(action, signed, self.issuer)
            self.assertEqual(self.runtime.execute(action, signed, stamp=stamp), b"local-verified-output")
        self.assertEqual(len(self.executed), 2)

    def test_cross_runtime_stamp_does_not_authorize_different_identity(self):
        first = Action("test-agent", "compile-intent", b"valid-source", "transfer", 150)
        signed = self.agent.sign(self.runtime.action_payload(first))
        stamp = self.runtime.issue_stamp_for_action(first, signed, self.issuer)
        other_db = Path(self.temp.name) / "other.sqlite"
        other_agent = Ed25519PrivateKey.generate()
        other = Runtime(other_db, raw(self.operator), {"compile-intent": self.adapter},
                        stamp_policies=self.policies, clock=lambda: 100)
        manifest = AgentManifest("test-agent", None, frozenset({"compile"}),
                                 frozenset({"local-plan"}), 5, 500, (), 0)
        birth = other.genesis_payload(manifest, raw(other_agent), "other-birth")
        other.genesis(manifest, raw(other_agent), "other-birth", self.operator.sign(birth))
        fresh = Action("test-agent", "compile-intent", b"valid-source", "transfer", 150)
        fresh_signature = other_agent.sign(other.action_payload(fresh))
        with self.assertRaisesRegex(ValueError, "signed action not bound"):
            other.execute(fresh, fresh_signature, stamp=stamp)
        self.assertEqual(self.executed, [])

    def test_revoking_agent_does_not_block_other_agent_nonce(self):
        second = Ed25519PrivateKey.generate()
        manifest = AgentManifest("second-agent", None, frozenset({"compile"}),
                                 frozenset({"local-plan"}), 5, 500, (), 0)
        birth = self.runtime.genesis_payload(manifest, raw(second), "second-birth")
        self.runtime.genesis(manifest, raw(second), "second-birth", self.operator.sign(birth))
        self.runtime.revoke_stamp("test-agent", "shared", "owner revoked")
        action = Action("second-agent", "compile-intent", b"valid-source", "shared", 150)
        signed = second.sign(self.runtime.action_payload(action))
        stamp = self.runtime.issue_stamp_for_action(action, signed, self.issuer)
        self.assertEqual(self.runtime.execute(action, signed, stamp=stamp), b"local-verified-output")

    def test_strict_mode_refuses_unknown_mode(self):
        with self.assertRaisesRegex(ValueError, "unrecognized stamp enforcement"):
            Runtime(Path(self.temp.name) / "invalid-mode.sqlite", raw(self.operator),
                    {"compile-intent": self.adapter}, stamp_policies=self.policies,
                    stamp_enforcement="all_except_one")

    def test_valid_stamp_dispatches_once(self):
        self.attempt(stamp=self.stamp())
        self.assertEqual(self.executed, [b"valid-source"])
        self.assertEqual(self.runtime.snapshot("test-agent")["remaining"], 4)

    def test_missing_stamp_fails_before_any_effect(self):
        with self.assertRaisesRegex(ValueError, "requires"):
            self.attempt()
        self.assertEqual(self.executed, [])
        self.assertEqual(self.runtime.snapshot("test-agent")["remaining"], 5)

    def test_tampered_artifact_fails_before_dispatch(self):
        with self.assertRaises(ValueError):
            self.attempt(source=b"attacker-source", stamp=self.stamp())
        self.assertEqual(self.executed, [])

    def test_stamp_nonce_must_bind_signed_action(self):
        with self.assertRaises(ValueError):
            self.attempt(nonce="act-2", stamp=self.stamp())
        self.assertEqual(self.executed, [])

    def test_stamp_cannot_be_reused_with_different_action(self):
        signed_stamp = self.stamp()
        self.attempt(stamp=signed_stamp)
        with self.assertRaises(ValueError):
            self.attempt(nonce="act-1", stamp=signed_stamp)
        self.assertEqual(len(self.executed), 1)

    def test_revocation_blocks_dispatch(self):
        self.runtime.revoke_stamp("test-agent", "act-1", "operator revoked")
        with self.assertRaisesRegex(ValueError, "revoked"):
            self.attempt(stamp=self.stamp())
        self.assertEqual(self.executed, [])

    def test_revocation_survives_restart(self):
        self.runtime.revoke_stamp("test-agent", "act-1", "operator revoked")
        self.runtime = self.make_runtime()
        with self.assertRaisesRegex(ValueError, "revoked"):
            self.attempt(stamp=self.stamp())
        self.assertEqual(self.executed, [])

    def test_missing_mandatory_check_fails_closed(self):
        with self.assertRaises(ValueError):
            self.attempt(stamp=self.stamp(checks=("source_digest",)))
        self.assertEqual(self.executed, [])

    def test_untrusted_issuer_fails_closed(self):
        other = Ed25519PrivateKey.generate()
        stamp = issue_scoped_stamp(StampRequest("test-agent", "compile-intent",
            b"valid-source", "a" * 64, 90, 120, "act-1",
            self.policies["compile-intent"].required_checks,
            digest(self.runtime.action_payload(Action("test-agent", "compile-intent", b"valid-source", "act-1", 150)))), other)
        with self.assertRaises(ValueError):
            self.attempt(stamp=stamp)
        self.assertEqual(self.executed, [])

    def test_reopen_without_protection_fails(self):
        with self.assertRaisesRegex(ValueError, "runtime trust configuration changed"):
            self.make_runtime(stamp_policies={})

    def test_reopen_with_weakened_policy_fails(self):
        weak = {"compile-intent": StampGatePolicy(raw(self.issuer), "a" * 64, ("source_digest",))}
        with self.assertRaisesRegex(ValueError, "runtime trust configuration changed"):
            self.make_runtime(stamp_policies=weak)

    def test_protected_execution_rejects_expired_stamp(self):
        expired = issue_scoped_stamp(StampRequest("test-agent", "compile-intent",
            b"valid-source", "a" * 64, 60, 95, "act-1",
            self.policies["compile-intent"].required_checks,
            digest(self.runtime.action_payload(Action("test-agent", "compile-intent", b"valid-source", "act-1", 150)))), self.issuer)
        with self.assertRaises(ValueError):
            self.attempt(stamp=expired)
        self.assertEqual(self.executed, [])


if __name__ == "__main__":
    unittest.main()
