//! # NEAR MPC key derivation for secp256k1 (ECDSA domain), and the EVM address of the result
//!
//! A port of the derivation the NEAR MPC signer contract itself runs, pinned to
//! `near/mpc` commit `686fbb2de1a6b426303c23617ab84773f15bfb19` (workspace version 3.19.0):
//!
//! - `crates/near-mpc-crypto-types/src/kdf.rs`, `derive_tweak`:
//!   `tweak = SHA3-256("near-mpc-recovery v0.1.0 epsilon derivation:" || predecessor || "," || path)`.
//! - `crates/contract/src/crypto_shared/kdf.rs`, `derive_key_secp256k1`:
//!   `derived = G * Scalar::from_repr(tweak) + root`, where `from_repr` reads the tweak big-endian
//!   and refuses a value not below the group order.
//! - The MPC contract writes secp256k1 public keys as `secp256k1:` + base58 of the 64-byte
//!   uncompressed point without its `0x04` prefix (`dtos::Secp256k1PublicKey`).
//!
//! The EVM address is the last 20 bytes of Keccak-256 over those 64 bytes (Ethereum yellow paper,
//! appendix F). Every function here is pure: the same code runs on chain and in the tests that
//! check it against the MPC repository's own snapshots and the NEAR docs' published addresses.

use k256::elliptic_curve::point::AffineCoordinates;
use k256::elliptic_curve::sec1::{FromEncodedPoint, ToEncodedPoint};
use k256::elliptic_curve::PrimeField;
use k256::{AffinePoint, EncodedPoint, ProjectivePoint, Scalar};
use sha3::{Digest, Keccak256, Sha3_256};

/// The exact prefix from `near-mpc-crypto-types/src/kdf.rs` (`TWEAK_DERIVATION_PREFIX`).
pub const TWEAK_DERIVATION_PREFIX: &str = "near-mpc-recovery v0.1.0 epsilon derivation:";
pub const SECP256K1_PREFIX: &str = "secp256k1:";

pub fn keccak256(data: &[u8]) -> [u8; 32] {
    Keccak256::digest(data).into()
}

/// The MPC tweak ("epsilon") for keys requested by `predecessor` under `path`.
pub fn derive_tweak(predecessor: &str, path: &str) -> [u8; 32] {
    let mut hasher = Sha3_256::new();
    hasher.update(TWEAK_DERIVATION_PREFIX.as_bytes());
    hasher.update(predecessor.as_bytes());
    hasher.update(b",");
    hasher.update(path.as_bytes());
    hasher.finalize().into()
}

fn point_from_uncompressed(key: &[u8; 64]) -> Result<AffinePoint, &'static str> {
    let encoded = EncodedPoint::from_untagged_bytes(key.into());
    Option::<AffinePoint>::from(AffinePoint::from_encoded_point(&encoded))
        .ok_or("the root public key is not a point on secp256k1")
}

fn uncompressed(point: &AffinePoint) -> Result<[u8; 64], &'static str> {
    let encoded = point.to_encoded_point(false);
    let bytes = encoded.as_bytes();
    if bytes.len() != 65 || bytes[0] != 0x04 {
        return Err("the derived key is the point at infinity");
    }
    let mut out = [0u8; 64];
    out.copy_from_slice(&bytes[1..]);
    Ok(out)
}

/// `root + G * tweak`, exactly as `derive_key_secp256k1` in the MPC contract.
pub fn derive_public_key(root: &[u8; 64], tweak: &[u8; 32]) -> Result<[u8; 64], &'static str> {
    let scalar = Option::<Scalar>::from(Scalar::from_repr((*tweak).into()))
        .ok_or("the tweak is not below the secp256k1 group order")?;
    let root = point_from_uncompressed(root)?;
    let derived = (ProjectivePoint::GENERATOR * scalar + ProjectivePoint::from(root)).to_affine();
    uncompressed(&derived)
}

/// Check that 64 bytes are an uncompressed secp256k1 point.
pub fn validate_public_key(key: &[u8; 64]) -> Result<(), &'static str> {
    point_from_uncompressed(key).map(|_| ())
}

/// `secp256k1:<base58 of 64 bytes>` → the 64 bytes.
pub fn parse_near_secp256k1_key(text: &str) -> Result<[u8; 64], &'static str> {
    let body = text
        .strip_prefix(SECP256K1_PREFIX)
        .ok_or("the key must start with secp256k1:")?;
    let bytes = near_sdk::bs58::decode(body)
        .into_vec()
        .map_err(|_| "the key is not base58")?;
    bytes.try_into().map_err(|_| "a secp256k1 key is 64 bytes")
}

pub fn format_near_secp256k1_key(key: &[u8; 64]) -> String {
    format!("{SECP256K1_PREFIX}{}", near_sdk::bs58::encode(key).into_string())
}

/// The EVM address of an uncompressed secp256k1 key.
pub fn evm_address(key: &[u8; 64]) -> [u8; 20] {
    let hash = keccak256(key);
    let mut out = [0u8; 20];
    out.copy_from_slice(&hash[12..]);
    out
}

/// Whether `y` of an uncompressed key is odd (used only by tests and views).
pub fn y_is_odd(key: &[u8; 64]) -> bool {
    key[63] & 1 == 1
}

/// The compressed x coordinate check used when reading MPC signatures: `big_r` must be a curve
/// point, and its x coordinate is the ECDSA `r`.
pub fn compressed_point_x(compressed: &[u8; 33]) -> Result<[u8; 32], &'static str> {
    let encoded = EncodedPoint::from_bytes(compressed).map_err(|_| "big_r is not a SEC1 point")?;
    if encoded.is_compressed() {
        let point = Option::<AffinePoint>::from(AffinePoint::from_encoded_point(&encoded))
            .ok_or("big_r is not on secp256k1")?;
        Ok(point.x().into())
    } else {
        Err("big_r must be compressed")
    }
}
