"""No-network, authentic historical maker-signature regression tests.

These tests do not forge an on-chain Guard view or claim the original quote is
still live. Synthetic policy rows are plainly untrusted test inputs.
"""
from __future__ import annotations

from copy import deepcopy
from datetime import datetime, timedelta, timezone
import json
from pathlib import Path
import sys
import unittest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "x3-lang"))
sys.path.insert(0, str(Path(__file__).resolve().parent))

import base_sepolia_guarded_admission as a
import base_sepolia_x3_intent as compiler
from base_vault_quote_binding import make_unsigned_binding

AGENT = "0x1111111111111111111111111111111111111111"


class GenuineQuoteVaultBindingTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        fixture = json.loads((ROOT / "x3-lang/near/x3-tidepool-guard/src/signed_live_fixture.json").read_text())
        cls.response = json.loads(fixture["signed_payload"])
        cls.response["signature"] = fixture["signature"]
        cls.issued = a.parse_time(cls.response["timestamp"])
        cls.deadline = a.parse_time(cls.response["quote"]["deadline"])
        cls.program = compiler.compile_testnet_intent()
        cls.quote = a.check_quote(cls.response, cls.issued)

    def setUp(self) -> None:
        self.owner_terms = {
            "source_owner": a.SOURCE,
            "agent_signer": AGENT,
            "origin_chain_id": 84532,
            "token_contract": a.TOKEN,
            "token_code_sha256": a.TOKEN_CODE_SHA256,
            "destination_account": a.RECEIVER,
            "refund_to": a.SOURCE,
            "deposit_address": self.quote["deposit_address"],
            "amount_base_units": a.AMOUNT,
            "max_per_payment": a.AMOUNT,
            "max_total": a.AMOUNT,
            "approved_quote_sha256": self.quote["quote_sha256"],
            "approved_program_sha256": self.program["program_sha256"],
        }
        self.guard_row = {
            "owner": a.RECEIVER,
            "nonce": self.program["nonce"],
            "revoked": False,
            "quote_sha256": self.quote["quote_sha256"],
            "policy": {
                "program_sha256": self.program["program_sha256"],
                "amount_in": str(a.AMOUNT),
                "minimum_out": self.program["minimum_out"],
                "refund_to": a.SOURCE,
                "max_slippage_bps": 100,
                "expiry_ms": int(self.deadline.timestamp() * 1000) + 60_000,
            },
        }

    def preview(self, *, now: datetime | None = None, response: dict | None = None) -> dict:
        return make_unsigned_binding(response or self.response, self.guard_row, self.owner_terms, now=now or self.issued)

    def test_authentic_historical_maker_quote_binds_exact_vault_immutable_terms(self) -> None:
        result = self.preview()
        self.assertEqual(result["classification"], "UNSIGNED_VAULT_BINDING_PREVIEW_NOT_AUTHORIZED")
        self.assertEqual(result["quote_sha256"], self.quote["quote_sha256"])
        self.assertEqual(result["program_sha256"], self.program["program_sha256"])
        self.assertEqual(result["constructor_arguments"]["recipient"].lower(), self.quote["deposit_address"].lower())
        self.assertEqual(result["constructor_arguments"]["agent"], AGENT)
        self.assertEqual(result["constructor_arguments"]["asset"].lower(), a.TOKEN.lower())
        self.assertEqual(result["constructor_arguments"]["quoteHash"], "0x" + self.quote["quote_sha256"])
        self.assertEqual(result["constructor_arguments"]["programHash"], "0x" + self.program["program_sha256"])
        self.assertEqual(result["constructor_arguments"]["maxPerPayment"], a.AMOUNT)
        self.assertEqual(result["constructor_arguments"]["maxTotal"], a.AMOUNT)
        self.assertTrue(result["verified_historical_maker_signature"])
        self.assertFalse(result["signed"])
        self.assertFalse(result["deployed"])
        self.assertFalse(result["broadcast"])
        self.assertFalse(result["settlement_proven"])
        self.assertFalse(result["owner_signature_verified"])
        self.assertFalse(result["onchain_guard_observation_verified"])
        self.assertEqual(result["agent_order_template"]["amount"], a.AMOUNT)
        self.assertEqual(result["agent_order_template"]["recipient"].lower(), self.quote["deposit_address"].lower())
        self.assertEqual(result["agent_order_template"]["deadline"], result["constructor_arguments"]["expiresAt"])
        self.assertRegex(result["agent_order_template"]["nonce"], r"^0x[0-9a-f]{64}$")

    def test_expired_real_signed_quote_cannot_authorize_new_vault(self) -> None:
        with self.assertRaisesRegex(a.AdmissionError, "QUOTE_EXPIRED_OR_FUTURE"):
            self.preview(now=self.issued + timedelta(hours=3))

    def test_missing_or_wrong_explicit_owner_hash_rejected(self) -> None:
        for replacement in (None, "0" * 64):
            with self.subTest(replacement=replacement):
                self.owner_terms["approved_quote_sha256"] = replacement
                with self.assertRaises(a.AdmissionError):
                    self.preview()

    def test_tampered_maker_signed_fields_never_bind(self) -> None:
        for section, key, value in (
            ("quote", "depositAddress", "0x2222222222222222222222222222222222222222"),
            ("quoteRequest", "refundTo", "0x2222222222222222222222222222222222222222"),
            ("quoteRequest", "amount", "5000000"),
            ("quoteRequest", "recipient", "attacker.testnet"),
            ("quote", "deadline", "2027-01-01T00:00:00.000Z"),
        ):
            altered = deepcopy(self.response)
            altered[section][key] = value
            with self.subTest(key=key), self.assertRaisesRegex(a.AdmissionError, "MAKER_SIGNATURE_INVALID"):
                self.preview(response=altered)

    def test_owner_terms_must_pin_quote_deposit_refund_and_principals(self) -> None:
        for key, value in (
            ("deposit_address", "0x2222222222222222222222222222222222222222"),
            ("refund_to", "0x2222222222222222222222222222222222222222"),
            ("source_owner", "0x2222222222222222222222222222222222222222"),
            ("agent_signer", a.SOURCE),
            ("destination_account", "attacker.testnet"),
        ):
            original = self.owner_terms[key]
            self.owner_terms[key] = value
            with self.subTest(key=key), self.assertRaises(a.AdmissionError):
                self.preview()
            self.owner_terms[key] = original

    def test_owner_terms_require_exact_asset_chain_budget_and_program(self) -> None:
        for key, value in (
            ("origin_chain_id", 1),
            ("origin_chain_id", "84532"),
            ("token_contract", "0x2222222222222222222222222222222222222222"),
            ("token_code_sha256", "0" * 64),
            ("amount_base_units", a.AMOUNT - 1),
            ("max_per_payment", a.AMOUNT * 2),
            ("max_total", a.AMOUNT * 3),
            ("approved_program_sha256", "a" * 64),
        ):
            original = self.owner_terms[key]
            self.owner_terms[key] = value
            with self.subTest(key=key), self.assertRaises(a.AdmissionError):
                self.preview()
            self.owner_terms[key] = original

    def test_near_guard_mutations_fail_even_when_maker_signature_is_authentic(self) -> None:
        for name, change in (
            ("revoked", lambda row: row.update(revoked=True)),
            ("wrong_quote", lambda row: row.update(quote_sha256="0" * 64)),
            ("wrong_program", lambda row: row["policy"].update(program_sha256="e" * 64)),
            ("wrong_nonce", lambda row: row.update(nonce="replayed")),
            ("wrong_refund", lambda row: row["policy"].update(refund_to="0x" + "2" * 40)),
            ("wrong_recipient_minimum", lambda row: row["policy"].update(minimum_out="9999999")),
        ):
            old = self.guard_row
            self.guard_row = deepcopy(old)
            change(self.guard_row)
            with self.subTest(name=name), self.assertRaises(a.AdmissionError):
                self.preview()
            self.guard_row = old

    def test_no_vault_address_signature_or_transfer_calldata_is_created(self) -> None:
        result = self.preview()
        self.assertNotIn("signature", result)
        self.assertNotIn("private_key", json.dumps(result).lower())
        self.assertNotIn("transaction", result)
        self.assertNotIn("executePayment", result)
        self.assertNotIn("calldata", result)
        self.assertNotIn("deployed_vault", result)
        self.assertEqual(result["agent_order_template"]["verifyingContract"], "UNDEPLOYED_NO_EIP712_DIGEST")

    def test_historical_signed_quote_cli_is_denied_before_network_or_signing(self) -> None:
        import subprocess
        import tempfile
        with tempfile.TemporaryDirectory() as folder:
            base = Path(folder)
            quote_file = base / "historical-live-quote.json"
            quote_file.write_text(json.dumps({
                "classification": "LIVE_QUOTE_UNFUNDED_UNAUTHORIZED_NOT_SETTLED",
                "response": self.response,
            }))
            owner_file = base / "reviewer-terms.json"
            owner_file.write_text(json.dumps(self.owner_terms))
            run = subprocess.run([
                sys.executable,
                str(ROOT / "scripts/hackathon/base_vault_quote_binding.py"),
                "--quote-file", str(quote_file),
                "--reviewer-terms-file", str(owner_file),
                "--guard-account", "no-such-guard.testnet",
                "--guard-wasm-sha256", "0" * 64,
            ], text=True, capture_output=True, timeout=15)
            self.assertEqual(run.returncode, 2, run.stdout + run.stderr)
            output = json.loads(run.stderr)
            self.assertEqual(output["classification"], "HARD_DENIED_NO_SIGNING_OR_DEPLOYMENT")
            self.assertIn("QUOTE_EXPIRED_OR_FUTURE", output["reason"])
            self.assertIs(output["signed"], False)
            self.assertIs(output["broadcast"], False)

    def test_extra_and_missing_reviewer_terms_fail_closed(self) -> None:
        self.owner_terms["unreviewed_override"] = "skip"
        with self.assertRaisesRegex(a.AdmissionError, "EXACT_REVIEWER_TERMS_REQUIRED"):
            self.preview()
        del self.owner_terms["unreviewed_override"]
        del self.owner_terms["approved_quote_sha256"]
        with self.assertRaisesRegex(a.AdmissionError, "EXACT_REVIEWER_TERMS_REQUIRED"):
            self.preview()

    def test_timezone_aware_clock_required(self) -> None:
        with self.assertRaises(a.AdmissionError):
            self.preview(now=datetime(2026, 11, 7, 5, 10))


if __name__ == "__main__":
    unittest.main()
