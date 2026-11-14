"""Issue #42: settlement encoding vector and exhaustive free-option model."""
import hashlib
import importlib.util
import sys
import unittest
from pathlib import Path


def _load(name: str, filename: str):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(filename))
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


enc = _load("x3_bridge_settlement", "settlement.py")
model = _load("x3_bridge_settlement_model", "settlement_model.py")

# Shared with near/src/lib.rs and bsc/test/NearSettledBnbEscrow.t.sol.
VECTOR = dict(
    bsc_chain_id=31337,
    escrow="0x00000000000000000000000000000000000E5C40",
    depositor="0x00000000000000000000000000000000000A11CE",
    lp="0x000000000000000000000000000000000000BEEF",
    bnb_wei=10**18,
    salt=bytes([0x11]) * 32,
)
HASHLOCK = hashlib.sha256(bytes([3]) * 32).digest()
WINDOW_TERMS = dict(
    vault="x3-vault.testnet",
    initiator="lp.testnet",
    recipient="alice.testnet",
    hashlock=HASHLOCK,
    near_yocto=10**24,
    funding_deadline_ns=1_700_000_600_000_000_000,
    expiry_ns=1_700_001_200_000_000_000,
)
EXPECTED = {
    "binding": "f9ceb35c05cbdd513aa7342a5ba33dd8538a9580668cea14017b5ef7f91d05aa",
    "window": "9d66ddfe92e56e993dd87e2f7ca53b87e39ceb35325cc44b404c0a8eb87de110",
    enc.Outcome.REGISTERED: "4b7b258b6db8ae6b3979c0ef505ecf625c0ab59727439d980ce017b01fa04808",
    enc.Outcome.CLAIMED: "c956f53937ecff906f42672b636a1b8216fd7beade6dd7eeaaa2074e52d9177e",
    enc.Outcome.REFUNDED: "561cc941f2d1e81c86d2d428753acae16de924e836c2d9b589dbbc4a303bbb17",
    enc.Outcome.NOT_FUNDED: "e440e5dd802c852009e6e7315b05e404404d8f6f861d4a2f114ee9014dde9dff",
}


class SettlementEncodingTests(unittest.TestCase):
    def window(self, **overrides):
        binding = enc.source_binding(**{**VECTOR, **overrides.pop("source", {})})
        return enc.window_id(**{**WINDOW_TERMS, **overrides}, binding=binding)

    def test_cross_language_vector(self):
        self.assertEqual(HASHLOCK.hex(), "648aa5c579fb30f38af744d97d6ec840c7a91277a499a0d780f3e7314eca090b")
        self.assertEqual(enc.source_binding(**VECTOR).hex(), EXPECTED["binding"])
        window = self.window()
        self.assertEqual(window.hex(), EXPECTED["window"])
        for outcome in enc.Outcome:
            self.assertEqual(enc.settlement_commitment(window, outcome).hex(), EXPECTED[outcome])
            self.assertEqual(enc.settlement_log_line(window, outcome),
                             "X3_NEAR_SETTLEMENT_V1 " + EXPECTED[outcome])

    def test_every_bound_term_changes_the_window_id(self):
        base = self.window()
        variants = [
            self.window(vault="x3-vault2.testnet"),
            self.window(initiator="lp2.testnet"),
            self.window(recipient="lp.testnet"),
            self.window(hashlock=bytes(32)),
            self.window(near_yocto=10**24 - 1),
            self.window(funding_deadline_ns=1_700_000_600_000_000_001),
            self.window(expiry_ns=1_700_001_200_000_000_001),
            self.window(source={"bsc_chain_id": 97}),
            self.window(source={"escrow": "0x00000000000000000000000000000000000E5C41"}),
            self.window(source={"depositor": "0x00000000000000000000000000000000000A11CF"}),
            self.window(source={"lp": "0x000000000000000000000000000000000000BEEE"}),
            self.window(source={"bnb_wei": 10**18 - 1}),
            self.window(source={"salt": bytes([0x12]) * 32}),
        ]
        self.assertEqual(len({v for v in variants} | {base}), len(variants) + 1)

    def test_outcomes_have_distinct_commitments_and_payees(self):
        window = self.window()
        commitments = {enc.settlement_commitment(window, o) for o in enc.Outcome}
        self.assertEqual(len(commitments), 4)
        self.assertEqual(enc.bsc_payee(enc.Outcome.CLAIMED), "lp")
        self.assertEqual(enc.bsc_payee(enc.Outcome.REFUNDED), "depositor")
        self.assertEqual(enc.bsc_payee(enc.Outcome.NOT_FUNDED), "depositor")
        with self.assertRaises(enc.SettlementEncodingError):
            enc.bsc_payee(enc.Outcome.REGISTERED)

    def test_malformed_inputs_rejected(self):
        bad = [
            lambda: enc.source_binding(**{**VECTOR, "escrow": "0x1234"}),
            lambda: enc.source_binding(**{**VECTOR, "salt": b"short"}),
            lambda: enc.source_binding(**{**VECTOR, "bnb_wei": 0}),
            lambda: enc.source_binding(**{**VECTOR, "bsc_chain_id": True}),
            lambda: self.window(recipient="Alice.testnet"),
            lambda: self.window(recipient="a" * 65),
            lambda: self.window(near_yocto=1 << 128),
            lambda: self.window(expiry_ns=-1),
            lambda: enc.settlement_commitment(b"\x00" * 31, enc.Outcome.CLAIMED),
            lambda: enc.settlement_commitment(b"\x00" * 32, 9),
        ]
        for fn in bad:
            with self.assertRaises((enc.SettlementEncodingError, ValueError)):
                fn()


class FreeOptionModelTests(unittest.TestCase):
    """Exhaustive AG EF model check; see settlement_model.py for scope."""

    def test_near_settled_rule_has_no_free_option_for_either_party(self):
        for honest in ("alice", "lp"):
            result = model.check("NEAR_SETTLED", honest)
            self.assertTrue(result.safe, (honest, result.trace, result.final_state))
            self.assertGreater(result.states, 100)

    def test_red_legacy_preimage_rule_leaks_bnb_on_failed_late_claim(self):
        result = model.check("LEGACY_HASHLOCK", "alice")
        self.assertFalse(result.safe)
        # Honest Alice reveals the preimage before NEAR expiry against FINAL
        # funding; the scheduler delays execution past expiry; LP takes BNB.
        self.assertIn("alice_submit_near_claim", result.trace)
        self.assertEqual(result.trace[-1], "lp_claim_bnb_with_preimage")
        self.assertEqual(result.final_state.bsc, "P")
        self.assertNotEqual(result.final_state.final_swap, "C")

    def test_red_legacy_preimage_rule_depends_on_lp_beating_bsc_expiry(self):
        result = model.check("LEGACY_HASHLOCK", "lp")
        self.assertFalse(result.safe)
        self.assertGreaterEqual(result.final_state.t, model.B)

    def test_red_owner_release_rule_lets_depositor_keep_both(self):
        self.assertTrue(model.check("OWNER_RELEASE", "alice").safe)
        result = model.check("OWNER_RELEASE", "lp")
        self.assertFalse(result.safe)
        self.assertEqual(result.trace[-1], "near_execute_claim")

    def test_every_mutant_of_the_withdrawal_condition_is_killed(self):
        for mutant in model.MUTANTS:
            caught = [h for h in ("alice", "lp") if not model.check("NEAR_SETTLED", h, mutant).safe]
            self.assertTrue(caught, f"mutant {mutant} survived the model")

    def test_rejects_unknown_rule_party_or_mutant(self):
        with self.assertRaises(ValueError):
            model.check("NOPE", "alice")
        with self.assertRaises(ValueError):
            model.check("NEAR_SETTLED", "bob")
        with self.assertRaises(ValueError):
            model.check("OWNER_RELEASE", "alice", "ACCEPT_PENDING")


if __name__ == "__main__":
    unittest.main()
