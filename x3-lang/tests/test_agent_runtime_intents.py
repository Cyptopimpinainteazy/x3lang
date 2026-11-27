"""Real compiler boundary tests; no network or wallet dispatch."""
import json
from pathlib import Path
import unittest
from datetime import datetime, timezone
from agent_runtime_intents import IntentCompilerPolicy

SOURCE = (Path(__file__).resolve().parents[1] / "examples/near_intents_agent_payment.x3").read_bytes()

class IntentCompilerTests(unittest.TestCase):
    def policy(self):
        return IntentCompilerPolicy(
            recipient="x3-demo.near", origin_chain="solana", destination_chain="near",
            asset="USDC", max_amount=25000000, min_receive=24800000,
            max_slippage_bps=50, max_fee_bps=100,
            refund_to="4Nd1mzi8Y1QYxJt9wZWBYZpG7S4pYkZs6YzD3Vt9aBcD",
            mandate_owner="x3-owner.near", clock=lambda: datetime(2026,11,13,tzinfo=timezone.utc),
        )

    def test_real_source_compiles(self):
        result=json.loads(self.policy().compile(SOURCE))
        self.assertEqual(result["leg"]["amount"],25000000)
        self.assertEqual(result["leg"]["recipient"],"x3-demo.near")
        self.assertEqual(result["kind"],"local-compiled-plan-no-payment")

    def test_attacker_recipient_rejected(self):
        with self.assertRaisesRegex(ValueError,"recipient"):
            self.policy().compile(SOURCE.replace(b"receiver x3-demo.near",b"receiver attacker.near"))

    def test_overspend_rejected(self):
        with self.assertRaisesRegex(ValueError,"amount"):
            self.policy().compile(SOURCE.replace(b"amount 25000000",b"amount 26000000"))

    def test_missing_nonce_does_not_compile(self):
        with self.assertRaises(Exception):
            self.policy().compile(SOURCE.replace(b"    require nonce unused agent_payment_001",b""))

    def test_refund_rebinding_rejected(self):
        with self.assertRaisesRegex(ValueError,"refund"):
            self.policy().compile(SOURCE.replace(b"timeout 10m refund Solana.USDC",b"timeout 10m refund Solana.USDC to 11111111111111111111111111111111"))

    def test_invalid_encoding_and_size(self):
        for source in (bytes([255]), b"x"*65537, "not bytes"):
            with self.assertRaises((ValueError,TypeError)):
                self.policy().compile(source)
