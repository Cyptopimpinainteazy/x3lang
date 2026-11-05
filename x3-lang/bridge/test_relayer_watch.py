"""Read-only observer relayer tests. No fake chain state or financial execution."""
import tempfile
import unittest
from pathlib import Path

from relayer_watch import WatchJournal, WatchError

SWAP = "aa" * 32
OTHER = "bb" * 32


class WatchTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="x3-relayer-")
        self.addCleanup(self.tmp.cleanup)
        self.path = Path(self.tmp.name) / "relayer.db"
        self.j = WatchJournal(self.path)
        self.j.enqueue("testnet", "vault.testnet", SWAP, deadline_sec=5000)

    def test_restart_recovers_pending_work(self):
        first = self.j.lease("worker-a", now=1000, lease_sec=30)
        self.assertEqual([x.swap_id for x in first], [SWAP])
        self.assertEqual(WatchJournal(self.path).lease("worker-b", now=1010), [])
        recovered = WatchJournal(self.path).lease("worker-b", now=1031)
        self.assertEqual([x.swap_id for x in recovered], [SWAP])

    def test_lease_ownership_and_expiry_enforced(self):
        self.j.lease("worker-a", now=1000)
        with self.assertRaises(WatchError):
            self.j.record_observation("worker-b", "testnet", "vault.testnet", SWAP,
                                      state="Claimed", block_hash="block-a", now=1001)
        with self.assertRaises(WatchError):
            self.j.record_observation("worker-a", "testnet", "vault.testnet", SWAP,
                                      state="Claimed", block_hash="block-a", now=1061)

    def test_read_only_outcome_demands_manual_review(self):
        self.j.lease("worker-a", now=1000)
        self.j.record_observation("worker-a", "testnet", "vault.testnet", SWAP,
                                  state="Claimed", block_hash="block-a", now=1001)
        state = WatchJournal(self.path).get("testnet", "vault.testnet", SWAP)
        self.assertEqual(state.phase, "REVIEW_REQUIRED")
        self.assertFalse(state.authorized_to_sign)
        self.assertEqual(state.last_outcome, "Claimed")
        self.assertEqual(WatchJournal(self.path).lease("worker-b", now=1200), [])

    def test_no_lock_and_expiry_never_imply_refund(self):
        self.j.lease("worker-a", now=10000)
        self.j.fail("worker-a", "testnet", "vault.testnet", SWAP,
                    reason="missing swap or RPC error", now=10001)
        state = self.j.get("testnet", "vault.testnet", SWAP)
        self.assertEqual(state.phase, "PENDING")
        self.assertIsNone(state.last_outcome)
        self.assertFalse(state.authorized_to_sign)

    def test_failed_poll_retries_after_backoff_and_restart(self):
        self.j.lease("worker-a", now=1000)
        self.j.fail("worker-a", "testnet", "vault.testnet", SWAP,
                    reason="timeout", now=1001, retry_after_sec=60)
        reopened = WatchJournal(self.path)
        self.assertEqual(reopened.lease("worker-b", now=1059), [])
        self.assertEqual(len(reopened.lease("worker-b", now=1061)), 1)

    def test_replay_registration_idempotent_but_conflict_refused(self):
        self.assertFalse(self.j.enqueue("testnet", "vault.testnet", SWAP, deadline_sec=5000))
        with self.assertRaises(WatchError):
            self.j.enqueue("testnet", "vault.testnet", SWAP, deadline_sec=6000)

    def test_conflicting_terminal_observation_fail_closed(self):
        self.j.lease("worker-a", now=1000)
        self.j.record_observation("worker-a", "testnet", "vault.testnet", SWAP,
                                  state="Claimed", block_hash="block-a", now=1001)
        with self.assertRaises(WatchError):
            self.j.record_observation("worker-a", "testnet", "vault.testnet", SWAP,
                                      state="Refunded", block_hash="block-a", now=1002)

    def test_no_executable_states_or_external_endpoints(self):
        self.j.lease("worker-a", now=1000)
        with self.assertRaises(WatchError):
            self.j.record_observation("worker-a", "testnet", "vault.testnet", SWAP,
                                      state="NotFunded", block_hash="block-a", now=1001)
        with self.assertRaises(WatchError):
            self.j.enqueue("mainnet", "vault.testnet", OTHER, deadline_sec=5000)


if __name__ == "__main__":
    unittest.main()
