"""NEAR light-client verification against published reference vectors (issue #56).

Real data
---------
* fixtures/near-light-client-522f940/: byte-identical NEAR testnet and mainnet
  `next_light_client_block` views and `light_client_proof` results published
  by https://github.com/near/near-light-client at commit 522f940 (see
  PROVENANCE.json). The validator approvals in them are real Ed25519
  signatures; nothing in this file re-signs or edits them except where a test
  is explicitly a tamper test.
* fixtures/near_live_*_20261105.json: genuine protocol-88 NEAR testnet data
  already committed by PR #57.

Synthetic data (always labeled)
-------------------------------
* The checkpoint approval signature is a deterministic TEST key. It stands in
  for the offline checkpoint ceremony so the real validator transitions after
  the anchor can be checked. It is not, and is never presented as, NEAR data.
* Tests named `synthetic_*` sign made-up headers with made-up validator keys
  to reach succession rules no published real vector exercises.
"""
import base64
import copy
import hashlib
import json
import unittest
from pathlib import Path

from cryptography.exceptions import InvalidSignature
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PublicKey

import near_authenticated as na
from near_authenticated import (
    NETWORK_GENESIS,
    AuthenticatedLightClientError,
    advance_state,
    authenticated_head,
    build_checkpoint_candidate,
    checkpoint_checksum,
    initialize_from_documents,
    verify_authenticated_outcome,
    verify_authenticated_inclusion,
)
from near_merkle import compute_merkle_root, decode_hash, encode_hash, lite_block_hash
from near_outcome import (
    OutcomeProofError,
    outcome_leaf,
    partial_execution_status,
    verify_outcome_inclusion,
)
from near_validator_commitment import compute_next_bps_hash
from test_near_authenticated import (
    make_view,
    marker,
    policy,
    seal_checkpoint,
    seed,
    sha,
    sign_candidate,
    validator,
)

ROOT = Path(__file__).parent
REF = ROOT / "fixtures" / "near-light-client-522f940"
REF_COMMIT = "522f94003fbbedfdc1e7a7de6284f5a1d25d8134"
P27 = ROOT / "fixtures" / "near_live_checkpoint_candidate_p27_20261105.json"
GB = ROOT / "fixtures" / "near_live_light_client_update_20261105.json"
RECEIPT_PROOF = ROOT / "fixtures" / "near_live_receipt_proof_20261105.json"
RECEIPT = "3f9FCzqC8k75AGob2UrCxbKVqADRyv5M5vGfoLEXbZyu"
RECEIVER = "league-v7.near-post.testnet"
CEREMONY = ROOT.parent.parent / "evidence" / "near-light-client-ceremony-2026-11-06"

# Head block Merkle root the reference implementation pins for old.json/new.json:
# crates/protocol/src/experimental.rs `BLOCK_MERKLE_ROOT` at REF_COMMIT.
REFERENCE_HEAD_ROOT = "WWrLWbWHwSmjtTn5oBZPYgRCuCYn6fkYVa4yhPWNK4L"
# Stake totals the reference asserts for mainnet main_0.next_bps approving
# main_1: crates/protocol/src/lib.rs
# `test_next_invalid_signatures_stake_isnt_sufficient` at REF_COMMIT.
REFERENCE_MAINNET_TOTAL = 512915271547861520119028536348929
REFERENCE_MAINNET_APPROVED = 345140782903867823005444871054881

# TEST-ONLY checkpoint authority (synthetic). Never a NEAR key.
SYNTHETIC_CHECKPOINT_SIGNER_SEED = 120


def ref_body(name):
    return json.loads((REF / f"{name}.json").read_text())["body"]


def ref_document(name):
    return json.loads((REF / f"{name}.json").read_text())


def anchored(network, first_view):
    """Initialize from a real view using a SYNTHETIC checkpoint approval.

    current_validators is null: the producers of the anchor's own epoch are
    not committed by the anchor, so only next-epoch successors can verify.
    """
    signer = seed(SYNTHETIC_CHECKPOINT_SIGNER_SEED)
    trust_policy = policy(signer, network=network, genesis=NETWORK_GENESIS[network])
    checkpoint = build_checkpoint_candidate(
        first_view,
        None,
        network=network,
        genesis_hash=NETWORK_GENESIS[network],
        protocol_version=88,
        provenance={
            "source": f"near-light-client {REF_COMMIT} fixture (synthetic test approval)",
            "verified_by": "unit test; NOT an offline checkpoint ceremony",
            "observed_at": "2026-11-13T00:00:00Z",
        },
    )
    checkpoint, approval = seal_checkpoint(checkpoint, signer)
    state = initialize_from_documents(
        checkpoint,
        approval,
        trust_policy,
        expected_policy_sha256=sha(trust_policy),
        expected_checkpoint_sha256=checkpoint_checksum(checkpoint),
    )
    return state, checkpoint, approval, trust_policy


def approval_message_for(view):
    block_hash = lite_block_hash(view)
    next_hash = hashlib.sha256(decode_hash(view["next_block_inner_hash"]) + block_hash).digest()
    return na._approval_message(next_hash, view["inner_lite"]["height"])


def untagged_v1_bps_hash(validators):
    """Pre-BlockHeaderV3 layout (no ValidatorStake enum tag), for contrast only."""
    raw = bytearray(len(validators).to_bytes(4, "little"))
    for item in validators:
        account = item["account_id"].encode()
        raw += len(account).to_bytes(4, "little") + account
        raw += b"\x00" + decode_hash(item["public_key"].removeprefix("ed25519:"))
        raw += int(item["stake"]).to_bytes(16, "little")
    return encode_hash(hashlib.sha256(raw).digest())


class ReferenceVectorProvenanceTests(unittest.TestCase):
    def test_vendored_vectors_match_provenance_manifest(self):
        manifest = json.loads((REF / "PROVENANCE.json").read_text())
        self.assertEqual(manifest["upstream_commit"], REF_COMMIT)
        self.assertEqual(
            manifest["upstream_repository"], "https://github.com/near/near-light-client"
        )
        on_disk = {path.name for path in REF.iterdir()} - {"PROVENANCE.json", "README.md"}
        self.assertEqual(on_disk, set(manifest["files"]))
        for name, entry in manifest["files"].items():
            with self.subTest(name=name):
                digest = hashlib.sha256((REF / name).read_bytes()).hexdigest()
                self.assertEqual(digest, entry["sha256"])
                self.assertIn(f"/blob/{REF_COMMIT}/", entry["upstream_url"])


class ReferenceEpochTransitionTests(unittest.TestCase):
    def test_lite_header_hash_reproduces_reference_recorded_block_hashes(self):
        # Each fixture records the block hash it was requested from; our
        # Borsh(inner_lite) -> nested SHA-256 reconstruction must reproduce it.
        for previous, following in (
            ("main_0", "main_1"),
            ("main_1", "main_2"),
            ("test_1", "test_2"),
        ):
            with self.subTest(previous=previous):
                self.assertEqual(
                    encode_hash(lite_block_hash(ref_body(previous))),
                    ref_document(following)["last_block_hash"],
                )

    def test_versioned_next_bps_borsh_commitment_matches_all_real_headers(self):
        views = [ref_body(f"{net}_{i}") for net in ("test", "main") for i in range(3)]
        views += [
            json.loads(P27.read_text())["result"],
            json.loads(GB.read_text())["result"],
        ]
        for view in views:
            with self.subTest(height=view["inner_lite"]["height"]):
                self.assertEqual(
                    compute_next_bps_hash(view["next_bps"]),
                    view["inner_lite"]["next_bp_hash"],
                )
                # The untagged pre-BlockHeaderV3 layout must NOT match: the
                # commitment is version-specific and we compute the tagged one.
                self.assertNotEqual(
                    untagged_v1_bps_hash(view["next_bps"]),
                    view["inner_lite"]["next_bp_hash"],
                )

    def test_testnet_two_real_epoch_transitions_verify(self):
        state, _, _, _ = anchored("testnet", ref_body("test_0"))
        self.assertIsNone(state["current_validators"])
        expected = (
            (
                "test_1",
                "EAUfcEw9BMjf1TqwqprD5rcMXbGrXmjdmmihLWEYbigA",
                154697976,
                313761370981034522210318209131304,
                318991009359393716115611141242479,
            ),
            (
                "test_2",
                "ByF6ngJ7WB9ZyYpDH3BzoY3PAbCbcc3ejKRPC2zRXUrQ",
                154741176,
                229067331026768025112729161310159,
                316758986562967801801464971713584,
            ),
        )
        for name, block_hash, height, approved, total in expected:
            previous_next = state["head"]["inner_lite"]["next_epoch_id"]
            announced = state["next_validators"]
            state = advance_state(state, ref_body(name))
            head = authenticated_head(state)
            self.assertEqual((head.block_hash, head.height), (block_hash, height))
            self.assertEqual(head.epoch_id, previous_next)
            # The approving set is exactly the one committed one epoch earlier.
            self.assertEqual(state["current_validators"], announced)
            self.assertEqual((int(state["last_approved_stake"]), int(state["last_total_stake"])), (approved, total))
            self.assertGreater(approved * 3, total * 2)

    def test_mainnet_two_real_epoch_transitions_match_reference_stake(self):
        state, _, _, _ = anchored("mainnet", ref_body("main_0"))
        state = advance_state(state, ref_body("main_1"))
        self.assertEqual(state["head_hash"], "Doy7Y7aVMgN8YhdAseGBMHNmYoqzWsXszqJ7MFLNMcQ7")
        self.assertEqual(int(state["last_total_stake"]), REFERENCE_MAINNET_TOTAL)
        self.assertEqual(int(state["last_approved_stake"]), REFERENCE_MAINNET_APPROVED)
        state = advance_state(state, ref_body("main_2"))
        self.assertEqual(state["head_hash"], "3tyxRRBgbYTo5DYd1LpX3EZtEiRYbDAAji6kcsf9QRge")
        self.assertEqual(
            (int(state["last_approved_stake"]), int(state["last_total_stake"])),
            (
                357397122544680318120500578206929,
                512311283258603581345127178847439,
            ),
        )

    def test_mainnet_shared_consensus_key_counts_per_slot(self):
        bps = ref_body("main_0")["next_bps"]
        approvals = ref_body("main_1")["approvals_after_next"]
        self.assertEqual(
            (bps[17]["account_id"], bps[42]["account_id"]),
            ("nearcrowd.poolv1.near", "nc2.poolv1.near"),
        )
        self.assertEqual(bps[17]["public_key"], bps[42]["public_key"])
        # Deterministic Ed25519: one key over one message gives one signature.
        self.assertEqual(approvals[17], approvals[42])
        message = approval_message_for(ref_body("main_1"))
        approved, total = na._verify_approvals(bps, approvals, message)
        self.assertEqual((approved, total), (REFERENCE_MAINNET_APPROVED, REFERENCE_MAINNET_TOTAL))
        signed = sum(int(item["stake"]) for item, sig in zip(bps, approvals) if sig is not None)
        self.assertEqual(signed, approved)
        # The second slot is load-bearing: without it this real mainnet block
        # is not final. A verifier that rejects shared keys cannot follow it.
        without_42 = list(approvals)
        without_42[42] = None
        with self.assertRaisesRegex(AuthenticatedLightClientError, "two-thirds"):
            na._verify_approvals(bps, without_42, message)
        self.assertLessEqual((approved - int(bps[42]["stake"])) * 3, total * 2)

    def test_full_stake_denominator_rejects_truncated_approval_vector(self):
        bps = ref_body("main_0")["next_bps"]
        view = ref_body("main_1")
        message = approval_message_for(view)
        truncated = view["approvals_after_next"][:1]
        self.assertIsNotNone(truncated[0])
        # A zip()-style count (denominator = only slots present) would accept:
        # the single present slot is 100% of the slots it covers.
        self.assertGreater(int(bps[0]["stake"]) * 3, int(bps[0]["stake"]) * 2)
        with self.assertRaisesRegex(AuthenticatedLightClientError, "two-thirds"):
            na._verify_approvals(bps, truncated, message)
        state, _, _, _ = anchored("mainnet", ref_body("main_0"))
        candidate = copy.deepcopy(view)
        candidate["approvals_after_next"] = truncated
        with self.assertRaisesRegex(AuthenticatedLightClientError, "two-thirds"):
            advance_state(state, candidate)

    def test_dropping_one_real_signature_falls_to_two_thirds_or_below(self):
        state, _, _, _ = anchored("mainnet", ref_body("main_0"))
        candidate = ref_body("main_1")
        bps = ref_body("main_0")["next_bps"]
        largest = max(
            (int(item["stake"]), index)
            for index, item in enumerate(bps)
            if candidate["approvals_after_next"][index] is not None
        )[1]
        candidate["approvals_after_next"][largest] = None
        with self.assertRaisesRegex(AuthenticatedLightClientError, "two-thirds"):
            advance_state(state, candidate)

    def test_trailing_approval_slots_beyond_epoch_producers_are_ignored(self):
        # Real testnet test_1 carries 33 slots for 32 producers (nearcore
        # appends next-epoch approvers); the Nomicon zip() ignores the extra.
        view = ref_body("test_1")
        self.assertEqual(len(ref_body("test_0")["next_bps"]), 32)
        self.assertEqual(len(view["approvals_after_next"]), 33)
        state, _, _, _ = anchored("testnet", ref_body("test_0"))
        self.assertEqual(authenticated_head(advance_state(state, view)).height, 154697976)

    def test_approval_message_is_endorsement_tag_next_hash_and_height_plus_two(self):
        view = ref_body("test_1")
        producer = ref_body("test_0")["next_bps"][0]
        signature = na._signature_bytes(view["approvals_after_next"][0], "approval")
        public = Ed25519PublicKey.from_public_bytes(
            na._key_bytes(producer["public_key"], "key")
        )
        message = approval_message_for(view)
        self.assertEqual(len(message), 1 + 32 + 8)
        self.assertEqual(message[0], 0)  # ApprovalInner::Endorsement
        public.verify(signature, message)
        height = view["inner_lite"]["height"]
        body = message[1:33]
        wrong = (
            b"\x01" + body + (height + 2).to_bytes(8, "little"),  # Skip tag
            b"\x00" + body + (height + 1).to_bytes(8, "little"),
            b"\x00" + body + (height + 3).to_bytes(8, "little"),
            b"\x00" + body + (height + 2).to_bytes(8, "big"),
            b"\x00" + lite_block_hash(view) + (height + 2).to_bytes(8, "little"),
        )
        for candidate in wrong:
            with self.subTest(message=candidate.hex()):
                with self.assertRaises(InvalidSignature):
                    public.verify(signature, candidate)


class ReferenceRejectionTests(unittest.TestCase):
    def test_skipping_an_epoch_is_rejected(self):
        state, _, _, _ = anchored("testnet", ref_body("test_0"))
        with self.assertRaisesRegex(AuthenticatedLightClientError, "does not succeed"):
            advance_state(state, ref_body("test_2"))

    def test_replayed_and_stale_real_updates_are_rejected(self):
        state, _, _, _ = anchored("testnet", ref_body("test_0"))
        first = advance_state(state, ref_body("test_1"))
        second = advance_state(first, ref_body("test_2"))
        for current, replay in (
            (first, "test_1"),
            (second, "test_1"),
            (second, "test_2"),
            (second, "test_0"),
        ):
            with self.subTest(replay=replay, height=current["head"]["inner_lite"]["height"]):
                with self.assertRaisesRegex(AuthenticatedLightClientError, "advance"):
                    advance_state(current, ref_body(replay))

    def test_mainnet_and_testnet_updates_cannot_cross(self):
        testnet, _, _, _ = anchored("testnet", ref_body("test_0"))
        mainnet, _, _, _ = anchored("mainnet", ref_body("main_0"))
        with self.assertRaises(AuthenticatedLightClientError):
            advance_state(testnet, ref_body("main_1"))
        with self.assertRaises(AuthenticatedLightClientError):
            advance_state(mainnet, ref_body("test_1"))
        with self.assertRaisesRegex(AuthenticatedLightClientError, "candidate network"):
            advance_state(testnet, {"network": "mainnet", "result": ref_body("test_1")})

    def test_network_genesis_pairing_is_enforced(self):
        signer = seed(SYNTHETIC_CHECKPOINT_SIGNER_SEED)
        for network, genesis in (
            ("testnet", NETWORK_GENESIS["mainnet"]),
            ("mainnet", NETWORK_GENESIS["testnet"]),
            ("testnet", marker("lookalike-genesis")),
        ):
            with self.subTest(network=network, genesis=genesis):
                trust_policy = policy(signer, network=network, genesis=genesis)
                with self.assertRaisesRegex(AuthenticatedLightClientError, "pinned NEAR"):
                    na.trust_policy_checksum(trust_policy)
                with self.assertRaisesRegex(AuthenticatedLightClientError, "pinned NEAR"):
                    build_checkpoint_candidate(
                        ref_body("test_0"),
                        None,
                        network=network,
                        genesis_hash=genesis,
                        protocol_version=88,
                        provenance={"source": "s", "verified_by": "v", "observed_at": "t"},
                    )

    def test_mainnet_checkpoint_under_testnet_policy_is_rejected(self):
        _, checkpoint, approval, _ = anchored("mainnet", ref_body("main_0"))
        testnet_policy = policy(
            seed(SYNTHETIC_CHECKPOINT_SIGNER_SEED),
            network="testnet",
            genesis=NETWORK_GENESIS["testnet"],
        )
        with self.assertRaisesRegex(AuthenticatedLightClientError, "network"):
            initialize_from_documents(
                checkpoint,
                approval,
                testnet_policy,
                expected_policy_sha256=sha(testnet_policy),
                expected_checkpoint_sha256=checkpoint_checksum(checkpoint),
            )

    def test_checkpoint_substitution_with_forged_validator_key_is_rejected(self):
        _, checkpoint, approval, trust_policy = anchored("testnet", ref_body("test_0"))
        pinned = checkpoint_checksum(checkpoint)
        forged = copy.deepcopy(checkpoint)
        attacker = validator(seed(121), "attacker.testnet", "1")["public_key"]
        forged["light_client_block"]["next_bps"][0]["public_key"] = attacker
        with self.assertRaises(AuthenticatedLightClientError):
            na.initialize_from_documents(
                forged, approval, trust_policy,
                expected_policy_sha256=sha(trust_policy),
                expected_checkpoint_sha256=pinned,
            )
        # Recommit the forged set and reseal: the content is self-consistent,
        # but it is not the externally pinned checkpoint and its approval
        # signature covers the original checksum.
        view = forged["light_client_block"]
        view["inner_lite"]["next_bp_hash"] = compute_next_bps_hash(view["next_bps"])
        forged["validator_commitments"]["next_bp_hash"] = view["inner_lite"]["next_bp_hash"]
        forged["block_hash"] = encode_hash(lite_block_hash(view))
        forged = na.seal_checkpoint_document(forged)
        for expected in (pinned, forged["checksum"]):
            with self.subTest(expected=expected):
                with self.assertRaises(AuthenticatedLightClientError):
                    na.initialize_from_documents(
                        forged, approval, trust_policy,
                        expected_policy_sha256=sha(trust_policy),
                        expected_checkpoint_sha256=expected,
                    )

    def test_forged_keys_in_candidate_are_rejected(self):
        state, _, _, _ = anchored("testnet", ref_body("test_0"))
        attacker = validator(seed(122), "attacker.testnet", "1")["public_key"]
        substituted = ref_body("test_1")
        substituted["next_bps"][3]["public_key"] = attacker
        with self.assertRaisesRegex(AuthenticatedLightClientError, "commitment"):
            advance_state(state, substituted)
        recommitted = copy.deepcopy(substituted)
        recommitted["inner_lite"]["next_bp_hash"] = compute_next_bps_hash(
            recommitted["next_bps"]
        )
        with self.assertRaisesRegex(AuthenticatedLightClientError, "invalid validator approval"):
            advance_state(state, recommitted)

    def test_tampered_authenticated_validator_state_is_rejected(self):
        state, _, _, _ = anchored("testnet", ref_body("test_0"))
        attacker = validator(seed(123), "attacker.testnet", "1")["public_key"]
        tampered = copy.deepcopy(state)
        tampered["next_validators"][0]["public_key"] = attacker
        with self.assertRaises(AuthenticatedLightClientError):
            advance_state(tampered, ref_body("test_1"))
        tampered["next_validators_hash"] = compute_next_bps_hash(tampered["next_validators"])
        tampered["state_sha256"] = na._sha256(
            na._canonical(na._without_checksum(tampered, "state_sha256"))
        )
        with self.assertRaisesRegex(AuthenticatedLightClientError, "do not match authenticated head"):
            advance_state(tampered, ref_body("test_1"))

    def test_encoding_attacks_on_real_update_are_rejected(self):
        state, _, _, _ = anchored("testnet", ref_body("test_0"))
        mutations = {
            "inner-lite V2 field": lambda v: v["inner_lite"].__setitem__(
                "chunk_execution_root", marker("spice")
            ),
            "unknown inner_lite field": lambda v: v["inner_lite"].__setitem__(
                "prev_outcome_root", marker("x")
            ),
            "unknown view field": lambda v: v.__setitem__("approvals", []),
            "timestamp_nanosec disagrees": lambda v: v["inner_lite"].__setitem__(
                "timestamp_nanosec", str(v["inner_lite"]["timestamp"] + 1)
            ),
            "timestamp_nanosec not a string": lambda v: v["inner_lite"].__setitem__(
                "timestamp_nanosec", v["inner_lite"]["timestamp"]
            ),
            "boolean height": lambda v: v["inner_lite"].__setitem__("height", True),
            "float height": lambda v: v["inner_lite"].__setitem__(
                "height", float(v["inner_lite"]["height"])
            ),
            "string height": lambda v: v["inner_lite"].__setitem__(
                "height", str(v["inner_lite"]["height"])
            ),
            "approvals not a list": lambda v: v.__setitem__("approvals_after_next", {}),
            "integer approval": lambda v: v["approvals_after_next"].__setitem__(0, 1),
            "malformed ignored trailing slot": lambda v: v["approvals_after_next"].__setitem__(
                32, "ed25519:0OIl"
            ),
            "secp256k1 approval": lambda v: v["approvals_after_next"].__setitem__(
                0, v["approvals_after_next"][0].replace("ed25519:", "secp256k1:")
            ),
            "secp256k1 next producer": lambda v: v["next_bps"][0].__setitem__(
                "public_key", v["next_bps"][0]["public_key"].replace("ed25519:", "secp256k1:")
            ),
            "unknown stake version": lambda v: v["next_bps"][0].__setitem__(
                "validator_stake_struct_version", "V2"
            ),
            "zero stake": lambda v: v["next_bps"][0].__setitem__("stake", "0"),
            "leading-zero stake": lambda v: v["next_bps"][0].__setitem__(
                "stake", "0" + v["next_bps"][0]["stake"]
            ),
            "duplicate account": lambda v: v["next_bps"][1].__setitem__(
                "account_id", v["next_bps"][0]["account_id"]
            ),
            "non-canonical base58 hash": lambda v: v.__setitem__(
                "prev_block_hash", "1" + v["prev_block_hash"]
            ),
            "next_bps null across epoch": lambda v: v.__setitem__("next_bps", None),
        }
        for label, mutate in mutations.items():
            with self.subTest(attack=label):
                candidate = ref_body("test_1")
                mutate(candidate)
                with self.assertRaises((AuthenticatedLightClientError, ValueError)):
                    advance_state(state, candidate)
        self.assertEqual(authenticated_head(advance_state(state, ref_body("test_1"))).height, 154697976)

    def test_reference_lossy_timestamp_vector_is_rejected_not_rehashed(self):
        # The reference repo's inline SuccessReceiptId vector carries a JSON
        # `timestamp` rounded by float parsing (…535000) next to the exact
        # `timestamp_nanosec` string (…535094). Only the exact value hashes to
        # the outcome's block hash; the verifier refuses the inconsistent view
        # instead of silently picking one.
        proof = json.loads((REF / "protocol_lib_rs_test_outcome_root.json").read_text())
        lite = proof["block_header_lite"]
        with self.assertRaisesRegex(ValueError, "timestamp_nanosec disagree"):
            lite_block_hash(lite)
        exact = copy.deepcopy(lite)
        exact["inner_lite"]["timestamp"] = int(exact["inner_lite"]["timestamp_nanosec"])
        self.assertEqual(
            encode_hash(lite_block_hash(exact)), proof["outcome_proof"]["block_hash"]
        )


class SyntheticSuccessionTests(unittest.TestCase):
    """SYNTHETIC headers and keys: succession rules absent from real vectors."""

    def setUp(self):
        self.keys = [seed(i) for i in (130, 131, 132)]
        self.current = [validator(key, f"cur{i}.testnet") for i, key in enumerate(self.keys)]
        self.next_keys = [seed(i) for i in (133, 134, 135)]
        self.future = [validator(key, f"nxt{i}.testnet") for i, key in enumerate(self.next_keys)]
        self.e0, self.e1, self.e2 = marker("syn-e0"), marker("syn-e1"), marker("syn-e2")
        anchor = make_view(height=1000, epoch=self.e0, next_epoch=self.e1,
                           next_validators=self.future, label="syn-anchor")
        signer = seed(136)
        trust_policy = policy(signer)
        checkpoint = na.checkpoint_document(
            network="testnet", genesis_hash=NETWORK_GENESIS["testnet"], protocol_version=88,
            view=anchor, current_validators=self.current,
            provenance={"source": "synthetic", "verified_by": "test", "observed_at": "t"},
        )
        checkpoint, approval = seal_checkpoint(checkpoint, signer)
        self.state = initialize_from_documents(
            checkpoint, approval, trust_policy,
            expected_policy_sha256=sha(trust_policy),
            expected_checkpoint_sha256=checkpoint_checksum(checkpoint),
        )

    def same_epoch(self, label, **changes):
        view = make_view(height=1010, epoch=self.e0, next_epoch=self.e1,
                         next_validators=self.future, label=label)
        view["inner_lite"].update(changes)
        return sign_candidate(view, self.current, self.keys)

    def test_synthetic_valid_same_epoch_successor_is_accepted(self):
        self.assertEqual(authenticated_head(advance_state(self.state, self.same_epoch("ok"))).height, 1010)

    def test_synthetic_timestamp_regression_is_rejected(self):
        anchor_time = self.state["head"]["inner_lite"]["timestamp"]
        for timestamp in (anchor_time, anchor_time - 1):
            with self.subTest(timestamp=timestamp):
                candidate = self.same_epoch(f"time-{timestamp}", timestamp=timestamp)
                with self.assertRaisesRegex(AuthenticatedLightClientError, "timestamp"):
                    advance_state(self.state, candidate)

    def test_synthetic_conflicting_next_validator_commitment_in_same_epoch(self):
        other = [validator(seed(137), "rogue.testnet")]
        view = make_view(height=1010, epoch=self.e0, next_epoch=self.e1,
                         next_validators=other, label="conflict-bps")
        candidate = sign_candidate(view, self.current, self.keys)
        with self.assertRaisesRegex(AuthenticatedLightClientError, "conflicting next validator"):
            advance_state(self.state, candidate)
        candidate = self.same_epoch("conflict-epoch", next_epoch_id=marker("syn-fork"))
        with self.assertRaisesRegex(AuthenticatedLightClientError, "conflicting next epoch"):
            advance_state(self.state, candidate)

    def test_synthetic_epoch_transition_cannot_loop_back(self):
        view = make_view(height=1100, epoch=self.e1, next_epoch=self.e0,
                         next_validators=self.current, label="loop")
        candidate = sign_candidate(view, self.future, self.next_keys)
        with self.assertRaisesRegex(AuthenticatedLightClientError, "loops back"):
            advance_state(self.state, candidate)
        view = make_view(height=1100, epoch=self.e1, next_epoch=self.e2,
                         next_validators=self.current, label="forward")
        self.assertEqual(
            authenticated_head(advance_state(self.state, sign_candidate(view, self.future, self.next_keys))).epoch_id,
            self.e1,
        )

    def test_synthetic_current_epoch_signers_cannot_approve_next_epoch(self):
        view = make_view(height=1100, epoch=self.e1, next_epoch=self.e2,
                         next_validators=self.current, label="wrong-set")
        candidate = sign_candidate(view, self.current, self.keys)
        with self.assertRaisesRegex(AuthenticatedLightClientError, "invalid validator approval"):
            advance_state(self.state, candidate)

    def test_real_anchor_without_current_set_refuses_same_epoch_successor(self):
        state, _, _, _ = anchored("testnet", ref_body("test_0"))
        view = copy.deepcopy(ref_body("test_0"))
        view["inner_lite"]["height"] += 1
        view["inner_lite"]["timestamp"] += 1
        view["inner_lite"]["timestamp_nanosec"] = str(view["inner_lite"]["timestamp"])
        with self.assertRaisesRegex(AuthenticatedLightClientError, "not authenticated"):
            advance_state(state, view)

    def test_cli_builds_checkpoint_without_current_validators(self):
        import contextlib
        import io
        import tempfile

        with tempfile.TemporaryDirectory(prefix="x3-lc-cli-") as tmp:
            out = Path(tmp) / "checkpoint.candidate.json"
            candidate = Path(tmp) / "test_0.response.json"
            candidate.write_text(json.dumps({"result": ref_body("test_0")}))
            common = [
                "build-checkpoint", "--candidate", str(candidate),
                "--network", "testnet", "--genesis-hash", NETWORK_GENESIS["testnet"],
                "--protocol-version", "88", "--source", "near-light-client 522f940 test_0",
                "--verified-by", "unit test", "--observed-at", "2026-11-13T00:00:00Z",
                "--out", str(out),
            ]
            with contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(na.main(common + ["--no-current-validators"]), 0)
            built = json.loads(out.read_text())
            self.assertIsNone(built["current_validators"])
            self.assertIsNone(built["validator_commitments"]["current_validators_hash"])
            self.assertEqual(built["block_hash"], "7iGV8NXZisEynt8jwuWZWurE9xDSXUKJr2xgWmaNGcvn")
            self.assertNotIn("checksum", built)
            mismatched = list(common)
            mismatched[mismatched.index("testnet")] = "mainnet"
            with contextlib.redirect_stderr(io.StringIO()) as err:
                self.assertEqual(na.main(mismatched + ["--no-current-validators"]), 2)
            self.assertIn("pinned NEAR mainnet genesis", err.getvalue())

    def test_checkpoint_with_no_validator_set_at_all_is_refused(self):
        view = make_view(height=5, epoch=self.e0, next_epoch=self.e1,
                         next_validators=None, label="empty")
        with self.assertRaisesRegex(AuthenticatedLightClientError, "no validator set"):
            build_checkpoint_candidate(
                view, None, network="testnet", genesis_hash=NETWORK_GENESIS["testnet"],
                protocol_version=88,
                provenance={"source": "s", "verified_by": "v", "observed_at": "t"},
            )


class OutcomeProofVectorTests(unittest.TestCase):
    def reference_root(self):
        return decode_hash(REFERENCE_HEAD_ROOT)

    def test_reference_success_value_proofs_reach_reference_head_root(self):
        expected = {
            "old": ("7LAgw7ETir15YXcKdbSuxLUZ1YNUpcDmKHjqxxAhpZ6U", 141429413),
            "new": ("j9XRxXGNhwh8GYAdEqs5BAGPaBpPFCgEDb7bUuTauu6", 141429462),
        }
        for name, (outcome_id, height) in expected.items():
            with self.subTest(vector=name):
                proof = ref_document(name)
                included = verify_outcome_inclusion(proof, head_block_merkle_root=self.reference_root())
                self.assertEqual(included.outcome_id, outcome_id)
                self.assertEqual(included.executor_id, "cdk.topgunbakugo.testnet")
                self.assertEqual(included.status, "SuccessValue")
                self.assertEqual(included.block_hash, proof["outcome_proof"]["block_hash"])
                self.assertEqual(
                    included.block_outcome_root,
                    proof["block_header_lite"]["inner_lite"]["outcome_root"],
                )
                self.assertEqual(proof["block_header_lite"]["inner_lite"]["height"], height)
                self.assertFalse(included.authenticated)

    def test_reference_success_receipt_id_outcome_root_path(self):
        proof = json.loads((REF / "protocol_lib_rs_test_outcome_root.json").read_text())
        status = proof["outcome_proof"]["outcome"]["status"]
        kind, encoded = partial_execution_status(status)
        self.assertEqual(kind, "SuccessReceiptId")
        self.assertEqual(encoded, b"\x03" + decode_hash(status["SuccessReceiptId"]))
        shard_root = compute_merkle_root(
            outcome_leaf(proof["outcome_proof"]), proof["outcome_proof"]["proof"]
        )
        block_root = compute_merkle_root(
            hashlib.sha256(shard_root).digest(), proof["outcome_root_proof"]
        )
        self.assertEqual(
            encode_hash(block_root), proof["block_header_lite"]["inner_lite"]["outcome_root"]
        )
        # The complete verifier refuses it because of the lossy timestamp.
        with self.assertRaises(OutcomeProofError):
            verify_outcome_inclusion(proof, head_block_merkle_root=block_root)

    def test_partial_execution_status_borsh_variants(self):
        self.assertEqual(partial_execution_status("Unknown"), ("Unknown", b"\x00"))
        detail_a = {"Failure": {"ActionError": {"index": 0, "kind": {"FunctionCallError": {}}}}}
        detail_b = {"Failure": {"InvalidTxError": "Expired"}}
        self.assertEqual(partial_execution_status(detail_a), ("Failure", b"\x01"))
        self.assertEqual(partial_execution_status(detail_b), ("Failure", b"\x01"))
        self.assertEqual(
            partial_execution_status({"SuccessValue": ""}), ("SuccessValue", b"\x02" + b"\0" * 4)
        )
        self.assertEqual(
            partial_execution_status({"SuccessValue": "AQI="}),
            ("SuccessValue", b"\x02\x02\x00\x00\x00\x01\x02"),
        )
        for bad in ("Failure", {"Failure": "x"}, {"SuccessValue": "AQJ="}, {"SuccessValue": "AQI"},
                    {"SuccessValue": 1}, {"Pending": None}, {}, {"SuccessValue": "", "Unknown": 0},
                    {"SuccessReceiptId": "short"}):
            with self.subTest(status=bad):
                with self.assertRaises((OutcomeProofError, ValueError)):
                    partial_execution_status(bad)

    def test_reference_proof_tampering_is_rejected(self):
        root = self.reference_root()
        tampers = {
            "status Failure": lambda p: p["outcome_proof"]["outcome"].__setitem__(
                "status", {"Failure": {"ActionError": {"index": 0, "kind": {}}}}
            ),
            "status Unknown": lambda p: p["outcome_proof"]["outcome"].__setitem__("status", "Unknown"),
            "status ReceiptId": lambda p: p["outcome_proof"]["outcome"].__setitem__(
                "status", {"SuccessReceiptId": p["outcome_proof"]["id"]}
            ),
            "return value": lambda p: p["outcome_proof"]["outcome"].__setitem__(
                "status", {"SuccessValue": base64.b64encode(b"141429999").decode()}
            ),
            "gas": lambda p: p["outcome_proof"]["outcome"].__setitem__(
                "gas_burnt", p["outcome_proof"]["outcome"]["gas_burnt"] + 1
            ),
            "tokens": lambda p: p["outcome_proof"]["outcome"].__setitem__(
                "tokens_burnt", str(int(p["outcome_proof"]["outcome"]["tokens_burnt"]) + 1)
            ),
            "executor": lambda p: p["outcome_proof"]["outcome"].__setitem__(
                "executor_id", "attacker.testnet"
            ),
            "receipt ids": lambda p: p["outcome_proof"]["outcome"]["receipt_ids"].append(
                p["outcome_proof"]["id"]
            ),
            "logs": lambda p: p["outcome_proof"]["outcome"]["logs"].append("forged"),
            "outcome id": lambda p: p["outcome_proof"].__setitem__("id", marker("other-id")),
            "outcome path direction": lambda p: p["outcome_proof"]["proof"][0].__setitem__(
                "direction", "Left" if p["outcome_proof"]["proof"][0]["direction"] == "Right" else "Right"
            ),
            "outcome root path sibling": lambda p: p["outcome_root_proof"][0].__setitem__(
                "hash", marker("sibling")
            ),
            "header outcome root": lambda p: p["block_header_lite"]["inner_lite"].__setitem__(
                "outcome_root", marker("outcome-root")
            ),
            "header inner rest": lambda p: p["block_header_lite"].__setitem__(
                "inner_rest_hash", marker("inner-rest")
            ),
            "block proof truncated": lambda p: p["block_proof"].pop(),
            "block proof direction": lambda p: p["block_proof"][0].__setitem__(
                "direction", "Right" if p["block_proof"][0]["direction"] == "Left" else "Left"
            ),
            "extra result field": lambda p: p.__setitem__("head", REFERENCE_HEAD_ROOT),
            "extra outcome_proof field": lambda p: p["outcome_proof"].__setitem__("trusted", True),
            "extra header field": lambda p: p["block_header_lite"].__setitem__("hash", marker("h")),
            "V2 header": lambda p: p["block_header_lite"]["inner_lite"].__setitem__(
                "chunk_execution_root", marker("spice")
            ),
            "unknown outcome field": lambda p: p["outcome_proof"]["outcome"].__setitem__(
                "authorized", True
            ),
        }
        self.assertEqual(
            verify_outcome_inclusion(ref_document("old"), head_block_merkle_root=root).status,
            "SuccessValue",
        )
        for label, tamper in tampers.items():
            with self.subTest(tamper=label):
                proof = ref_document("old")
                tamper(proof)
                with self.assertRaises(OutcomeProofError):
                    verify_outcome_inclusion(proof, head_block_merkle_root=root)
        with self.assertRaises(OutcomeProofError):
            verify_outcome_inclusion(ref_document("old"), head_block_merkle_root=decode_hash(marker("root")))

    def test_genuine_receipt_outcome_requires_the_head_its_proof_was_built_for(self):
        response = json.loads(RECEIPT_PROOF.read_text())
        # Real chain: p27 anchored with its own producers unknown (no synthetic
        # validator set), Gb approved by the 20 real producers p27 committed.
        p27 = json.loads(P27.read_text())["result"]
        state, _, _, _ = anchored("testnet", p27)
        real_gb = advance_state(state, json.loads(GB.read_text()))
        self.assertEqual(real_gb["head_hash"], "4gugmCPdbFQPhfo6YS9PVQaQAfkxpgeaAMwCNPa1akj2")
        # The committed proof was requested for light_client_head V7Kc...
        # (height 276418680), so its block_proof ends at that head's root, not
        # Gb's. It must NOT verify against the real authenticated Gb head.
        # Closing this needs a live light_client_proof built for Gb.
        for head in (state, real_gb):
            with self.subTest(height=head["head"]["inner_lite"]["height"]):
                with self.assertRaises(AuthenticatedLightClientError):
                    verify_authenticated_outcome(
                        head, response["result"], outcome_id=RECEIPT, executor_id=RECEIVER
                    )
        # SYNTHETIC head carrying the real V7Kc block Merkle root (same
        # technique as test_near_authenticated): the real outcome, its status
        # serialization and both Merkle paths verify under that root.
        synthetic_head = make_view(
            height=276418680,
            epoch=marker("authenticated-epoch"),
            next_epoch=marker("authenticated-next"),
            next_validators=[validator(seed(124), "n.testnet")],
            label="synthetic-head-with-real-root",
            block_merkle_root="6F2mBDUkzB8tRpETPpRTtH5zo1E5yGGpftfyNYbRVDVw",
        )
        synthetic_state, _, _, _ = anchored("testnet", synthetic_head)
        outcome = verify_authenticated_outcome(
            synthetic_state, response["result"], outcome_id=RECEIPT, executor_id=RECEIVER
        )
        self.assertTrue(outcome.authenticated)
        self.assertFalse(outcome.authorized_to_sign)
        self.assertEqual(outcome.status, "SuccessValue")
        self.assertEqual(outcome.block_outcome_root, "7mUR47bB4y7d7MbakknJGSdqmpCEDL5X47Y5LMq9BQnp")
        included = verify_authenticated_inclusion(
            synthetic_state, response, receipt_id=RECEIPT, receiver_id=RECEIVER
        )
        self.assertEqual(included.block_outcome_root, outcome.block_outcome_root)
        with self.assertRaisesRegex(AuthenticatedLightClientError, "another outcome"):
            verify_authenticated_outcome(
                synthetic_state, response["result"], outcome_id=RECEIPT,
                executor_id="attacker.testnet",
            )


class CommittedCeremonyCompatibilityTests(unittest.TestCase):
    def test_committed_ceremony_documents_still_validate(self):
        # Compatibility only: the expected digests are taken from the files,
        # so this does not re-establish trust in the ceremony.
        read = lambda name: json.loads((CEREMONY / name).read_text())  # noqa: E731
        trust_policy = read("trust-policy.json")
        checkpoint = read("checkpoint.json")
        state = initialize_from_documents(
            checkpoint,
            read("checkpoint-approval.json"),
            trust_policy,
            expected_policy_sha256=na.trust_policy_checksum(trust_policy),
            expected_checkpoint_sha256=checkpoint_checksum(checkpoint),
        )
        self.assertEqual(state["head"]["inner_lite"]["height"], 276392452)
        head = na._validate_state(read("authenticated-head.json"))
        self.assertEqual(head["head_hash"], "4gugmCPdbFQPhfo6YS9PVQaQAfkxpgeaAMwCNPa1akj2")
        self.assertEqual(advance_state(state, json.loads(GB.read_text()))["head_hash"], head["head_hash"])


if __name__ == "__main__":
    unittest.main()
