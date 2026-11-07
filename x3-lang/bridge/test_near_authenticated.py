"""Authenticated light-client tests; synthetic trust never masquerades as live."""
import copy
import contextlib
import hashlib
import io
import json
import tempfile
import unittest
from pathlib import Path

from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey

from near_authenticated import (
    APPROVAL_SCHEMA,
    CHECKPOINT_DOMAIN,
    CHECKPOINT_SCHEMA,
    STATE_SCHEMA,
    TRUST_POLICY_SCHEMA,
    AuthenticatedHeadStore,
    AuthenticatedLightClientError,
    _approval_message,
    _b58encode,
    _key_bytes,
    _signature_bytes,
    _valid_ed25519_public_key,
    _verify_approvals,
    _view_block_hash,
    advance_state,
    authenticated_head,
    build_checkpoint_candidate,
    checkpoint_checksum,
    initialize_from_documents,
    main,
    seal_checkpoint_document,
    trust_policy_checksum,
    verify_authenticated_inclusion,
)
from near_merkle import decode_hash, encode_hash, lite_block_hash
from near_validator_commitment import compute_next_bps_hash


ROOT = Path(__file__).parent
P27 = ROOT / "fixtures" / "near_live_checkpoint_candidate_p27_20261105.json"
GB = ROOT / "fixtures" / "near_live_light_client_update_20261105.json"
RECEIPT_PROOF = ROOT / "fixtures" / "near_live_receipt_proof_20261105.json"
RECEIPT = "3f9FCzqC8k75AGob2UrCxbKVqADRyv5M5vGfoLEXbZyu"
RECEIVER = "league-v7.near-post.testnet"
REAL_HEAD_ROOT = "6F2mBDUkzB8tRpETPpRTtH5zo1E5yGGpftfyNYbRVDVw"


def canonical(value):
    return json.dumps(
        value, sort_keys=True, separators=(",", ":"), ensure_ascii=False
    ).encode()


def sha(value):
    return hashlib.sha256(canonical(value)).hexdigest()


def seed(byte):
    return Ed25519PrivateKey.from_private_bytes(bytes([byte]) * 32)


def key_token(private):
    return "ed25519:" + _b58encode(private.public_key().public_bytes_raw())


def signature_token(private, message):
    return "ed25519:" + _b58encode(private.sign(message))


def identity_key_token():
    return "ed25519:" + _b58encode(b"\x01" + b"\x00" * 31)


def identity_universal_signature_token():
    # For the identity public key, R = B and S = 1 verifies for every message.
    point_b = bytes.fromhex("58" + "66" * 31)
    scalar_one = b"\x01" + b"\x00" * 31
    return "ed25519:" + _b58encode(point_b + scalar_one)


def marker(label):
    return encode_hash(hashlib.sha256(label.encode()).digest())


def validator(private, account, stake="100"):
    return {
        "account_id": account,
        "public_key": key_token(private),
        "stake": str(stake),
        "validator_stake_struct_version": "V1",
    }


def make_view(
    *,
    height,
    epoch,
    next_epoch,
    next_validators,
    label,
    block_merkle_root=None,
):
    next_bp_hash = (
        compute_next_bps_hash(next_validators) if next_validators is not None
        else marker(f"{label}:next-bp")
    )
    return {
        "prev_block_hash": marker(f"{label}:prev"),
        "next_block_inner_hash": marker(f"{label}:next-inner"),
        "inner_rest_hash": marker(f"{label}:inner-rest"),
        "inner_lite": {
            "height": height,
            "epoch_id": epoch,
            "next_epoch_id": next_epoch,
            "prev_state_root": marker(f"{label}:state-root"),
            "outcome_root": marker(f"{label}:outcome-root"),
            "timestamp": 1_800_000_000_000_000_000 + height,
            "next_bp_hash": next_bp_hash,
            "block_merkle_root": block_merkle_root or marker(f"{label}:block-root"),
        },
        "next_bps": copy.deepcopy(next_validators),
        "approvals_after_next": [],
    }


def sign_candidate(view, validators, private_keys, indices=None):
    result = copy.deepcopy(view)
    block_hash = _view_block_hash(result)
    next_hash = hashlib.sha256(
        decode_hash(result["next_block_inner_hash"]) + decode_hash(block_hash)
    ).digest()
    message = _approval_message(next_hash, result["inner_lite"]["height"])
    selected = set(range(len(validators)) if indices is None else indices)
    result["approvals_after_next"] = [
        signature_token(private, message) if index in selected else None
        for index, private in enumerate(private_keys)
    ]
    return result


def policy(private, *, network="testnet", genesis=None, versions=None):
    return {
        "schema": TRUST_POLICY_SCHEMA,
        "network": network,
        "genesis_hash": genesis or marker("testnet-genesis"),
        "allowed_protocol_versions": versions or [88],
        "checkpoint_signers": [
            {"signer_id": "offline-test-authority", "public_key": key_token(private)}
        ],
        "threshold": 1,
    }


def checkpoint_document(view, current_validators, *, network="testnet", genesis=None,
                        protocol_version=88, block_hash=None):
    try:
        current_hash = compute_next_bps_hash(current_validators)
    except ValueError:
        current_hash = marker("invalid-current-validator-commitment")
    return {
        "schema": CHECKPOINT_SCHEMA,
        "network": network,
        "genesis_hash": genesis or marker("testnet-genesis"),
        "protocol_version": protocol_version,
        "block_height": view["inner_lite"]["height"],
        "block_hash": block_hash or _view_block_hash(view),
        "epoch_id": view["inner_lite"]["epoch_id"],
        "next_epoch_id": view["inner_lite"]["next_epoch_id"],
        "light_client_block": copy.deepcopy(view),
        "current_validators": copy.deepcopy(current_validators),
        "validator_commitments": {
            "current_validators_hash": current_hash,
            "next_bp_hash": view["inner_lite"]["next_bp_hash"],
        },
        "provenance": {
            "source": "offline checkpoint review fixture",
            "verified_by": "test operator",
            "observed_at": "2026-11-06T00:00:00Z",
        },
    }


def seal_checkpoint(document, signer):
    document = copy.deepcopy(document)
    document["checksum"] = sha(document)
    checksum = document["checksum"]
    approval = {
        "schema": APPROVAL_SCHEMA,
        "checkpoint_sha256": checksum,
        "approved_at": "2026-11-06T00:00:00Z",
        "signatures": [
            {
                "signer_id": "offline-test-authority",
                "signature": signature_token(
                    signer, CHECKPOINT_DOMAIN + bytes.fromhex(checksum)
                ),
            }
        ],
    }
    return document, approval


def approved_state(view, current_validators, signer):
    trust_policy = policy(signer)
    checkpoint, approval = seal_checkpoint(
        checkpoint_document(view, current_validators), signer
    )
    state = initialize_from_documents(
        checkpoint,
        approval,
        trust_policy,
        expected_policy_sha256=sha(trust_policy),
        expected_checkpoint_sha256=checkpoint_checksum(checkpoint),
    )
    return state, checkpoint, approval, trust_policy


class AuthenticatedLightClientTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="x3-near-authenticated-")
        self.addCleanup(self.tmp.cleanup)
        self.tmp_path = Path(self.tmp.name)

    @staticmethod
    def real_p27_setup():
        p27 = json.loads(P27.read_text())["result"]
        gb = json.loads(GB.read_text())
        trust_signer = seed(90)
        current = [validator(seed(91), "checkpoint-current.testnet", 100)]
        state, checkpoint, approval, trust_policy = approved_state(
            p27, current, trust_signer
        )
        return state, gb, checkpoint, approval, trust_policy

    def test_real_fixture_bytes_are_pinned(self):
        self.assertEqual(
            hashlib.sha256(P27.read_bytes()).hexdigest(),
            "466f916b65037b2f3d51e0958d3cb8ae1e989c4e18f8d64e07b9961939f1ffb6",
        )
        self.assertEqual(
            hashlib.sha256(GB.read_bytes()).hexdigest(),
            "c47f0c381dbd9a37ac432aa4f77cf9d47ea8ccc452bfe4ff2a67b868953caed7",
        )

    def test_checkpoint_candidate_builder_and_hashes_are_deterministic(self):
        p27 = json.loads(P27.read_text())
        current = [validator(seed(92), "reviewed-current.testnet")]
        built = build_checkpoint_candidate(
            p27,
            current,
            network="testnet",
            genesis_hash=marker("testnet-genesis"),
            protocol_version=88,
            provenance={
                "source": "fixture",
                "verified_by": "test review",
                "observed_at": "2026-11-06T00:00:00Z",
            },
        )
        sealed = seal_checkpoint_document(built)
        self.assertEqual(checkpoint_checksum(sealed), sealed["checksum"])
        self.assertEqual(sealed, seal_checkpoint_document(sealed))
        trust = policy(seed(93))
        self.assertEqual(trust_policy_checksum(trust), sha(trust))

    def test_real_testnet_signatures_advance_after_offline_checkpoint_approval(self):
        state, gb, _, _, _ = self.real_p27_setup()
        advanced = advance_state(state, gb)
        head = authenticated_head(advanced)
        self.assertEqual(head.height, 276420402)
        self.assertEqual(head.epoch_id, "GbZqwYHuVsdzj91APrve7tykRtgZbrp8MrfEJYq9DFLU")
        self.assertEqual(head.block_hash, "4gugmCPdbFQPhfo6YS9PVQaQAfkxpgeaAMwCNPa1akj2")
        self.assertEqual(head.sequence, 1)
        self.assertGreater(
            int(advanced["last_approved_stake"]) * 3,
            int(advanced["last_total_stake"]) * 2,
        )
        self.assertGreater(
            int(advanced["last_approved_stake"]),
            int(advanced["last_total_stake"]) * 99 // 100,
        )

    def test_exactly_two_thirds_fails_and_greater_than_two_thirds_passes(self):
        epoch = marker("epoch-a")
        next_epoch = marker("epoch-b")
        current = [validator(seed(i), f"v{i}.testnet") for i in (1, 2, 3)]
        future = [validator(seed(i), f"n{i}.testnet") for i in (4, 5, 6)]
        view = make_view(
            height=100,
            epoch=epoch,
            next_epoch=next_epoch,
            next_validators=future,
            label="checkpoint",
        )
        state, _, _, _ = approved_state(view, current, seed(20))
        candidate = make_view(
            height=110,
            epoch=epoch,
            next_epoch=next_epoch,
            next_validators=future,
            label="candidate",
        )
        two = sign_candidate(candidate, current, [seed(1), seed(2), seed(3)], {0, 1})
        with self.assertRaises(AuthenticatedLightClientError):
            advance_state(state, two)
        three = sign_candidate(candidate, current, [seed(1), seed(2), seed(3)])
        self.assertEqual(authenticated_head(advance_state(state, three)).height, 110)

    def test_checkpoint_signature_hash_and_protocol_are_not_booleans(self):
        trust = seed(30)
        current = [validator(seed(31), "v.testnet")]
        future = [validator(seed(32), "n.testnet")]
        view = make_view(
            height=100,
            epoch=marker("epoch"),
            next_epoch=marker("next"),
            next_validators=future,
            label="checkpoint",
        )
        checkpoint, approval = seal_checkpoint(checkpoint_document(view, current), trust)
        trust_policy = policy(trust)
        good = dict(
            expected_policy_sha256=sha(trust_policy),
            expected_checkpoint_sha256=checkpoint_checksum(checkpoint),
        )
        bad_approval = copy.deepcopy(approval)
        bad_approval["signatures"][0]["signature"] = signature_token(
            seed(33), CHECKPOINT_DOMAIN
        )
        with self.assertRaises(AuthenticatedLightClientError):
            initialize_from_documents(checkpoint, bad_approval, trust_policy, **good)
        with self.assertRaises(AuthenticatedLightClientError):
            initialize_from_documents(
                checkpoint,
                approval,
                trust_policy,
                expected_policy_sha256="0" * 64,
                expected_checkpoint_sha256=good["expected_checkpoint_sha256"],
            )
        with self.assertRaises(AuthenticatedLightClientError):
            initialize_from_documents(
                checkpoint,
                approval,
                policy(trust, versions=[87]),
                expected_policy_sha256=sha(policy(trust, versions=[87])),
                expected_checkpoint_sha256=good["expected_checkpoint_sha256"],
            )

    def test_checkpoint_policy_threshold_is_enforced(self):
        first, second = seed(34), seed(35)
        current = [validator(seed(36), "v.testnet")]
        view = make_view(
            height=100,
            epoch=marker("threshold-epoch"),
            next_epoch=marker("threshold-next"),
            next_validators=[validator(seed(37), "n.testnet")],
            label="threshold-checkpoint",
        )
        checkpoint, approval = seal_checkpoint(checkpoint_document(view, current), first)
        approval["signatures"][0]["signer_id"] = "authority-a"
        trust_policy = {
            "schema": TRUST_POLICY_SCHEMA,
            "network": "testnet",
            "genesis_hash": marker("testnet-genesis"),
            "allowed_protocol_versions": [88],
            "checkpoint_signers": [
                {"signer_id": "authority-a", "public_key": key_token(first)},
                {"signer_id": "authority-b", "public_key": key_token(second)},
            ],
            "threshold": 2,
        }
        with self.assertRaises(AuthenticatedLightClientError):
            initialize_from_documents(
                checkpoint,
                approval,
                trust_policy,
                expected_policy_sha256=sha(trust_policy),
                expected_checkpoint_sha256=checkpoint["checksum"],
            )
        approval["signatures"].append(
            {
                "signer_id": "authority-b",
                "signature": signature_token(
                    second,
                    CHECKPOINT_DOMAIN + bytes.fromhex(checkpoint["checksum"]),
                ),
            }
        )
        state = initialize_from_documents(
            checkpoint,
            approval,
            trust_policy,
            expected_policy_sha256=sha(trust_policy),
            expected_checkpoint_sha256=checkpoint["checksum"],
        )
        self.assertEqual(state["sequence"], 0)

    def test_small_order_checkpoint_signer_cannot_forge_approval(self):
        current = [validator(seed(33), "v.testnet")]
        view = make_view(
            height=100,
            epoch=marker("identity-epoch"),
            next_epoch=marker("identity-next"),
            next_validators=[validator(seed(34), "n.testnet")],
            label="identity-checkpoint",
        )
        checkpoint = checkpoint_document(view, current)
        checkpoint["checksum"] = sha(checkpoint)
        approval = {
            "schema": APPROVAL_SCHEMA,
            "checkpoint_sha256": checkpoint["checksum"],
            "approved_at": "2026-11-06T00:00:00Z",
            "signatures": [
                {
                    "signer_id": "identity-authority",
                    "signature": identity_universal_signature_token(),
                }
            ],
        }
        trust_policy = {
            "schema": TRUST_POLICY_SCHEMA,
            "network": "testnet",
            "genesis_hash": marker("testnet-genesis"),
            "allowed_protocol_versions": [88],
            "checkpoint_signers": [
                {
                    "signer_id": "identity-authority",
                    "public_key": identity_key_token(),
                }
            ],
            "threshold": 1,
        }
        with self.assertRaisesRegex(
            AuthenticatedLightClientError, "malformed checkpoint signer key"
        ):
            initialize_from_documents(
                checkpoint,
                approval,
                trust_policy,
                expected_policy_sha256=sha(trust_policy),
                expected_checkpoint_sha256=checkpoint["checksum"],
            )

    def test_small_order_validator_cannot_forge_quorum(self):
        signer = seed(38)
        current = [
            {
                "account_id": "identity-validator.testnet",
                "public_key": identity_key_token(),
                "stake": "100",
                "validator_stake_struct_version": "V1",
            }
        ]
        future = [validator(seed(39), "n.testnet")]
        checkpoint_view = make_view(
            height=100,
            epoch=marker("validator-identity-epoch"),
            next_epoch=marker("validator-identity-next"),
            next_validators=future,
            label="validator-identity-checkpoint",
        )
        with self.assertRaisesRegex(
            AuthenticatedLightClientError, "malformed validator public key"
        ):
            state, _, _, _ = approved_state(checkpoint_view, current, signer)
            candidate = make_view(
                height=110,
                epoch=marker("validator-identity-epoch"),
                next_epoch=marker("validator-identity-next"),
                next_validators=future,
                label="validator-identity-candidate",
            )
            candidate["approvals_after_next"] = [
                identity_universal_signature_token()
            ]
            advance_state(state, candidate)

    def test_approval_vector_cannot_extend_past_validator_set(self):
        signer = seed(43)
        current = [validator(seed(44), "v.testnet")]
        future = [validator(seed(45), "n.testnet")]
        epoch = marker("extra-approval-epoch")
        next_epoch = marker("extra-approval-next")
        view = make_view(
            height=100,
            epoch=epoch,
            next_epoch=next_epoch,
            next_validators=future,
            label="extra-approval-checkpoint",
        )
        state, _, _, _ = approved_state(view, current, signer)
        candidate = make_view(
            height=110,
            epoch=epoch,
            next_epoch=next_epoch,
            next_validators=future,
            label="extra-approval-candidate",
        )
        candidate = sign_candidate(candidate, current, [seed(44)])
        candidate["approvals_after_next"].append(None)
        with self.assertRaisesRegex(
            AuthenticatedLightClientError, "approval vector exceeds validator set"
        ):
            advance_state(state, candidate)

    def test_cross_network_and_genesis_mismatch_rejected_before_initialization(self):
        trust = seed(40)
        current = [validator(seed(41), "v.testnet")]
        view = make_view(
            height=100,
            epoch=marker("epoch"),
            next_epoch=marker("next"),
            next_validators=[validator(seed(42), "n.testnet")],
            label="checkpoint",
        )
        checkpoint, approval = seal_checkpoint(checkpoint_document(view, current), trust)
        wrong_policy = policy(trust, network="mainnet")
        with self.assertRaises(AuthenticatedLightClientError):
            initialize_from_documents(
                checkpoint,
                approval,
                wrong_policy,
                expected_policy_sha256=sha(wrong_policy),
                expected_checkpoint_sha256=checkpoint_checksum(checkpoint),
            )
        wrong_genesis = policy(trust, genesis=marker("other-genesis"))
        with self.assertRaises(AuthenticatedLightClientError):
            initialize_from_documents(
                checkpoint,
                approval,
                wrong_genesis,
                expected_policy_sha256=sha(wrong_genesis),
                expected_checkpoint_sha256=checkpoint_checksum(checkpoint),
            )

    def test_duplicate_and_overflowing_validator_sets_fail(self):
        trust = seed(50)
        view = make_view(
            height=100,
            epoch=marker("epoch"),
            next_epoch=marker("next"),
            next_validators=None,
            label="checkpoint",
        )
        for validators in (
            [validator(seed(51), "same.testnet"), validator(seed(51), "other.testnet")],
            [validator(seed(51), "same.testnet"), validator(seed(52), "same.testnet")],
            [validator(seed(51), "v.testnet", (1 << 128))],
            [
                {
                    "account_id": "malformed.testnet",
                    "public_key": "ed25519:" + _b58encode(b"\xff" * 32),
                    "stake": "100",
                    "validator_stake_struct_version": "V1",
                }
            ],
        ):
            with self.subTest(validators=validators):
                checkpoint, approval = seal_checkpoint(
                    checkpoint_document(view, validators), trust
                )
                trust_policy = policy(trust)
                with self.assertRaises(AuthenticatedLightClientError):
                    initialize_from_documents(
                        checkpoint,
                        approval,
                        trust_policy,
                        expected_policy_sha256=sha(trust_policy),
                        expected_checkpoint_sha256=checkpoint_checksum(checkpoint),
                    )
        high = [
            validator(seed(51), "high-a.testnet", (1 << 127) + 1),
            validator(seed(52), "high-b.testnet", (1 << 127) - 1),
        ]
        with self.assertRaises(AuthenticatedLightClientError):
            _verify_approvals(high, [None, None], b"\0" + b"\0" * 40)

    def test_known_ed25519_torsion_points_are_rejected(self):
        small_order = (
            "0000000000000000000000000000000000000000000000000000000000000000",
            "0000000000000000000000000000000000000000000000000000000000000080",
            "0100000000000000000000000000000000000000000000000000000000000000",
            "ecffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff7f",
            "26e8958fc2b227b045c3f489f2ef98f0d5dfac05d3c63339b13802886d53fc05",
            "c7176a703d4dd84fba3c0b760d10670f2a2053fa2c39ccdefb2c3f5d0c3b6e",
        )
        for encoded in small_order:
            with self.subTest(encoded=encoded):
                self.assertFalse(_valid_ed25519_public_key(bytes.fromhex(encoded)))
        valid_key = seed(57).public_key().public_bytes_raw()
        self.assertTrue(_valid_ed25519_public_key(valid_key))

    def test_oversized_ed25519_text_is_rejected_before_decode(self):
        with self.assertRaisesRegex(
            AuthenticatedLightClientError, "invalid key encoding length"
        ):
            _key_bytes("ed25519:" + "1" * 100_000, "key")
        with self.assertRaisesRegex(
            AuthenticatedLightClientError, "invalid signature encoding length"
        ):
            _signature_bytes("ed25519:" + "1" * 100_000, "signature")

    def test_epoch_crossing_requires_announced_validator_set(self):
        current = [validator(seed(i), f"v{i}.testnet") for i in (60, 61, 62)]
        future = [validator(seed(i), f"n{i}.testnet") for i in (63, 64, 65)]
        epoch, next_epoch = marker("old"), marker("new")
        view = make_view(
            height=100,
            epoch=epoch,
            next_epoch=next_epoch,
            next_validators=future,
            label="checkpoint",
        )
        state, _, _, _ = approved_state(view, current, seed(66))
        crossing = make_view(
            height=120,
            epoch=next_epoch,
            next_epoch=marker("future"),
            next_validators=[validator(seed(70), "x.testnet")],
            label="crossing",
        )
        crossing["next_bps"] = None
        crossing["inner_lite"]["next_bp_hash"] = marker("unknown-next")
        crossing = sign_candidate(
            crossing, future, [seed(63), seed(64), seed(65)]
        )
        with self.assertRaises(AuthenticatedLightClientError):
            advance_state(state, crossing)
        wrong_epoch = sign_candidate(
            make_view(
                height=120,
                epoch=marker("wrong"),
                next_epoch=marker("future"),
                next_validators=None,
                label="wrong",
            ),
            current,
            [seed(60), seed(61), seed(62)],
        )
        with self.assertRaises(AuthenticatedLightClientError):
            advance_state(state, wrong_epoch)

    def test_two_epoch_validator_rotations_use_only_precommitted_sets(self):
        a = [validator(seed(i), f"a{i}.testnet") for i in (70, 71, 72)]
        b = [validator(seed(i), f"b{i}.testnet") for i in (73, 74, 75)]
        c = [validator(seed(i), f"c{i}.testnet") for i in (76, 77, 78)]
        d = [validator(seed(i), f"d{i}.testnet") for i in (79, 80, 81)]
        e0, e1, e2, e3 = (marker(f"rotation-{index}") for index in range(4))
        checkpoint = make_view(
            height=100,
            epoch=e0,
            next_epoch=e1,
            next_validators=b,
            label="rotation-checkpoint",
        )
        state, _, _, _ = approved_state(checkpoint, a, seed(82))
        first = make_view(
            height=200,
            epoch=e1,
            next_epoch=e2,
            next_validators=c,
            label="rotation-first",
        )
        first = sign_candidate(first, b, [seed(73), seed(74), seed(75)])
        state = advance_state(state, first)
        second = make_view(
            height=300,
            epoch=e2,
            next_epoch=e3,
            next_validators=d,
            label="rotation-second",
        )
        second = sign_candidate(second, c, [seed(76), seed(77), seed(78)])
        final = authenticated_head(advance_state(state, second))
        self.assertEqual((final.height, final.epoch_id), (300, e2))
        self.assertEqual(final.sequence, 2)

    def test_forged_missing_reordered_and_modified_proofs_fail(self):
        state, gb, _, _, _ = self.real_p27_setup()
        candidate = copy.deepcopy(gb["result"])
        forged = copy.deepcopy(candidate)
        forged["approvals_after_next"][0] = forged["approvals_after_next"][1]
        with self.assertRaises(AuthenticatedLightClientError):
            advance_state(state, forged)
        reordered = copy.deepcopy(candidate)
        reordered["approvals_after_next"][0], reordered["approvals_after_next"][1] = (
            reordered["approvals_after_next"][1],
            reordered["approvals_after_next"][0],
        )
        with self.assertRaises(AuthenticatedLightClientError):
            advance_state(state, reordered)
        missing = copy.deepcopy(candidate)
        missing["approvals_after_next"] = [None] * len(
            missing["approvals_after_next"]
        )
        with self.assertRaises(AuthenticatedLightClientError):
            advance_state(state, missing)
        modified = copy.deepcopy(candidate)
        modified["inner_lite"]["timestamp"] += 1
        with self.assertRaises(AuthenticatedLightClientError):
            advance_state(state, modified)
        bad_ancestor = copy.deepcopy(candidate)
        bad_ancestor["prev_block_hash"] = marker("different-parent")
        with self.assertRaises(AuthenticatedLightClientError):
            advance_state(state, bad_ancestor)

    def test_next_bps_substitution_and_fake_commitment_fail(self):
        state, gb, _, _, _ = self.real_p27_setup()
        fake = copy.deepcopy(gb["result"])
        fake["next_bps"][0]["stake"] = "1"
        with self.assertRaises(AuthenticatedLightClientError):
            advance_state(state, fake)

        trust = seed(80)
        p27 = json.loads(P27.read_text())["result"]
        current = [validator(seed(81), "v.testnet")]
        checkpoint, approval = seal_checkpoint(checkpoint_document(p27, current), trust)
        trust_policy = policy(trust)
        original_checksum = checkpoint_checksum(checkpoint)
        substituted = copy.deepcopy(checkpoint)
        substituted["light_client_block"]["next_bps"][0]["stake"] = "1"
        substituted["light_client_block"]["inner_lite"]["next_bp_hash"] = (
            compute_next_bps_hash(substituted["light_client_block"]["next_bps"])
        )
        with self.assertRaises(AuthenticatedLightClientError):
            initialize_from_documents(
                substituted,
                approval,
                trust_policy,
                expected_policy_sha256=sha(trust_policy),
                expected_checkpoint_sha256=original_checksum,
            )

    def test_replay_rollback_and_conflicting_headers_fail(self):
        state, gb, _, _, _ = self.real_p27_setup()
        advanced = advance_state(state, gb)
        with self.assertRaises(AuthenticatedLightClientError):
            advance_state(advanced, gb)
        conflict = copy.deepcopy(gb["result"])
        conflict["prev_block_hash"] = marker("conflicting-parent")
        with self.assertRaises(AuthenticatedLightClientError):
            advance_state(state, conflict)
        with self.assertRaises(AuthenticatedLightClientError):
            advance_state(state, gb | {"network": "mainnet"})
        with self.assertRaises(AuthenticatedLightClientError):
            advance_state(state, gb | {"genesis_hash": marker("other-genesis")})
        with self.assertRaises(AuthenticatedLightClientError):
            advance_state(state, gb | {"protocol_version": 87})

    def test_state_tamper_and_interrupted_write_preserve_old_authority(self):
        state, gb, _, _, _ = self.real_p27_setup()
        state_path = self.tmp_path / "head.json"
        store = AuthenticatedHeadStore(state_path)
        # Use a direct atomic state write through a fresh approved checkpoint.
        trust = seed(85)
        current = [validator(seed(86), "v.testnet")]
        checkpoint_view = make_view(
            height=1,
            epoch=marker("a"),
            next_epoch=marker("b"),
            next_validators=[validator(seed(87), "n.testnet")],
            label="persist",
        )
        checkpoint, approval = seal_checkpoint(
            checkpoint_document(checkpoint_view, current), trust
        )
        trust_policy = policy(trust)
        store.initialize(
            checkpoint,
            approval,
            trust_policy,
            expected_policy_sha256=sha(trust_policy),
            expected_checkpoint_sha256=checkpoint_checksum(checkpoint),
        )
        before = state_path.read_bytes()
        candidate = make_view(
            height=10,
            epoch=marker("a"),
            next_epoch=marker("b"),
            next_validators=checkpoint_view["next_bps"],
            label="persist-candidate",
        )
        signed = sign_candidate(candidate, current, [seed(86)])

        def fail_write(_state):
            raise OSError("simulated crash before rename")

        with self.assertRaises(OSError):
            store.advance(signed, writer=fail_write)
        self.assertEqual(state_path.read_bytes(), before)
        persisted = json.loads(before)
        persisted["sequence"] = 99
        state_path.write_text(json.dumps(persisted))
        with self.assertRaises(AuthenticatedLightClientError):
            store.load()

    def test_real_receipt_inclusion_is_bound_to_authenticated_head_root(self):
        proof = json.loads(RECEIPT_PROOF.read_text())
        trust = seed(88)
        current = [validator(seed(89), "v.testnet")]
        view = make_view(
            height=276418680,
            epoch=marker("authenticated-epoch"),
            next_epoch=marker("authenticated-next"),
            next_validators=None,
            label="authenticated-head",
            block_merkle_root=REAL_HEAD_ROOT,
        )
        state, _, _, _ = approved_state(view, current, trust)
        included = verify_authenticated_inclusion(
            state,
            proof,
            receipt_id=RECEIPT,
            receiver_id=RECEIVER,
        )
        self.assertTrue(included.authenticated)
        self.assertFalse(included.authorized_to_sign)
        self.assertEqual(included.block_outcome_root,
                         "7mUR47bB4y7d7MbakknJGSdqmpCEDL5X47Y5LMq9BQnp")
        wrong = copy.deepcopy(state)
        wrong["head"]["inner_lite"]["block_merkle_root"] = marker("wrong-head-root")
        wrong["state_sha256"] = sha(
            {k: v for k, v in wrong.items() if k != "state_sha256"}
        )
        with self.assertRaises((AuthenticatedLightClientError, ValueError)):
            verify_authenticated_inclusion(
                wrong,
                proof,
                receipt_id=RECEIPT,
                receiver_id=RECEIVER,
            )

    def test_cli_initializes_advances_and_reads_persisted_head(self):
        trust = seed(94)
        current = [validator(seed(95), "cli-v.testnet")]
        future = [validator(seed(96), "cli-n.testnet")]
        checkpoint_view = make_view(
            height=10,
            epoch=marker("cli-epoch"),
            next_epoch=marker("cli-next"),
            next_validators=future,
            label="cli-checkpoint",
        )
        checkpoint, approval = seal_checkpoint(
            checkpoint_document(checkpoint_view, current), trust
        )
        trust_policy = policy(trust)
        paths = {
            "checkpoint": self.tmp_path / "checkpoint.json",
            "approval": self.tmp_path / "approval.json",
            "policy": self.tmp_path / "policy.json",
            "candidate": self.tmp_path / "candidate.json",
            "state": self.tmp_path / "state.json",
        }
        for key, value in (
            ("checkpoint", checkpoint),
            ("approval", approval),
            ("policy", trust_policy),
        ):
            paths[key].write_text(json.dumps(value))
        candidate = make_view(
            height=20,
            epoch=marker("cli-epoch"),
            next_epoch=marker("cli-next"),
            next_validators=future,
            label="cli-candidate",
        )
        candidate = sign_candidate(candidate, current, [seed(95)])
        paths["candidate"].write_text(json.dumps(candidate))
        output = io.StringIO()
        with contextlib.redirect_stdout(output):
            self.assertEqual(
                main(
                    [
                        "initialize",
                        "--checkpoint",
                        str(paths["checkpoint"]),
                        "--approval",
                        str(paths["approval"]),
                        "--trust-policy",
                        str(paths["policy"]),
                        "--trust-policy-sha256",
                        trust_policy_checksum(trust_policy),
                        "--checkpoint-sha256",
                        checkpoint_checksum(checkpoint),
                        "--state",
                        str(paths["state"]),
                    ]
                ),
                0,
            )
            self.assertEqual(
                main(
                    [
                        "advance",
                        "--state",
                        str(paths["state"]),
                        "--candidate",
                        str(paths["candidate"]),
                    ]
                ),
                0,
            )
            self.assertEqual(
                main(["head", "--state", str(paths["state"])]), 0
            )
        self.assertIn('"height": 20', output.getvalue())


if __name__ == "__main__":
    unittest.main()
