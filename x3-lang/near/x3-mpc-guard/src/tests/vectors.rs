//! Published vectors. Sources (read 2026-11-13):
//!
//! - NEAR MPC tweak and key derivation: `near/mpc` commit
//!   `686fbb2de1a6b426303c23617ab84773f15bfb19`,
//!   `crates/near-mpc-crypto-types/src/snapshots/near_mpc_crypto_types__kdf__tests__derive_tweak__has_not_changed.snap`
//!   and
//!   `crates/contract/src/crypto_shared/snapshots/mpc_contract__crypto_shared__kdf__tests__derive_key_secp256k1_has_not_changed.snap`.
//! - Derived EVM addresses: <https://docs.near.org/chain-abstraction/chain-signatures/implementation>
//!   (`example.near` + `ethereum-1` / `ethereum-2`), with the MPC root keys published in the
//!   official `chainsig.js` 1.1.16 npm package (`ROOT_PUBLIC_KEYS`).
//! - EIP-1559: ethereumjs-monorepo commit `11695f6cec3d99e143169e51ee91a0b6d146908f`,
//!   `packages/tx/test/testData/eip1559.ts` (marked "Source: Besu") and the
//!   `getMessageToSign()/getHashedMessageToSign()` access-list case in
//!   `packages/tx/test/eip-1559.spec.ts`.

use k256::ecdsa::{RecoveryId, Signature, SigningKey, VerifyingKey};
use k256::elliptic_curve::PrimeField;
use k256::{ProjectivePoint, Scalar};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

use super::*;
use crate::authority::{build_intent, canonical_bytes, intent_hash, SigningContext, SigningMandate};
use crate::derivation::{derive_public_key, derive_tweak, evm_address, keccak256, parse_near_secp256k1_key};
use crate::evm::{erc20_transfer_data, Eip1559Tx, EvmSignature, ERC20_TRANSFER_SELECTOR};

/// near/mpc derive_tweak snapshot: accounts × paths in that loop order.
const TWEAK_ACCOUNTS: [&str; 4] = ["dwefqwg", "qfweqwgwegqw", "fqwerijqw385", "fnwef0942534"];
const TWEAK_PATHS: [&str; 4] = ["frwewegwegweg", "fwei2.3f230", "f23fjwef8232", "fwefwo23fewfw"];
const TWEAK_SNAPSHOT: [[u8; 32]; 16] = [
    [
        173, 45, 111, 193, 244, 69, 161, 180, 56, 48, 65, 94, 126, 110, 61, 3, 205, 7, 118, 115, 4, 120, 72, 160, 133,
        241, 48, 194, 79, 83, 238, 0,
    ],
    [
        95, 118, 235, 164, 138, 32, 227, 221, 53, 94, 56, 245, 18, 193, 66, 171, 140, 141, 164, 153, 159, 107, 50, 236,
        211, 38, 210, 39, 180, 74, 192, 90,
    ],
    [
        209, 51, 247, 58, 188, 179, 199, 203, 148, 77, 204, 13, 211, 191, 114, 221, 137, 253, 195, 207, 152, 140, 159,
        2, 90, 245, 113, 162, 5, 201, 183, 48,
    ],
    [
        61, 208, 25, 44, 69, 190, 89, 238, 227, 221, 11, 166, 39, 118, 134, 66, 69, 247, 170, 194, 149, 20, 49, 75,
        128, 186, 171, 32, 87, 69, 62, 199,
    ],
    [
        81, 12, 211, 159, 9, 179, 193, 189, 193, 253, 101, 92, 229, 122, 178, 158, 178, 13, 252, 230, 71, 88, 71, 39,
        252, 202, 14, 72, 49, 99, 168, 208,
    ],
    [
        156, 247, 244, 10, 177, 75, 197, 221, 111, 238, 228, 129, 34, 179, 216, 95, 88, 70, 27, 213, 149, 228, 54, 250,
        193, 134, 184, 250, 56, 79, 48, 72,
    ],
    [
        245, 63, 92, 181, 198, 144, 46, 224, 121, 104, 16, 19, 180, 82, 241, 144, 137, 34, 244, 143, 214, 104, 173,
        108, 165, 34, 189, 73, 132, 43, 247, 42,
    ],
    [
        121, 5, 26, 209, 74, 98, 85, 46, 233, 218, 63, 184, 19, 129, 58, 157, 47, 47, 168, 225, 84, 158, 95, 76, 101,
        130, 84, 5, 37, 98, 225, 143,
    ],
    [
        249, 30, 219, 33, 42, 3, 96, 122, 182, 28, 231, 159, 84, 227, 137, 1, 175, 109, 159, 248, 112, 149, 147, 242,
        211, 3, 229, 144, 105, 28, 38, 166,
    ],
    [
        223, 182, 11, 59, 164, 52, 141, 61, 120, 122, 161, 175, 145, 41, 16, 164, 204, 194, 251, 52, 233, 44, 1, 15,
        218, 230, 115, 15, 79, 171, 7, 52,
    ],
    [
        53, 56, 98, 231, 150, 143, 127, 5, 138, 79, 4, 152, 175, 187, 205, 80, 75, 102, 33, 105, 13, 251, 49, 24, 135,
        138, 122, 101, 2, 226, 46, 14,
    ],
    [
        172, 5, 246, 50, 11, 14, 210, 15, 159, 24, 163, 83, 28, 87, 29, 178, 55, 244, 26, 185, 179, 18, 3, 89, 139,
        216, 35, 150, 49, 203, 4, 15,
    ],
    [
        117, 113, 252, 170, 208, 68, 83, 135, 8, 126, 188, 222, 88, 149, 64, 232, 9, 38, 207, 208, 129, 68, 170, 63,
        69, 188, 120, 184, 104, 61, 6, 195,
    ],
    [
        116, 241, 24, 207, 208, 19, 220, 16, 152, 72, 207, 197, 224, 249, 99, 225, 120, 29, 203, 176, 149, 174, 100,
        41, 98, 48, 7, 204, 245, 155, 98, 72,
    ],
    [
        135, 75, 223, 241, 23, 214, 217, 12, 14, 181, 205, 122, 24, 30, 127, 32, 217, 154, 191, 170, 4, 245, 3, 233,
        120, 103, 236, 8, 246, 121, 111, 150,
    ],
    [
        145, 218, 77, 137, 115, 234, 60, 202, 173, 156, 123, 113, 203, 129, 250, 81, 44, 76, 201, 57, 197, 21, 124,
        219, 246, 169, 96, 93, 61, 28, 228, 162,
    ],
];

#[test]
fn tweak_matches_the_near_mpc_snapshot() {
    let mut i = 0;
    for account in TWEAK_ACCOUNTS {
        for path in TWEAK_PATHS {
            assert_eq!(derive_tweak(account, path), TWEAK_SNAPSHOT[i], "{account},{path}");
            i += 1;
        }
    }
}

#[test]
fn secp256k1_derivation_matches_the_near_mpc_snapshot() {
    // test_derive_key_secp256k1_has_not_changed: root = G * StdRng::from_seed([42; 32]).gen().
    let random_bytes: [u8; 32] = StdRng::from_seed([42u8; 32]).gen();
    let scalar = Scalar::from_repr(random_bytes.into()).unwrap();
    let root = uncompressed_of(ProjectivePoint::GENERATOR * scalar);
    let derived = derive_public_key(&root, &derive_tweak("hello", "my-path")).unwrap();
    assert_eq!(
        format_near_secp256k1_key(&derived),
        "secp256k1:4zvzanBjPrvRWAr3mjdPX9sSkS8GZTZ11L6V2pcAbmHRd3HHbTyboHdWauExDmHrEoWnbhDkUwnzVEa8hg52C55p"
    );
}

/// chainsig.js 1.1.16 `ROOT_PUBLIC_KEYS[MAINNET].secp256k1` (`v1.signer`).
const MAINNET_ROOT: &str =
    "secp256k1:3tFRbMqmoa6AAALMrEFAYCEoHcqKxeW38YptwowBVBtXK1vo36HDbUWuR6EZmoK4JcH6HDkNMGGqP1ouV7VZUWya";
/// chainsig.js 1.1.16 `ROOT_PUBLIC_KEYS[TESTNET].secp256k1` (`v1.signer-prod.testnet`).
const TESTNET_ROOT: &str =
    "secp256k1:4NfTiv3UsGahebgTaHyD9vF8KYKMBnfd6kh94mK6xv8fGBiJB8TBtFMP5WWXz6B89Ac1fbpzPwAvoyQebemHFwx3";

fn address_for(root: &str, predecessor: &str, path: &str) -> String {
    let root = parse_near_secp256k1_key(root).unwrap();
    let key = derive_public_key(&root, &derive_tweak(predecessor, path)).unwrap();
    hex::format_evm_address(&evm_address(&key))
}

/// NOT an external vector. The NEAR docs example (`example.near` + `ethereum-1` →
/// `0x1b48b83a308ea4beb845db088180dc3389f8aa3b`, `ethereum-2` →
/// `0x99c5d3025dc736541f2d97c3ef3c90de4d221315`) could not be reproduced with either root key
/// chainsig.js 1.1.16 publishes, although chainsig.js derives exactly as this crate does
/// (`z(C(root), predecessor, path)`, then keccak of the 64-byte key). The example was likely made
/// with an older signer key. This test only pins our own outputs for the published keys so a
/// change is noticed; the binding check is the signer's own `derived_public_key`, compared on
/// chain by `on_foreign_account_checked`, and the near/mpc snapshots above.
#[test]
fn derived_addresses_for_published_root_keys_are_pinned_and_differ_from_the_docs_example() {
    let mainnet = [
        address_for(MAINNET_ROOT, "example.near", "ethereum-1"),
        address_for(MAINNET_ROOT, "example.near", "ethereum-2"),
    ];
    let testnet = [
        address_for(TESTNET_ROOT, "example.near", "ethereum-1"),
        address_for(TESTNET_ROOT, "example.near", "ethereum-2"),
    ];
    assert_eq!(
        mainnet,
        [
            "0xef4ea05e5c11057decc8e1bd6f5369f2acd91fa5",
            "0x273eeb4d7651d9296ec5bd9a6ceebca219101cb8"
        ]
    );
    assert_eq!(
        testnet,
        [
            "0x6787a751b4592a3871267c320c19ca01e5abaec4",
            "0x83bef8135884a31035a654cad4305345d13d3dc2"
        ]
    );
    let docs = "0x1b48b83a308ea4beb845db088180dc3389f8aa3b";
    assert!(mainnet[0] != docs && testnet[0] != docs);
}

#[test]
fn root_keys_from_chainsig_are_curve_points() {
    for key in [MAINNET_ROOT, TESTNET_ROOT] {
        crate::derivation::validate_public_key(&parse_near_secp256k1_key(key).unwrap()).unwrap();
    }
}

#[test]
fn derived_public_key_equals_the_key_of_the_derived_secret() {
    // The MPC signs with root_secret + tweak; the address must be that key's address.
    for path in [
        "x3-mpc-guard/v1/evm/owner.testnet/0",
        "x3-mpc-guard/v1/evm/owner.testnet/1",
    ] {
        let root = uncompressed_of(ProjectivePoint::GENERATOR * root_secret());
        let public = derive_public_key(&root, &derive_tweak(GUARD, path)).unwrap();
        assert_eq!(
            public,
            uncompressed_of(ProjectivePoint::GENERATOR * derived_secret(GUARD, path))
        );
    }
}

#[test]
fn evm_address_of_secret_one_is_the_known_address() {
    let g = uncompressed_of(ProjectivePoint::GENERATOR);
    assert_eq!(
        hex::format_evm_address(&evm_address(&g)),
        "0x7e5f4552091a69125d5dfcb7b8c2659029395bdf"
    );
}

#[test]
fn a_tweak_not_below_the_group_order_is_refused() {
    let root = uncompressed_of(ProjectivePoint::GENERATOR);
    assert!(derive_public_key(&root, &[0xff; 32]).is_err());
}

// ---- RLP decoding, test-only, to read the published signed transactions ----

#[derive(Debug, PartialEq)]
enum Rlp {
    Bytes(Vec<u8>),
    List(Vec<Rlp>),
}

fn be_len(bytes: &[u8]) -> usize {
    bytes.iter().fold(0, |n, b| (n << 8) | *b as usize)
}

fn decode(input: &[u8]) -> (Rlp, usize) {
    let p = input[0];
    match p {
        0x00..=0x7f => (Rlp::Bytes(vec![p]), 1),
        0x80..=0xb7 => {
            let n = (p - 0x80) as usize;
            (Rlp::Bytes(input[1..1 + n].to_vec()), 1 + n)
        }
        0xb8..=0xbf => {
            let l = (p - 0xb7) as usize;
            let n = be_len(&input[1..1 + l]);
            (Rlp::Bytes(input[1 + l..1 + l + n].to_vec()), 1 + l + n)
        }
        _ => {
            let (start, n) = if p <= 0xf7 {
                (1, (p - 0xc0) as usize)
            } else {
                let l = (p - 0xf7) as usize;
                (1 + l, be_len(&input[1..1 + l]))
            };
            let mut items = Vec::new();
            let mut at = start;
            while at < start + n {
                let (item, used) = decode(&input[at..]);
                items.push(item);
                at += used;
            }
            (Rlp::List(items), start + n)
        }
    }
}

fn bytes(item: &Rlp) -> &[u8] {
    match item {
        Rlp::Bytes(b) => b,
        Rlp::List(_) => panic!("expected bytes"),
    }
}

fn pad32(b: &[u8]) -> [u8; 32] {
    let mut out = [0u8; 32];
    out[32 - b.len()..].copy_from_slice(b);
    out
}

/// (nonce, value, gasLimit, maxPriorityFeePerGas, maxFeePerGas, signedTransactionRLP)
const BESU_EIP1559: [(u64, u128, u64, u128, u128, &str); 10] = [
    (819, 43203529, 35552, 75853, 121212, "b87102f86e048203338301284d8301d97c828ae094000000000000000000000000000000000000aaaa8402933bc980c080a00f924cb68412c8f1cfd74d9b581c71eeaf94fff6abdde3e5b02ca6b2931dcf47a07dd1c50027c3e31f8b565e25ce68a5072110f61fce5eee81b195dd51273c2f83"),
    (353, 61901619, 32593, 38850, 136295, "b87002f86d048201618297c283021467827f5194000000000000000000000000000000000000aaaa8403b08b3380c080a08caf712f72489da6f1a634b651b4b1c7d9be7d1e8d05ea76c1eccee3bdfb86a5a06aecc106f588ce51e112f5e9ea7aba3e089dc7511718821d0e0cd52f52af4e45"),
    (985, 32531825, 68541, 66377, 136097, "b87202f86f048203d983010349830213a183010bbd94000000000000000000000000000000000000aaaa8401f0657180c001a08c03a86e85789ee9a1b42fa0a86d316fca262694f8c198df11f194678c2c2d35a028f8e7de02b35014a17b6d28ff8c7e7be6860e7265ac162fb721f1aeae75643c"),
    (623, 21649799, 57725, 74140, 81173, "b87102f86e0482026f8301219c83013d1582e17d94000000000000000000000000000000000000aaaa84014a598780c001a0b87c4c8c505d2d692ac77ba466547e79dd60fe7ecd303d520bf6e8c7085e3182a06dc7d00f5e68c3f3ebe8ae35a90d46051afde620ac12e43cae9560a29b13e7fb"),
    (972, 94563383, 65254, 42798, 103466, "b87002f86d048203cc82a72e8301942a82fee694000000000000000000000000000000000000aaaa8405a2ec3780c001a006cf07af78c187db104496c58d679f37fcd2d5790970cecf9a59fe4a5321b375a039f3faafc71479d283a5b1e66a86b19c4bdc516655d89dbe57d9747747c01dfe"),
    (588, 99359647, 37274, 87890, 130273, "b87102f86e0482024c830157528301fce182919a94000000000000000000000000000000000000aaaa8405ec1b9f80c080a03e2f59ac9ca852034c2c1da35a742ca19fdd910aa5d2ed49ab8ad27a2fcb2b10a03ac1c29c26723c58f91400fb6dfb5f5b837467b1c377541b47dae474dddbe469"),
    (900, 30402257, 76053, 8714, 112705, "b87102f86e0482038482220a8301b8418301291594000000000000000000000000000000000000aaaa8401cfe6d180c001a0f7ffc5bca2512860f8236360159bf303dcfab71546b6a0032df0306f3739d0c4a05d38fe2c4edebdc1edc157034f780c53a0e5ae089e57220745bd48bcb10cdf87"),
    (709, 6478043, 28335, 86252, 94636, "b87002f86d048202c5830150ec830171ac826eaf94000000000000000000000000000000000000aaaa8362d8db80c001a0a61a5710512f346c9996377f7b564ccb64c73a5fdb615499adb1250498f3e01aa002d10429572cecfaa911a58bbe05f2b26e4c3aee3402202153a93692849add11"),
    (939, 2782905, 45047, 45216, 91648, "b86f02f86c048203ab82b0a08301660082aff794000000000000000000000000000000000000aaaa832a76b980c001a0191f0f6667a20cefc0b454e344cc01108aafbdc4e4e5ed88fdd1b5d108495b31a020879042b0f8d3807609f18fe42a9820de53c8a0ea1d0a2d50f8f5e92a94f00d"),
    (119, 65456115, 62341, 24721, 107729, "b86e02f86b04778260918301a4d182f38594000000000000000000000000000000000000aaaa8403e6c7f380c001a05e40977f4064a2bc08785e422ed8a47b56aa219abe93251d2b3b4d0cf937b8c0a071e600cd15589c3607bd9627314b99e9b5976bd427b774aa685bd0d036b1771e"),
];
const BESU_PRIVATE_KEY: &str = "8f2a55949038a9610f50fb23b5883af3b4ecb3c3bb792cbcefbd1542c692be63";

#[test]
fn eip1559_signing_hash_and_encoding_match_the_besu_vectors() {
    let secret = SigningKey::from_slice(&hex::decode(BESU_PRIVATE_KEY).unwrap()).unwrap();
    let expected_key = *secret.verifying_key();
    let mut checked = 0;
    for (nonce, value, gas_limit, priority, max_fee, signed_hex) in BESU_EIP1559 {
        let tx = Eip1559Tx {
            chain_id: 4,
            nonce,
            max_priority_fee_per_gas: priority,
            max_fee_per_gas: max_fee,
            gas_limit,
            to: hex::parse_evm_address("0x000000000000000000000000000000000000aaaa").unwrap(),
            value,
            data: Vec::new(),
            access_list: Vec::new(),
        };
        // The vector is the signed typed transaction wrapped once more as an RLP string.
        let (outer, _) = decode(&hex::decode(signed_hex).unwrap());
        let signed = bytes(&outer).to_vec();
        assert_eq!(signed[0], 0x02);
        let (Rlp::List(fields), _) = decode(&signed[1..]) else {
            panic!("not a list")
        };
        assert_eq!(fields.len(), 12);
        let y_parity = bytes(&fields[9]).first().copied().unwrap_or(0);
        let sig = EvmSignature {
            y_parity,
            r: pad32(bytes(&fields[10])),
            s: pad32(bytes(&fields[11])),
        };
        // Our encoder reproduces the published signed bytes exactly ...
        assert_eq!(tx.signed_bytes(&sig), signed, "nonce {nonce}");
        // ... and the published signature recovers, over our signing hash, to the vector's key.
        let hash = tx.signing_hash();
        let signature = Signature::from_scalars(sig.r, sig.s).unwrap();
        let recovered =
            VerifyingKey::recover_from_prehash(&hash, &signature, RecoveryId::from_byte(y_parity).unwrap()).unwrap();
        assert_eq!(recovered, expected_key, "nonce {nonce}");
        assert_eq!(hash, keccak256(&tx.unsigned_bytes()));
        checked += 1;
    }
    assert_eq!(checked, 10);
}

#[test]
fn eip1559_access_list_vector_matches_ethereumjs() {
    let tx = Eip1559Tx {
        chain_id: 4,
        nonce: 0,
        max_priority_fee_per_gas: 0,
        max_fee_per_gas: 0,
        gas_limit: 0,
        to: [1u8; 20],
        value: 0,
        data: vec![0x01, 0x02, 0x00],
        access_list: vec![([1u8; 20], vec![[1u8; 32]])],
    };
    assert_eq!(
        hex::encode(&tx.unsigned_bytes()),
        "02f85904808080809401010101010101010101010101010101010101018083010200f838f7940101010101010101010101010101010101010101e1a00101010101010101010101010101010101010101010101010101010101010101"
    );
    assert_eq!(
        hex::encode(&tx.signing_hash()),
        "fa81814f7dd57bad435657a05eabdba2815f41e3f15ddd6139027e7db56b0dea"
    );
}

#[test]
fn erc20_selector_is_the_transfer_signature_hash() {
    assert_eq!(keccak256(b"transfer(address,uint256)")[..4], ERC20_TRANSFER_SELECTOR);
    let data = erc20_transfer_data(&[0x22; 20], 0x0102);
    assert_eq!(data.len(), 68);
    assert_eq!(&data[16..36], &[0x22; 20]);
    assert_eq!(&data[66..], &[0x01, 0x02]);
}

fn sample_intent() -> crate::authority::X3SigningIntent {
    set(OWNER, 0, T0_MS);
    let m = SigningMandate {
        owner: acc(OWNER),
        agent: acc(AGENT),
        terms: terms(),
        version: 3,
        revoked: false,
        created_at_ms: T0_MS,
        spends: Vec::new(),
        storage_credit: U128(0),
    };
    let ctx = SigningContext {
        mpc_signer: &acc(SIGNER),
        mpc_domain_id: 0,
        predecessor: &acc(GUARD),
        derivation_path: "x3-mpc-guard/v1/evm/owner.testnet/0",
        from: "0x4444444444444444444444444444444444444444",
    };
    build_intent(&m, &request("n-1", 400_000, 5), &ctx).unwrap().0
}

#[test]
fn the_intent_carries_the_exact_preimage_and_hash_of_the_canonical_transaction() {
    let intent = sample_intent();
    let tx = Eip1559Tx {
        chain_id: SEPOLIA,
        nonce: 5,
        max_priority_fee_per_gas: GWEI,
        max_fee_per_gas: 30 * GWEI,
        gas_limit: 21_000,
        to: hex::parse_evm_address(RECIPIENT).unwrap(),
        value: 400_000,
        data: Vec::new(),
        access_list: Vec::new(),
    };
    assert_eq!(intent.unsigned_tx, hex::encode(&tx.unsigned_bytes()));
    assert_eq!(
        intent.signing_hash,
        hex::encode(&keccak256(&hex::decode(&intent.unsigned_tx).unwrap()))
    );
    assert_eq!(intent.version, crate::authority::SIGNING_INTENT_VERSION);
    assert_eq!(intent.chain_namespace, "eip155");
    assert_eq!(intent.max_total_fee.0, 21_000 * 30 * GWEI);
    assert_eq!(intent.refund, crate::authority::RefundStrategy::AtomicTransferNoRefund);
}

#[test]
fn the_intent_hash_is_canonical_and_covers_every_field() {
    let a = sample_intent();
    let b = sample_intent();
    assert_eq!(canonical_bytes(&a), canonical_bytes(&b));
    assert_eq!(intent_hash(&a), intent_hash(&b));
    let mut c = sample_intent();
    c.derivation_path.push('x');
    assert_ne!(intent_hash(&a), intent_hash(&c));
    let mut d = sample_intent();
    d.mandate_version += 1;
    assert_ne!(intent_hash(&a), intent_hash(&d));
}
