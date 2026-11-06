//! NEAR light-client *approval signature primitive*. No checkpoint, state,
//! epoch transitions or permission to attest/pay funds.

use ed25519_dalek::{Signature, Verifier, VerifyingKey};
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
        || approvals.len() < validators.len()
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
        total_stake = total_stake.checked_add(validator.stake).ok_or("stake overflow")?;
        if let Some(bytes) = approvals.get(i).and_then(Option::as_ref) {
            let public = VerifyingKey::from_bytes(&validator.public_key).map_err(|_| "bad validator key")?;
            let sig = Signature::from_bytes(bytes);
            public.verify(message, &sig).map_err(|_| "bad approval signature")?;
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
}
