"""Offline compiler integration tests; no wallet, node, or chain evidence."""
from __future__ import annotations

import copy
import hashlib
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
MODULE = ROOT / "near_native_vault.py"
SOURCE = b"""mandate supplier {
    target near_native_vault_v1
    owner owner.testnet
    agent agent.testnet
    spend Near.NEAR
    per_payment 200000000000000000000000
    lifetime_budget 400000000000000000000000
    allow recipients allowed.testnet
    expires 1h
}
"""
NOW = 1_794_096_000_000
CONTEXT = {
    "network": "sandbox", "vault_account": "vault.testnet",
    "wasm_sha256": hashlib.sha256(b"offline contract identity fixture").hexdigest(),
    "policy_version": 1,
}


class NativeVaultPolicyTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.native = None
        if MODULE.is_file():
            spec = importlib.util.spec_from_file_location("near_native_vault", MODULE)
            cls.native = importlib.util.module_from_spec(spec)
            sys.modules[spec.name] = cls.native
            spec.loader.exec_module(cls.native)

    def setUp(self):
        self.assertIsNotNone(self.native, "native vault compiler backend is not implemented")

    def compile(self, source=SOURCE, **overrides):
        context = {**CONTEXT, **overrides}
        return self.native.compile_policy(source, issued_at_ms=NOW, **context)

    def test_supported_source_generates_exact_constructor(self):
        result = self.compile()
        self.assertEqual(result["constructor"], {
            "owner": "owner.testnet", "agent": "agent.testnet", "recipient": "allowed.testnet",
            "cap": "200000000000000000000000", "budget": "400000000000000000000000",
            "deadline_ms": NOW + 3_600_000,
        })
        self.assertEqual(result["source_sha256"], hashlib.sha256(SOURCE).hexdigest())
        self.assertEqual(result["budget_semantics"], "lifetime_spent_plus_reserved")
        self.assertEqual(result["units"], "yoctonear")
        self.assertFalse(result["owner_authorized"])
        self.assertFalse(result["deployment_verified"])
        self.assertEqual(self.native.verify_manifest(result, SOURCE, now_ms=NOW, expected_compiled_at_ms=NOW, **CONTEXT), result["constructor"])

    def test_same_source_and_context_are_deterministic(self):
        self.assertEqual(self.compile(), self.compile())

    def test_rule_change_changes_constructor_and_identity(self):
        changed = SOURCE.replace(b"per_payment 200000000000000000000000", b"per_payment 50000000000000000000000")
        before, after = self.compile(), self.compile(changed)
        self.assertEqual(after["constructor"]["cap"], "50000000000000000000000")
        self.assertNotEqual(before["source_sha256"], after["source_sha256"])
        self.assertNotEqual(before["manifest_sha256"], after["manifest_sha256"])
        self.assertEqual({k: v for k, v in before["constructor"].items() if k != "cap"},
                         {k: v for k, v in after["constructor"].items() if k != "cap"})

    def test_unsupported_guard_semantics_are_never_dropped(self):
        for clause in (b"per_24h 400000000000000000000000", b"refund_to owner.testnet",
                       b"allow assets Near.NEAR", b"require slippage <= 0%", b"require fees <= 0 bps",
                       b"approve above 1 by 1 of owner.testnet", b"unknown ignored"):
            with self.subTest(clause=clause), self.assertRaises(ValueError):
                self.compile(SOURCE.replace(b"}\n", b"    " + clause + b"\n}\n"))

    def test_missing_duplicate_or_wrong_target_is_rejected(self):
        for source in (SOURCE.replace(b"    target near_native_vault_v1\n", b""),
                       SOURCE.replace(b"near_native_vault_v1", b"guard"),
                       SOURCE.replace(b"    lifetime_budget 400000000000000000000000\n", b""),
                       SOURCE.replace(b"}\n", b"    per_payment 1\n}\n"), SOURCE + b"unexpected\n"):
            with self.subTest(source=source), self.assertRaises(ValueError):
                self.compile(source)

    def test_only_native_near_and_one_recipient_are_supported(self):
        for old, new in ((b"Near.NEAR", b"Near.USDC"), (b"Near.NEAR", b"Base.ETH"),
                         (b"allowed.testnet", b"allowed.testnet, other.testnet"),
                         (b"allowed.testnet", b"allowed.testnet,")):
            with self.subTest(new=new), self.assertRaises(ValueError):
                self.compile(SOURCE.replace(old, new))

    def test_principals_are_distinct_testnet_accounts(self):
        for old, new in ((b"owner.testnet", b"agent.testnet"), (b"owner.testnet", b"vault.testnet"),
                         (b"allowed.testnet", b"agent.testnet"), (b"allowed.testnet", b"vault.testnet"),
                         (b"agent.testnet", b"agent.near"), (b"allowed.testnet", b"bad..testnet")):
            with self.subTest(new=new), self.assertRaises(ValueError):
                self.compile(SOURCE.replace(old, new))

    def test_amount_bounds_and_exact_units(self):
        for value in (b"0", b"-1", b"0.2", str(1 << 128).encode(), b"9" * 5000):
            with self.subTest(value=value[:40]), self.assertRaises(ValueError):
                self.compile(SOURCE.replace(b"200000000000000000000000", value))
        with self.assertRaises(ValueError):
            self.compile(SOURCE.replace(b"400000000000000000000000", b"1"))
        one = self.compile(SOURCE.replace(b"200000000000000000000000", b"1"))
        self.assertEqual(one["constructor"]["cap"], "1")

    def test_context_rejects_mainnet_bad_version_and_code_identity(self):
        for override in ({"network": "mainnet"}, {"network": "unknown"}, {"vault_account": "vault.near"},
                         {"policy_version": 0}, {"policy_version": True}, {"policy_version": 1 << 64},
                         {"wasm_sha256": "not-a-hash"}, {"wasm_sha256": "A" * 64}):
            with self.subTest(override=override), self.assertRaises(ValueError):
                self.compile(**override)

    def test_bad_source_time_and_expiry_fail_closed(self):
        for raw in (b"", b"\xff", b"x" * 65537):
            with self.assertRaises(ValueError):
                self.compile(raw)
        for value in (True, -1, 1.5, 253402300799999):
            with self.subTest(value=value), self.assertRaises(ValueError):
                self.native.compile_policy(SOURCE, issued_at_ms=value, **CONTEXT)
        for expiry in (b"0s", b"-1s", b"999999999999999999999d"):
            with self.assertRaises(ValueError):
                self.compile(SOURCE.replace(b"1h", expiry))

    def test_altered_constructor_is_rejected_even_with_rehashed_manifest(self):
        for field, value in (("owner", "other.testnet"), ("agent", "other.testnet"),
                             ("recipient", "other.testnet"), ("cap", "999999999999999999999999"),
                             ("budget", "999999999999999999999999"), ("deadline_ms", NOW + 7_200_000)):
            changed = self.compile()
            changed["constructor"][field] = value
            changed["manifest_sha256"] = self.native.manifest_digest(changed)
            with self.subTest(field=field), self.assertRaises(ValueError):
                self.native.verify_manifest(changed, SOURCE, now_ms=NOW, expected_compiled_at_ms=NOW, **CONTEXT)

    def test_context_mismatch_and_old_policy_version_are_rejected(self):
        manifest = self.compile()
        for override in ({"network": "testnet"}, {"vault_account": "other.testnet"},
                         {"policy_version": 2}, {"wasm_sha256": "f" * 64}):
            with self.subTest(override=override), self.assertRaises(ValueError):
                self.native.verify_manifest(manifest, SOURCE, now_ms=NOW, expected_compiled_at_ms=NOW, **{**CONTEXT, **override})

    def test_source_identity_compiler_identity_and_unknown_fields_are_checked(self):
        manifest = self.compile()
        self.assertTrue(manifest["compiler_sources_sha256"])
        for field, value in (("source_sha256", "f" * 64), ("compiler_sources_sha256", {}),
                             ("owner_authorized", True), ("deployment_verified", True), ("extra", "ignored")):
            changed = copy.deepcopy(manifest)
            changed[field] = value
            changed["manifest_sha256"] = self.native.manifest_digest(changed)
            with self.subTest(field=field), self.assertRaises(ValueError):
                self.native.verify_manifest(changed, SOURCE, now_ms=NOW, expected_compiled_at_ms=NOW, **CONTEXT)
        with self.assertRaises(ValueError):
            self.native.verify_manifest(manifest, SOURCE + b"// changed source bytes\n", now_ms=NOW, expected_compiled_at_ms=NOW, **CONTEXT)

    def test_recompilation_cannot_move_the_frozen_deadline(self):
        changed = self.compile()
        changed["compiled_at_ms"] += 1
        changed["constructor"]["deadline_ms"] += 1
        changed["manifest_sha256"] = self.native.manifest_digest(changed)
        with self.assertRaises(ValueError):
            self.native.verify_manifest(changed, SOURCE, now_ms=NOW + 1, expected_compiled_at_ms=NOW, **CONTEXT)

    def test_future_stale_and_expired_manifests_are_rejected(self):
        manifest = self.compile()
        for now in (NOW - 1, NOW + 300_001, NOW + 3_600_000, True):
            with self.subTest(now=now), self.assertRaises(ValueError):
                self.native.verify_manifest(manifest, SOURCE, now_ms=now, expected_compiled_at_ms=NOW, **CONTEXT)

    def test_manifest_bytes_reject_duplicates_nonfinite_and_bad_types(self):
        manifest = self.compile()
        raw = json.dumps(manifest).encode()
        self.assertEqual(self.native.verify_manifest_bytes(raw, SOURCE, now_ms=NOW, expected_compiled_at_ms=NOW, **CONTEXT), manifest["constructor"])
        for broken in (b'{"schema":"forged",' + raw[1:], b"NaN", b"[]", b"{" , raw.replace(b'"policy_version": 1', b'"policy_version": true')):
            with self.subTest(broken=broken[:60]), self.assertRaises(ValueError):
                self.native.verify_manifest_bytes(broken, SOURCE, now_ms=NOW, expected_compiled_at_ms=NOW, **CONTEXT)

    def test_existing_guard_parser_does_not_accept_native_profile_implicitly(self):
        import near_intents as ni
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "policy.x3"
            source.write_bytes(SOURCE)
            with self.assertRaises(ni.NearIntentsError):
                ni.mandates().parse_mandate(str(source))

    def test_cli_verification_requires_independently_pinned_time(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "policy.x3"
            manifest_path = Path(directory) / "manifest.json"
            source.write_bytes(SOURCE)
            manifest_path.write_text(json.dumps(self.compile()))
            args = [sys.executable, str(MODULE), "verify", str(source), "--network", "sandbox",
                    "--vault-account", "vault.testnet", "--wasm-sha256", CONTEXT["wasm_sha256"],
                    "--policy-version", "1", "--at-ms", str(NOW), "--manifest", str(manifest_path)]
            missing = subprocess.run(args, capture_output=True, text=True, timeout=20)
            self.assertEqual(missing.returncode, 2)
            self.assertEqual(missing.stdout, "")
            valid = subprocess.run(args + ["--approved-at-ms", str(NOW)], capture_output=True, text=True, timeout=20)
            self.assertEqual(valid.returncode, 0, valid.stderr)
            self.assertFalse(json.loads(valid.stdout)["deployment_verified"])
            mismatch = subprocess.run(args + ["--approved-at-ms", str(NOW - 1)], capture_output=True, text=True, timeout=20)
            self.assertEqual(mismatch.returncode, 2)
            self.assertEqual(mismatch.stdout, "")

    def test_short_expiry_and_trusted_time_types_are_checked(self):
        source = SOURCE.replace(b"expires 1h", b"expires 1ms")
        manifest = self.compile(source)
        with self.assertRaises(ValueError):
            self.native.verify_manifest(manifest, source, now_ms=NOW + 1, expected_compiled_at_ms=NOW, **CONTEXT)
        for value in (True, -1, float(NOW), str(NOW)):
            with self.subTest(value=value), self.assertRaises(ValueError):
                self.native.verify_manifest(self.compile(), SOURCE, now_ms=NOW, expected_compiled_at_ms=value, **CONTEXT)

    def test_cli_compiles_without_deployment_and_rejects_unsupported_policy(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "policy.x3"
            source.write_bytes(SOURCE)
            args = [sys.executable, str(MODULE), "compile", str(source), "--network", "sandbox",
                    "--vault-account", "vault.testnet", "--wasm-sha256", CONTEXT["wasm_sha256"],
                    "--policy-version", "1", "--at-ms", str(NOW)]
            result = subprocess.run(args, capture_output=True, text=True, timeout=20)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(json.loads(result.stdout), self.compile())
            self.assertEqual([p.name for p in Path(directory).iterdir()], ["policy.x3"])
            source.write_bytes(SOURCE.replace(b"lifetime_budget", b"per_24h"))
            failed = subprocess.run(args, capture_output=True, text=True, timeout=20)
            self.assertNotEqual(failed.returncode, 0)
            self.assertEqual(failed.stdout, "")


if __name__ == "__main__":
    unittest.main(verbosity=2)
