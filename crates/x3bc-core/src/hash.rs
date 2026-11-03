//! The one digest the canonical core binds artifacts, sources, state and receipts with.

use sha2::{Digest, Sha256};

/// SHA-256 of `bytes`.
pub fn sha256(bytes: &[u8]) -> [u8; 32] {
    let mut out = [0u8; 32];
    out.copy_from_slice(&Sha256::digest(bytes));
    out
}

/// SHA-256 of `bytes` under a domain tag, so a digest computed for one purpose can never be
/// presented as one computed for another.
pub fn sha256_domain(domain: &[u8], bytes: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update((domain.len() as u32).to_le_bytes());
    hasher.update(domain);
    hasher.update(bytes);
    let mut out = [0u8; 32];
    out.copy_from_slice(&hasher.finalize());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_matches_the_standard_vector() {
        // SHA-256("abc")
        assert_eq!(sha256(b"abc")[..4], [0xba, 0x78, 0x16, 0xbf],);
    }

    #[test]
    fn domains_separate_digests() {
        assert_ne!(sha256_domain(b"a", b"bc"), sha256_domain(b"ab", b"c"));
        assert_ne!(sha256_domain(b"x", b"m"), sha256(b"m"));
    }
}
