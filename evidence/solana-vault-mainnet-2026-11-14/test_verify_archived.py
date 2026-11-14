"""Tamper public archived inputs and regenerate checksums to test semantic refusal."""
import base64
import hashlib
import json
from pathlib import Path
import shutil
import tempfile
import unittest
import verify_archived as proof


class ArchivedPilotTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        source = Path(__file__).resolve().parent
        for path in source.iterdir():
            if path.is_file() and path.suffix in {".json", ".x3", ".md"}:
                shutil.copyfile(path, self.root / path.name)

    def tearDown(self):
        self.temp.cleanup()

    def change(self, filename, mutate):
        path = self.root / filename
        value = json.loads(path.read_text())
        mutate(value)
        path.write_text(json.dumps(value))
        manifest = self.root / "manifest.json"
        data = json.loads(manifest.read_text())
        data["sha256"][filename] = hashlib.sha256(path.read_bytes()).hexdigest()
        manifest.write_text(json.dumps(data))

    def test_original_archive(self):
        self.assertEqual(proof.verify(self.root)["archived_checks"], "passed")

    def test_changed_quote_recipient(self):
        self.change("live-quote.near-receipt.json",
                    lambda d: d["quote_response"]["quoteRequest"].update(recipient="attacker.near"))
        with self.assertRaises(ValueError):
            proof.verify(self.root)

    def test_changed_source_signature(self):
        self.change("finalized-execution-primary.json",
                    lambda d: d["transaction"]["transaction"]["signatures"].__setitem__(0, proof.NEAR_TX))
        with self.assertRaises(ValueError):
            proof.verify(self.root)

    def test_changed_execution_message(self):
        def mutate(d):
            raw = bytearray(base64.b64decode(d["messageBase64"]))
            raw[-1] ^= 1
            d["messageBase64"] = base64.b64encode(raw).decode()
        self.change("execution-submission.json", mutate)
        with self.assertRaises(Exception):
            proof.verify(self.root)

    def test_changed_near_recipient(self):
        def mutate(d):
            for receipt in d["receipts_outcome"]:
                receipt["outcome"]["logs"] = [line.replace("x3lang.near", "attacker.near")
                                              for line in receipt["outcome"]["logs"]]
        self.change("near-payout-transaction.json", mutate)
        with self.assertRaises(ValueError):
            proof.verify(self.root)

    def test_changed_provider_link(self):
        self.change("provider-latest-status.json",
                    lambda d: d["swapDetails"]["originChainTxHashes"][0].update(hash=proof.NEAR_TX))
        with self.assertRaises(ValueError):
            proof.verify(self.root)

    def test_changed_deployed_bytes(self):
        def mutate(d):
            value = d["result"]["value"][1]["data"]
            raw = bytearray(base64.b64decode(value[0]))
            raw[-1] ^= 1
            value[0] = base64.b64encode(raw).decode()
        self.change("finalized-accounts.json", mutate)
        with self.assertRaises(ValueError):
            proof.verify(self.root)

    def test_changed_approval_expiry(self):
        self.change("finalized-approval.json", lambda d: d["policyState"].update(pending_until=1794658708))
        with self.assertRaises(ValueError):
            proof.verify(self.root)

    def test_substituted_valid_owner_packet(self):
        setup = json.loads((self.root / "setup-transaction.json").read_text())["result"]
        setup_signature = json.loads((self.root / "setup.json").read_text())["signature"]
        def mutate(d):
            d["result"]["transaction"] = setup["transaction"]
            # Spoof the unsigned RPC metadata to preserve the expected approval debit.
            d["result"]["meta"]["postBalances"][0] = d["result"]["meta"]["preBalances"][0] - 1493440
        self.change("approval-transaction.json", mutate)
        self.change("finalized-approval.json", lambda d: d.update(signature=setup_signature))
        with self.assertRaises(ValueError):
            proof.verify(self.root)


if __name__ == "__main__":
    unittest.main()
