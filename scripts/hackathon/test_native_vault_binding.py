"""Synthetic readback/comparator tests; these do not claim chain execution."""
from __future__ import annotations
import base64
import copy
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import unittest

ROOT = Path(__file__).resolve().parents[2]
MODULE = ROOT / "scripts/hackathon/native_vault_binding.py"
SOURCE = (ROOT / "x3-lang/examples/near_native_vault_supplier.x3").read_bytes()
NOW = 1_900_000_000_000


class NativeVaultBindingTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.mod = None
        if MODULE.exists():
            spec = importlib.util.spec_from_file_location("native_vault_binding", MODULE)
            cls.mod = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(cls.mod)

    def setUp(self):
        self.assertIsNotNone(self.mod, "generated-policy Sandbox binding is not implemented")
        self.context = {"network": "sandbox", "vault_account": "vault.testnet", "wasm_sha256": "ab" * 32,
                        "policy_version": 1, "compiled_at_ms": NOW}
        self.request = {"source_base64": base64.b64encode(SOURCE).decode(), "context": self.context}

    def fixture(self):
        binding = self.mod.compile_binding(self.request)
        observation = {"vault_account": "vault.testnet", "block_hash": "1" * 32, "block_height": 100,
                       "code_hash": self.mod.code_hash(self.context["wasm_sha256"]),
                       "configuration": copy.deepcopy(binding["constructor"])}
        return binding, observation

    def test_generated_constructor_matches_exact_readback(self):
        binding, observed = self.fixture()
        report = self.mod.verify_readback(binding, observed, now_ms=NOW + 1)
        self.assertTrue(report["configuration_matches"])
        self.assertTrue(report["code_hash_matches"])
        self.assertFalse(report["cryptographic_proof"])
        self.assertFalse(report["owner_authorized"])
        self.assertFalse(report["public_chain"])

    def test_every_constructor_field_mismatch_fails(self):
        binding, observed = self.fixture()
        for field in observed["configuration"]:
            changed = copy.deepcopy(observed)
            changed["configuration"][field] = 1 if field == "deadline_ms" else "other.testnet"
            with self.subTest(field=field), self.assertRaises(ValueError):
                self.mod.verify_readback(binding, changed, now_ms=NOW + 1)

    def test_missing_extra_and_wrong_type_configuration_fails(self):
        binding, observed = self.fixture()
        for change in (None, [], {**observed["configuration"], "ignored": True},
                       {k: v for k, v in observed["configuration"].items() if k != "budget"},
                       {**observed["configuration"], "cap": 200000000000000000000000}):
            with self.subTest(change=change), self.assertRaises(ValueError):
                self.mod.verify_readback(binding, {**observed, "configuration": change}, now_ms=NOW + 1)

    def test_wrong_account_or_code_hash_fails(self):
        binding, observed = self.fixture()
        for field, value in (("vault_account", "other.testnet"), ("code_hash", "1" * 32)):
            with self.subTest(field=field), self.assertRaises(ValueError):
                self.mod.verify_readback(binding, {**observed, field: value}, now_ms=NOW + 1)

    def test_readback_requires_sane_block_identity(self):
        binding, observed = self.fixture()
        for field, value in (("block_hash", ""), ("block_hash", "0" * 44), ("block_height", True),
                             ("block_height", -1), ("block_height", "100")):
            with self.subTest(field=field), self.assertRaises(ValueError):
                self.mod.verify_readback(binding, {**observed, field: value}, now_ms=NOW + 1)

    def test_modified_manifest_and_constructor_are_not_trusted(self):
        binding, observed = self.fixture()
        changed = copy.deepcopy(binding)
        changed["manifest"]["constructor"]["cap"] = "999999999999999999999999"
        changed["manifest"]["manifest_sha256"] = self.mod.backend.manifest_digest(changed["manifest"])
        changed["constructor"] = changed["manifest"]["constructor"]
        with self.assertRaises(ValueError):
            self.mod.verify_readback(changed, {**observed, "configuration": changed["constructor"]}, now_ms=NOW + 1)

    def test_stale_and_expired_readbacks_fail(self):
        binding, observed = self.fixture()
        for now in (NOW - 1, NOW + 300001, NOW + 3600001):
            with self.subTest(now=now), self.assertRaises(ValueError):
                self.mod.verify_readback(binding, observed, now_ms=now)

    def test_only_sandbox_context_is_allowed(self):
        for network in ("testnet", "mainnet", "sandbox-other"):
            with self.subTest(network=network), self.assertRaises(ValueError):
                self.mod.compile_binding({**self.request, "context": {**self.context, "network": network}})

    def test_source_only_cap_change_produces_only_cap_difference(self):
        before = self.mod.compile_binding(self.request)
        changed = SOURCE.replace(b"per_payment 200000000000000000000000", b"per_payment 50000000000000000000000")
        after = self.mod.compile_binding({**self.request, "source_base64": base64.b64encode(changed).decode()})
        self.assertEqual(after["constructor"]["cap"], "50000000000000000000000")
        self.assertEqual({k: v for k, v in before["constructor"].items() if k != "cap"},
                         {k: v for k, v in after["constructor"].items() if k != "cap"})
        self.assertNotEqual(before["manifest"]["source_sha256"], after["manifest"]["source_sha256"])

    def test_noncanonical_base64_and_unknown_context_fail(self):
        for value in ("?", self.request["source_base64"] + "\n", "", 1):
            with self.subTest(value=value), self.assertRaises(ValueError):
                self.mod.compile_binding({**self.request, "source_base64": value})
        with self.assertRaises(ValueError):
            self.mod.compile_binding({**self.request, "context": {**self.context, "ignore_limits": True}})

    def test_base58_known_vectors(self):
        self.assertEqual(self.mod.code_hash("00" * 32), "1" * 32)
        self.assertEqual(self.mod.code_hash("00" * 31 + "01"), "1" * 31 + "2")

    def test_cli_rejects_duplicate_keys_and_oversized_input(self):
        for payload in (b'{"context":{},"context":{}}', b" " * 131073):
            run = subprocess.run([sys.executable, str(MODULE), "compile"], input=payload,
                                 capture_output=True, timeout=10)
            self.assertEqual(run.returncode, 2)
            self.assertEqual(run.stdout, b"")

    def test_cli_compiles_and_verifies_actual_module_output(self):
        result = subprocess.run([sys.executable, str(MODULE), "compile"], input=json.dumps(self.request).encode(),
                                capture_output=True, timeout=10)
        self.assertEqual(result.returncode, 0, result.stderr)
        binding = json.loads(result.stdout)
        _, observed = self.fixture()
        result = subprocess.run([sys.executable, str(MODULE), "verify"],
                                input=json.dumps({"binding": binding, "observation": observed, "now_ms": NOW+1}).encode(),
                                capture_output=True, timeout=10)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(json.loads(result.stdout)["configuration_matches"])

if __name__ == "__main__":
    unittest.main()
