"""The agent and source entry points must commit to identical issuance terms."""
import copy
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import unittest

from coin import CoinError, author_coin, compile_coin
from ed25519 import b58encode

ROOT = Path(__file__).resolve().parents[1]
FIXTURE = ROOT / "coin/examples/dual_vm.x3"
SPEC = json.loads(FIXTURE.with_suffix(".json").read_text())
SOURCE = FIXTURE.read_text()


class CoinAuthoring(unittest.TestCase):
    def test_source_round_trip(self):
        generated = author_coin(SPEC)
        from_agent = compile_coin(generated)
        from_file = compile_coin(SOURCE)
        self.assertEqual(from_agent["declaration_hash"], from_file["declaration_hash"])
        self.assertEqual(from_agent["evm_solidity"], from_file["evm_solidity"])
        self.assertEqual(from_agent["svm_issuance"], from_file["svm_issuance"])
        self.assertNotEqual(from_agent["source_sha256"], from_file["source_sha256"])
        manifest = from_file["manifest"]
        encoded = json.dumps(manifest, separators=(",", ":"), sort_keys=True).encode()
        self.assertEqual(from_file["declaration_hash"], hashlib.sha256(encoded).hexdigest())
        self.assertIn(from_file["declaration_hash"], from_file["evm_solidity"])
        self.assertEqual(from_file["svm_issuance"]["memo"], "x3-coin-v1:" + from_file["declaration_hash"])

    def test_terms_are_committed_to_both_targets(self):
        changed = copy.deepcopy(SPEC)
        changed["svm"]["allocation"] = "1"
        changed["evm"]["allocation"] = "999999999"
        baseline = compile_coin(SOURCE)
        amended = compile_coin(author_coin(changed))
        for key in ("declaration_hash", "evm_solidity", "svm_issuance"):
            self.assertNotEqual(baseline[key], amended[key])

    def test_invalid_source(self):
        for bad in (
            SOURCE + "coin MORE {", SOURCE + "\0", SOURCE[:-2], SOURCE * 20,
            SOURCE.replace("  cap ", "  cap 1000000000\n  cap "),
            SOURCE.replace("  evm {", "  near {"), SOURCE.replace("coin DEMO", "coin DÉMO"),
            SOURCE.replace("1000000000", "1e9"), SOURCE.replace("600000000", "600000001"),
            SOURCE.replace('name "X3 Demo Credit"', 'name "bad\\""; contract Inject"'),
        ):
            with self.subTest(bad=bad[:40]), self.assertRaises(CoinError):
                compile_coin(bad)

    def test_structured_authoring_is_strict(self):
        invalid = (
            ("evm", "chain_id", "0"), ("evm", "allocation", 600000000), ("evm", "allocation", True),
            ("evm", "recipient", "0x"+"0"*40), ("evm", "recipient", SPEC["evm"]["contract"]),
            ("evm", "contract", "0xf2E246BB76DF876Cef8b38ae84130F4F55De395b"),
            ("svm", "recipient", SPEC["svm"]["mint"]), ("svm", "genesis_hash", SPEC["svm"]["genesis_hash"][:32]),
            ("svm", "mint", b58encode(b"\x01" + bytes(31))), ("svm", "allocation", "18446744073709551616"),
            (None, "name", "X\nY"), (None, "decimals", 19), (None, "symbol", "X3;"), (None, "cap", "01000000000"),
            (None, "extra", "ignored"),
        )
        for target, key, value in invalid:
            bad = copy.deepcopy(SPEC)
            (bad[target] if target else bad)[key] = value
            with self.subTest(key=key, value=value), self.assertRaises(CoinError):
                author_coin(bad)

    def test_module_entry_points(self):
        env = {**os.environ, "PYTHONPATH": str(ROOT)}
        def run(command, path):
            return subprocess.run([sys.executable, "-m", "coin", command, str(path)],
                                  env=env, capture_output=True, text=True, check=True).stdout
        self.assertEqual(run("author", FIXTURE.with_suffix(".json")), author_coin(SPEC))
        self.assertEqual(json.loads(run("build", FIXTURE)), compile_coin(SOURCE))
