import importlib.util
import tempfile
import time
import unittest
from contextlib import closing
from pathlib import Path

spec = importlib.util.spec_from_file_location("x3_liquidity", Path(__file__).with_name("liquidity.py"))
module = importlib.util.module_from_spec(spec)
import sys
sys.modules[spec.name] = module
spec.loader.exec_module(module)
Book, Error = module.LiquidityBook, module.LiquidityError

ADDR = "0xf311B0CD27d4fB9C1a615df8579004008aEFc228"

class LiquidityTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.book = Book(Path(self.tmp.name) / "liquidity.db")
        self.book.register("x3-owned", ADDR, "x3-demo.near")
        self.book.configure_offer("bnb-near", "x3-owned", source_asset="BSC.BNB",
            destination_asset="NEAR", price_num=3, price_den=2,
            fee_bps=100, max_input=1_000_000, expires_at=int(time.time()) + 600)

    def test_no_unauthorized_liquidity(self):
        with self.assertRaisesRegex(Error, "authorization not verified"):
            self.book.reserve("r1", "bnb-near", 100)

    def test_cannot_self_certify_authorization(self):
        with self.assertRaisesRegex(Error, "chain/wallet adapter"):
            self.book.mark_authorized("x3-owned", verified=True)

    def test_unverified_provider_stays_inactive_even_with_inventory_claim(self):
        with closing(self.book._db()) as db, db:
            db.execute("UPDATE offers SET verified_inventory=100000 WHERE id='bnb-near'")
        with self.assertRaises(Error):
            self.book.reserve("r1", "bnb-near", 100)

    def test_quote_exact_integer_price_and_overbooking(self):
        # Internal test-only setup: simulates a future independently verified adapter.
        with closing(self.book._db()) as db, db:
            db.execute("UPDATE providers SET authorized=1 WHERE id='x3-owned'")
            db.execute("UPDATE offers SET verified_inventory=150 WHERE id='bnb-near'")
        quote = self.book.reserve("r1", "bnb-near", 100)
        self.assertEqual((quote.destination_amount, quote.fee_amount), (148, 2))
        with self.assertRaisesRegex(Error, "insufficient"):
            self.book.reserve("r2", "bnb-near", 100)
        self.book.release("r1")
        self.assertEqual(self.book.reserve("r2", "bnb-near", 100).destination_amount, 148)

    def test_replayed_reservation_id_fails(self):
        with closing(self.book._db()) as db, db:
            db.execute("UPDATE providers SET authorized=1")
            db.execute("UPDATE offers SET verified_inventory=100000")
        self.book.reserve("same", "bnb-near", 100)
        with self.assertRaises(Exception):
            self.book.reserve("same", "bnb-near", 100)

    def test_invalid_math_and_expiry(self):
        with self.assertRaises(Error):
            self.book.reserve("r1", "bnb-near", -5)
        with self.assertRaises(Error):
            self.book.configure_offer("bad", "x3-owned", source_asset="BNB",
                destination_asset="NEAR",price_num=0,price_den=1,
                fee_bps=1,max_input=1,expires_at=int(time.time()) + 10)

if __name__ == "__main__":
    unittest.main()
