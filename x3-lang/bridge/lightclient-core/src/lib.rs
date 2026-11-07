//! NEAR light-client *approval signature primitive*. No checkpoint, state,
//! epoch transitions or permission to attest/pay funds.

use ed25519_dalek::{Signature, VerifyingKey};
use sha2::{Digest, Sha256};
use std::collections::HashSet;

/// One caller-provided validator in the epoch. **Untrusted** until a real
/// checkpoint and next_bps commitment have been authenticated separately.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Validator {
    pub public_key: [u8; 32],
    pub stake: u128,
}

/// Derive the *next* block hash, not just an RPC-supplied next hash.
/// Caller still must independently verify the current lite header and its
/// preceding checkpoint to authenticate either input.
pub fn reconstruct_next_block_hash(next_block_inner_hash: [u8; 32], current_block_hash: [u8; 32]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(next_block_inner_hash);
    hasher.update(current_block_hash);
    hasher.finalize().into()
}

/// NEAR approval serialization for an Endorsement(next_block_hash) followed
/// by the height of the block after next. Endorsement enum variant index = 0.
/// Does not itself authenticate or reconstruct next_block_hash.
pub fn approval_message(next_block_hash: [u8; 32], current_height: u64) -> Result<Vec<u8>, &'static str> {
    let after_next_height = current_height.checked_add(2).ok_or("height overflow")?;
    let mut out = Vec::with_capacity(41);
    out.push(0); // Borsh enum discriminant for ApprovalInner::Endorsement.
    out.extend_from_slice(&next_block_hash);
    out.extend_from_slice(&after_next_height.to_le_bytes());
    Ok(out)
}

fn strict_validator_key(public_key: &[u8; 32]) -> Result<VerifyingKey, &'static str> {
    let key = VerifyingKey::from_bytes(public_key).map_err(|_| "bad validator key")?;
    if key.is_weak() || !key.to_edwards().is_torsion_free() {
        return Err("weak validator key");
    }
    Ok(key)
}

/// Verify authentic Ed25519 approvals and strict >2/3 stake threshold
/// against a CALLER-SUPPLIED (potentially fake) validator set.
/// Do not infer validator provenance, epoch safety, or NEAR finality from Ok.
pub fn verify_approvals(
    validators: &[Validator],
    approvals: &[Option<[u8; 64]>],
    message: &[u8],
) -> Result<(), &'static str> {
    if validators.is_empty()
        || validators.len() > 4096
        || message.len() != 41
        || message[0] != 0
        || approvals.len() > validators.len().saturating_add(4096)
    {
        return Err("invalid approval shape");
    }
    let mut keys = HashSet::new();
    let mut total_stake: u128 = 0;
    let mut approved_stake: u128 = 0;
    for (i, validator) in validators.iter().enumerate() {
        if validator.stake == 0 || !keys.insert(validator.public_key) {
            return Err("zero stake or repeated validator public key");
        }
        let public = strict_validator_key(&validator.public_key)?;
        total_stake = total_stake.checked_add(validator.stake).ok_or("stake overflow")?;
        if let Some(bytes) = approvals.get(i).and_then(Option::as_ref) {
            let sig = Signature::from_bytes(bytes);
            public
                .verify_strict(message, &sig)
                .map_err(|_| "bad approval signature")?;
            approved_stake = approved_stake
                .checked_add(validator.stake)
                .ok_or("signed stake overflow")?;
        }
    }
    // Strictly greater than 2/3 of TOTAL stake, never just present signers.
    let lhs = approved_stake.checked_mul(3).ok_or("signed stake threshold overflow")?;
    let rhs = total_stake.checked_mul(2).ok_or("total stake threshold overflow")?;
    if lhs <= rhs {
        return Err("insufficient approved stake");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    fn hex_key(value: &str) -> [u8; 32] {
        assert_eq!(value.len(), 64);
        let mut output = [0u8; 32];
        for (index, byte) in output.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16).unwrap();
        }
        output
    }

    fn identity_key() -> [u8; 32] {
        let mut key = [0u8; 32];
        key[0] = 1;
        key
    }

    fn voters() -> ([SigningKey; 3], Vec<Validator>) {
        let keys = [
            SigningKey::from_bytes(&[1; 32]),
            SigningKey::from_bytes(&[2; 32]),
            SigningKey::from_bytes(&[3; 32]),
        ];
        let validators = keys
            .iter()
            .map(|k| Validator {
                public_key: k.verifying_key().to_bytes(),
                stake: 100u128,
            })
            .collect();
        (keys, validators)
    }

    #[test]
    fn authentic_two_of_three_is_rejected_when_exactly_two_thirds() {
        let (keys, validators) = voters();
        let next_hash = [7u8; 32];
        let message = approval_message(next_hash, 42).unwrap();
        let approvals = vec![
            Some(keys[0].sign(&message).to_bytes()),
            Some(keys[1].sign(&message).to_bytes()),
            None,
        ];
        assert!(verify_approvals(&validators, &approvals, &message).is_err());
    }

    #[test]
    fn three_valid_signatures_exceed_threshold() {
        let (keys, validators) = voters();
        let message = approval_message([7u8; 32], 42).unwrap();
        let approvals = keys
            .iter()
            .map(|k| Some(k.sign(&message).to_bytes()))
            .collect::<Vec<_>>();
        assert!(verify_approvals(&validators, &approvals, &message).is_ok());
    }

    #[test]
    fn forged_signature_and_wrong_height_fail() {
        let (keys, validators) = voters();
        let message = approval_message([7u8; 32], 42).unwrap();
        let wrong = approval_message([7u8; 32], 43).unwrap();
        let approvals = keys
            .iter()
            .map(|k| Some(k.sign(&message).to_bytes()))
            .collect::<Vec<_>>();
        assert!(verify_approvals(&validators, &approvals, &wrong).is_err());
        let mut forged = approvals;
        forged[1] = Some(keys[1].sign(&wrong).to_bytes());
        assert!(verify_approvals(&validators, &forged, &message).is_err());
    }

    #[test]
    fn duplicate_keys_and_empty_quorum_fail() {
        let (keys, mut validators) = voters();
        let message = approval_message([7u8; 32], 42).unwrap();
        let approvals = keys
            .iter()
            .map(|k| Some(k.sign(&message).to_bytes()))
            .collect::<Vec<_>>();
        validators[1].public_key = validators[0].public_key;
        assert!(verify_approvals(&validators, &approvals, &message).is_err());
        assert!(verify_approvals(&[], &[], &message).is_err());
    }

    #[test]
    fn high_stake_signer_can_pass_threshold() {
        let (keys, mut validators) = voters();
        validators[0].stake = 500;
        let message = approval_message([7u8; 32], 42).unwrap();
        let approvals = vec![Some(keys[0].sign(&message).to_bytes()), None, None];
        assert!(verify_approvals(&validators, &approvals, &message).is_ok());
    }

    #[test]
    fn missing_trailing_approvals_are_implicit_none() {
        let (keys, validators) = voters();
        let message = approval_message([7u8; 32], 42).unwrap();
        let short = vec![
            Some(keys[0].sign(&message).to_bytes()),
            Some(keys[1].sign(&message).to_bytes()),
        ];
        assert!(verify_approvals(&validators, &short, &message).is_err());
    }

    #[test]
    fn a_short_approval_vector_can_pass_with_sufficient_signed_stake() {
        let (keys, mut validators) = voters();
        validators[0].stake = 500;
        let message = approval_message([7u8; 32], 42).unwrap();
        let short = vec![Some(keys[0].sign(&message).to_bytes())];
        assert!(verify_approvals(&validators, &short, &message).is_ok());
    }

    #[test]
    fn next_block_hash_uses_sha256_of_two_32_byte_inputs() {
        let first = [1u8; 32];
        let second = [2u8; 32];
        let mut raw = Vec::new();
        raw.extend_from_slice(&first);
        raw.extend_from_slice(&second);
        let expected: [u8; 32] = Sha256::digest(raw).into();
        assert_eq!(reconstruct_next_block_hash(first, second), expected);
        assert_ne!(reconstruct_next_block_hash(second, first), expected);
    }

    #[test]
    fn additional_unchecked_signers_never_increase_stake() {
        let (keys, validators) = voters();
        let message = approval_message([7u8; 32], 42).unwrap();
        let mut approvals = vec![Some(keys[0].sign(&message).to_bytes()), None, None];
        approvals.push(Some(keys[0].sign(&message).to_bytes()));
        assert!(verify_approvals(&validators, &approvals, &message).is_err());
    }

    #[test]
    fn malformed_inactive_validator_key_is_still_rejected() {
        let (keys, mut validators) = voters();
        validators[0].public_key = [
            0x5f, 0xec, 0xeb, 0x66, 0xff, 0xc8, 0x6f, 0x38, 0xd9, 0x52, 0x78, 0x6c, 0x6d, 0x69, 0x6c, 0x79, 0xc2, 0xdb,
            0xc2, 0x39, 0xdd, 0x4e, 0x91, 0xb4, 0x67, 0x29, 0xd7, 0x3a, 0x27, 0xfb, 0x57, 0xe9,
        ];
        validators[0].stake = 1;
        validators[1].stake = 500;
        validators[2].stake = 500;
        let message = approval_message([7u8; 32], 42).unwrap();
        let approvals = vec![
            None,
            Some(keys[1].sign(&message).to_bytes()),
            Some(keys[2].sign(&message).to_bytes()),
        ];
        assert!(verify_approvals(&validators, &approvals, &message).is_err());
    }

    #[test]
    fn weak_and_torsion_keys_are_rejected_even_when_not_signing() {
        let message = approval_message([7u8; 32], 42).unwrap();
        let weak_keys = [
            identity_key(),
            [0u8; 32],
            hex_key("ecffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff7f"),
            hex_key("c7176a703d4dd84fba3c0b760d10670f2a2053fa2c39ccc64ec7fd7792ac037a"),
            hex_key("26e8958fc2b227b045c3f489f2ef98f0d5dfac05d3c63339b13802886d53fc05"),
            hex_key("e13c37ce5dce819535f932cbc63fc224d60d129d8e0e4d90cad59438d2d7042a"),
            [0xffu8; 32],
        ];
        for public_key in weak_keys {
            let validators = [Validator { public_key, stake: 100 }];
            let error = verify_approvals(&validators, &[None], &message).unwrap_err();
            assert!(error.contains("key"), "weak key reached stake evaluation: {error}");
        }
    }

    #[test]
    fn identity_key_cannot_forge_a_universal_approval() {
        let identity = identity_key();
        let message = approval_message([7u8; 32], 42).unwrap();
        let mut forged = [0u8; 64];
        forged[..32].copy_from_slice(&identity);
        assert_eq!(
            verify_approvals(
                &[Validator {
                    public_key: identity,
                    stake: 100,
                }],
                &[Some(forged)],
                &message,
            ),
            Err("weak validator key")
        );
    }
}
