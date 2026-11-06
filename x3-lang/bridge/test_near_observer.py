"""Fail-closed read-only NEAR escrow observation; no signing or chain proof."""
import base64
import json
import sqlite3
import unittest
import tempfile
from pathlib import Path
from near_observer import _strict_json_loads, observe, ObservationError, OutcomeJournal

SWAP = "ab" * 32
VAULT = "escrow.testnet"
RECIPIENT = "alice.testnet"
INITIATOR = "lp.testnet"
HASHLOCK = bytes(range(32))
AMOUNT = 10**24
BLOCK = "B" * 44
FUNDED_BEFORE_NS = 1899996400000000000
EXPIRY_NS = 1899999999000000000


def replies(state="Claimed", amount=AMOUNT, block=BLOCK, vault=VAULT,
            funded_before=FUNDED_BEFORE_NS):
    swap = {"initiator": INITIATOR, "recipient": RECIPIENT,
            "hashlock": list(HASHLOCK), "amount": str(amount),
            "funding_deadline_ns": funded_before,
            "expires_at_ns": EXPIRY_NS, "state": state}
    payload = json.dumps(swap).encode()
    return {
        "jsonrpc": "2.0", "id": "x3-near-observer", "result": {
            "block_hash": block, "block_height": 123456,
            "result": list(payload), "logs": []}}


def fake_network(results):
    calls = []
    def fetch(endpoint, payload):
        calls.append((endpoint, payload))
        return results[endpoint]
    return fetch, calls


class NearObserverTests(unittest.TestCase):
    def test_two_independent_endpoints_agree_on_final_state(self):
        fetch, calls = fake_network({"https://rpc-a.example": replies(), "https://rpc-b.example": replies()})
        observed = observe(["https://rpc-a.example", "https://rpc-b.example"],
            vault=VAULT, swap_id=SWAP, recipient=RECIPIENT,
            initiator=INITIATOR, hashlock=HASHLOCK, near_amount_yocto=AMOUNT,
            near_funded_before_ns=FUNDED_BEFORE_NS, near_expiry_ns=EXPIRY_NS,
            fetch=fetch)
        assert observed.outcome == "Claimed"
        assert observed.block_hash == BLOCK
        assert observed.authorized_to_sign is False
        assert all(p["params"]["finality"] == "final" for _, p in calls)
        assert all(p["params"]["account_id"] == VAULT for _, p in calls)



    def test_disagreement_or_invalid_state_refuses(self):
        for change in ("block", "amount", "recipient", "state", "missing", "logs"):
            with self.subTest(change=change):
                first = replies()
                second = replies()
                if change == "block":
                    second["result"]["block_hash"] = "C" * 44
                elif change == "amount":
                    second = replies(amount=AMOUNT + 1)
                elif change == "recipient":
                    raw = json.loads(bytes(second["result"]["result"]))
                    raw["recipient"] = "attacker.testnet"
                    second["result"]["result"] = list(json.dumps(raw).encode())
                elif change == "state":
                    second = replies(state="Refunded")
                elif change == "missing":
                    second["result"]["result"] = list(b"null")
                elif change == "logs":
                    second["result"]["logs"] = ["unexpected"]
                fetch, _ = fake_network({"https://rpc-a.example": first, "https://rpc-b.example": second})
                with self.assertRaises(ObservationError):
                    observe(["https://rpc-a.example", "https://rpc-b.example"],
                        vault=VAULT, swap_id=SWAP, recipient=RECIPIENT,
                        initiator=INITIATOR, hashlock=HASHLOCK, near_amount_yocto=AMOUNT,
                        near_funded_before_ns=FUNDED_BEFORE_NS,
                        near_expiry_ns=EXPIRY_NS, fetch=fetch)



    def test_same_hostname_does_not_count_as_independent(self):
        fetch, _ = fake_network({})
        with self.assertRaises(ObservationError):
            observe(["https://rpc-a.example", "https://rpc-a.example/alternate"],
                vault=VAULT, swap_id=SWAP, recipient=RECIPIENT,
                initiator=INITIATOR, hashlock=HASHLOCK, near_amount_yocto=AMOUNT,
                near_funded_before_ns=FUNDED_BEFORE_NS,
                near_expiry_ns=EXPIRY_NS, fetch=fetch)



    def test_wrong_hashlock_refuses(self):
        fetch, _ = fake_network({"https://rpc-a.example": replies(), "https://rpc-b.example": replies()})
        with self.assertRaises(ObservationError):
            observe(["https://rpc-a.example", "https://rpc-b.example"],
                vault=VAULT, swap_id=SWAP, recipient=RECIPIENT,
                initiator=INITIATOR, hashlock=b"x" * 32, near_amount_yocto=AMOUNT,
                near_funded_before_ns=FUNDED_BEFORE_NS,
                near_expiry_ns=EXPIRY_NS, fetch=fetch)

    def test_cross_chain_expiry_substitution_is_rejected(self):
        fetch, _ = fake_network({"https://rpc-a.example": replies(),
                                 "https://rpc-b.example": replies()})
        with self.assertRaises(ObservationError):
            observe(["https://rpc-a.example", "https://rpc-b.example"],
                    vault=VAULT, swap_id=SWAP, recipient=RECIPIENT,
                    initiator=INITIATOR, hashlock=HASHLOCK, near_amount_yocto=AMOUNT,
                    near_funded_before_ns=FUNDED_BEFORE_NS,
                    near_expiry_ns=EXPIRY_NS + 1, fetch=fetch)

    def test_cross_chain_funding_cutoff_substitution_is_rejected(self):
        fetch, _ = fake_network({"https://rpc-a.example": replies(),
                                 "https://rpc-b.example": replies()})
        with self.assertRaises(ObservationError):
            observe(["https://rpc-a.example", "https://rpc-b.example"],
                    vault=VAULT, swap_id=SWAP, recipient=RECIPIENT,
                    initiator=INITIATOR, hashlock=HASHLOCK, near_amount_yocto=AMOUNT,
                    near_funded_before_ns=FUNDED_BEFORE_NS + 1,
                    near_expiry_ns=EXPIRY_NS, fetch=fetch)

    def test_legacy_swap_without_registered_funding_cutoff_is_rejected(self):
        fetch, _ = fake_network({"https://rpc-a.example": replies(funded_before=0),
                                 "https://rpc-b.example": replies(funded_before=0)})
        with self.assertRaises(ObservationError):
            observe(["https://rpc-a.example", "https://rpc-b.example"],
                    vault=VAULT, swap_id=SWAP, recipient=RECIPIENT,
                    initiator=INITIATOR, hashlock=HASHLOCK, near_amount_yocto=AMOUNT,
                    near_funded_before_ns=FUNDED_BEFORE_NS,
                    near_expiry_ns=EXPIRY_NS, fetch=fetch)

    def test_duplicate_json_keys_are_rejected_before_interpretation(self):
        with self.assertRaises(ObservationError):
            _strict_json_loads(b'{"state":"Claimed","state":"Refunded"}')
        with self.assertRaises(ObservationError):
            _strict_json_loads(b'{"amount":NaN}')



    def test_persistent_conflicting_outcome_is_rejected(self):
        self.tmp_path = Path(tempfile.mkdtemp(prefix="x3-near-observer-"))
        self.addCleanup(lambda: __import__("shutil").rmtree(self.tmp_path))
        j = OutcomeJournal(self.tmp_path / "observer.sqlite")
        assert j.record("testnet", VAULT, SWAP, "Claimed", BLOCK) is True
        assert j.record("testnet", VAULT, SWAP, "Claimed", BLOCK) is False
        with self.assertRaises(ObservationError):
            j.record("testnet", VAULT, SWAP, "Refunded", BLOCK)
        assert OutcomeJournal(self.tmp_path / "observer.sqlite").get("testnet", VAULT, SWAP) == ("Claimed", BLOCK)



    def test_unverified_observation_cannot_enter_journal(self):
        self.tmp_path = Path(tempfile.mkdtemp(prefix="x3-near-observer-"))
        self.addCleanup(lambda: __import__("shutil").rmtree(self.tmp_path))
        j = OutcomeJournal(self.tmp_path / "observer.sqlite")
        with self.assertRaises(ObservationError):
            j.record("testnet", VAULT, SWAP, "OBSERVED", BLOCK)



    def test_locked_state_is_only_an_observation(self):
        fetch, _ = fake_network({"https://rpc-a.example": replies(state="Locked"),
                                 "https://rpc-b.example": replies(state="Locked")})
        o = observe(["https://rpc-a.example", "https://rpc-b.example"],
                    vault=VAULT, swap_id=SWAP, recipient=RECIPIENT,
                    initiator=INITIATOR, hashlock=HASHLOCK, near_amount_yocto=AMOUNT,
                    near_funded_before_ns=FUNDED_BEFORE_NS,
                    near_expiry_ns=EXPIRY_NS, fetch=fetch)
        assert o.outcome == "Locked" and not o.authorized_to_sign



    def test_rpc_error_is_never_accepted_as_unfunded(self):
        fetch, _ = fake_network({"https://rpc-a.example": replies(),
                                 "https://rpc-b.example": {"jsonrpc": "2.0", "id": "x3-near-observer",
                                                           "error": {"code": -32000, "message": "unknown account"}}})
        with self.assertRaises(ObservationError):
            observe(["https://rpc-a.example", "https://rpc-b.example"],
                    vault=VAULT, swap_id=SWAP, recipient=RECIPIENT,
                    initiator=INITIATOR, hashlock=HASHLOCK, near_amount_yocto=AMOUNT,
                    near_funded_before_ns=FUNDED_BEFORE_NS,
                    near_expiry_ns=EXPIRY_NS, fetch=fetch)



    def test_local_journal_does_not_allow_mutating_final_evidence(self):
        self.tmp_path = Path(tempfile.mkdtemp(prefix="x3-near-observer-"))
        self.addCleanup(lambda: __import__("shutil").rmtree(self.tmp_path))
        path = self.tmp_path / "durable.sqlite"
        a = OutcomeJournal(path)
        a.record("testnet", VAULT, SWAP, "Claimed", BLOCK)
        b = OutcomeJournal(path)
        with self.assertRaises(ObservationError):
            b.record("testnet", VAULT, SWAP, "Claimed", "D" * 44)
        assert b.get("testnet", VAULT, SWAP) == ("Claimed", BLOCK)

class FundingWindowObservationTests(unittest.TestCase):
    def test_closed_unfunded_window_is_read_only_not_authorized(self):
        from near_observer import observe_window
        raw = {"initiator": INITIATOR, "recipient": RECIPIENT,
               "hashlock": list(HASHLOCK), "amount": str(AMOUNT),
               "deadline_ns": 1800000000000000000, "expiry_ns": 1890000000000000000,
               "state": "NotFunded"}
        reply = replies()
        reply["result"]["result"] = list(json.dumps(raw).encode())
        fetch, calls = fake_network({"https://rpc-a.example": reply, "https://rpc-b.example": reply})
        o = observe_window(["https://rpc-a.example", "https://rpc-b.example"],
                           vault=VAULT, swap_id=SWAP, recipient=RECIPIENT, initiator=INITIATOR,
                           hashlock=HASHLOCK, near_amount_yocto=AMOUNT,
                           funding_deadline_ns=raw["deadline_ns"], expiry_ns=raw["expiry_ns"], fetch=fetch)
        self.assertEqual(o.outcome, "NotFunded")
        self.assertFalse(o.authorized_to_sign)
        self.assertEqual({p["params"]["method_name"] for _, p in calls}, {"get_window"})

    def test_unregistered_window_cannot_be_treated_as_not_funded(self):
        from near_observer import observe_window
        reply = replies()
        reply["result"]["result"] = list(b"null")
        fetch, _ = fake_network({"https://rpc-a.example": reply, "https://rpc-b.example": reply})
        with self.assertRaises(ObservationError):
            observe_window(["https://rpc-a.example", "https://rpc-b.example"],
                           vault=VAULT, swap_id=SWAP, recipient=RECIPIENT, initiator=INITIATOR,
                           hashlock=HASHLOCK, near_amount_yocto=AMOUNT,
                           funding_deadline_ns=1800000000000000000, expiry_ns=1890000000000000000,
                           fetch=fetch)
