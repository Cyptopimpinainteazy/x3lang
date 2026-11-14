"""Stdlib unittest suite for synthetic Mt. Gox-inspired accounting controls."""
import importlib.util
import sys
import unittest
from pathlib import Path

source = Path(__file__).with_name("replay_mtgox_accounting.py")
spec = importlib.util.spec_from_file_location("mtgox_accounting", source)
module = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = module
spec.loader.exec_module(module)


class AccountingTests(unittest.TestCase):
    def test_six_invariants(self):
        report = module.evaluate()
        self.assertTrue(report["ok"])
        self.assertEqual((report["passed"], report["total"]), (6, 6))

    def test_changed_txid_never_double_debits(self):
        ledger = module.WithdrawalLedger(1000, 1000)
        self.assertEqual(ledger.process("withdrawal-a", "tx-a", 100), "settled")
        self.assertEqual(ledger.process("withdrawal-a", "tx-b", 100), "blocked_duplicate_intent")
        self.assertEqual(ledger.reserves_sats, 900)
        self.assertEqual(ledger.obligations_sats, 900)

    def test_missing_reserves_fail_closed(self):
        ledger = module.WithdrawalLedger(200, 500)
        self.assertEqual(ledger.process("withdrawal-b", "tx-c", 100), "blocked_missing_reserves")
        self.assertTrue(ledger.frozen)
        self.assertFalse(ledger.settled_intents)

    def test_valid_payment_passes(self):
        ledger = module.WithdrawalLedger(300, 300)
        self.assertEqual(ledger.process("withdrawal-c", "tx-d", 50), "settled")


if __name__ == "__main__":
    unittest.main()
