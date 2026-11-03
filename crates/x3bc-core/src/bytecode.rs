//! The X3BC envelope's fixed header — the part a decoder must check before it trusts anything else.
//!
//! This moved here from `x3-common::bytecode` (which re-exports it) so that the canonical compiler
//! in `x3-lang/` and the chain's engine read one definition without either depending on Substrate.
//! One checksum algorithm, one set of version bounds, one feature-bit table (TICKET-108).
//!
//! # Versions
//!
//! A module's header carries the format version it was written as and the minimum loader version
//! it demands. Base modules are written as **1.0.0** and demand 1.0.0, byte-identical to every
//! artifact before the trading extension. A module carrying the trading section is written as
//! **1.1.0** and demands 1.1.0 ([`TRADING_VERSION`]), so a 1.0 loader — every runtime before this
//! one — refuses it instead of running it without its trading semantics. This loader reads up to
//! [`READER_VERSION`].

/// Magic bytes identifying X3 bytecode.
pub const MAGIC: &[u8; 4] = b"X3BC";

/// Fixed header length: magic, version, flags, checksum, min-version, features.
pub const HEADER_LEN: usize = 24;

/// Byte offset of the format version inside the header.
pub const VERSION_OFFSET: usize = 4;

/// Byte offset of the checksum inside the header.
pub const CHECKSUM_OFFSET: usize = 12;

/// Byte offset of the minimum-loader-version word inside the header.
pub const MIN_VERSION_OFFSET: usize = 16;

/// Byte offset of the feature-flags word inside the header.
pub const FEATURE_FLAGS_OFFSET: usize = 20;

/// Feature flag: the program's compiled policy requires private submission.
///
/// This is a *capability the artifact carries*, not a format change. AGENTS.md §11 says the
/// security policy has to come from the compiled artifact, and §12 that a check the runtime does
/// not interpret is worse than no check at all — so a bit set here means the loader must refuse the
/// module unless the execution context it is loading into actually offers a private channel. A
/// reader that reads the header and ignores this bit turns "this program demands privacy" into
/// "this program runs in the clear", which is the one direction a private submission policy must
/// never take.
pub const FEATURE_PRIVATE_SUBMISSION_REQUIRED: u32 = 1 << 8;

/// Feature flag: the module carries a trading section (format 1.1).
pub const FEATURE_TRADING_EXTENSION: u32 = 1 << 9;

/// Format version a base module is written as: major 1, minor 0, patch 0, which is what
/// `(major << 16) | (minor << 8) | patch` packs to.
pub const VERSION: u32 = 1 << 16;

/// Format version a module with the trading section is written as, and demands: 1.1.0.
pub const TRADING_VERSION: u32 = (1 << 16) | (1 << 8);

/// The newest format this loader reads and satisfies.
pub const READER_VERSION: u32 = TRADING_VERSION;

/// Oldest format version this loader can read.
pub const MIN_SUPPORTED_VERSION: u32 = VERSION;

/// First format version this loader cannot read: the next major.
pub const MAX_SUPPORTED_VERSION: u32 = 2 << 16;

/// The envelope's body checksum, exactly as the writer computes it.
///
/// A wrapping multiply-and-add over the bytes after the header. It is a corruption check, not a
/// security boundary — anyone can recompute it — and it is here so that the writer, the std reader
/// and the no-std reader cannot drift apart about what it means.
pub fn checksum(body: &[u8]) -> u32 {
    let mut sum: u32 = 0;
    for byte in body {
        sum = sum.wrapping_add(*byte as u32);
        sum = sum.wrapping_mul(31);
    }
    sum
}

/// Is a module that declares `version` readable by this loader?
///
/// Same major, no newer minor than [`READER_VERSION`]. Patch differences are compatible: the
/// format's own semantic versioning says so ("patch version changes are bug fixes (fully
/// compatible)"), and comparing the whole word refused a *patch* bump — so `1.0.1` was accepted by
/// `x3-backend` and refused by the no-std reader that executes on chain (TICKET-137).
pub const fn version_is_readable(version: u32) -> bool {
    (version >> 16) == (READER_VERSION >> 16)
        && ((version >> 8) & 0xFF) <= ((READER_VERSION >> 8) & 0xFF)
}

/// Does this loader satisfy a module that requires at least `min_version`?
pub const fn loader_satisfies(min_version: u32) -> bool {
    min_version <= READER_VERSION
}

/// Would a **1.0** loader — one without the trading extension — satisfy `min_version`? Used to
/// prove that every pre-extension runtime refuses a trading module.
pub const fn loader_satisfies_base(min_version: u32) -> bool {
    min_version <= VERSION
}

fn header_word(bytes: &[u8], offset: usize) -> Option<u32> {
    if bytes.len() < HEADER_LEN || &bytes[0..4] != MAGIC {
        return None;
    }
    let mut word = [0u8; 4];
    word.copy_from_slice(&bytes[offset..offset + 4]);
    Some(u32::from_le_bytes(word))
}

/// The feature-flags word of an X3BC header, or `None` when these bytes are not a header.
///
/// `None` means "not an X3BC module", not "no demands": the magic and the header length are the
/// two facts a caller needs before it can read the field at all.
pub fn feature_flags(bytes: &[u8]) -> Option<u32> {
    header_word(bytes, FEATURE_FLAGS_OFFSET)
}

/// The minimum loader version an X3BC header demands, or `None` when these bytes are not a header.
pub fn min_version(bytes: &[u8]) -> Option<u32> {
    header_word(bytes, MIN_VERSION_OFFSET)
}

/// Does this payload declare that its compiled policy requires private submission?
///
/// `false` covers both "the module does not demand it" and "this is not a module"; a caller that
/// needs to tell those apart uses [`feature_flags`] directly.
pub fn requires_private_submission(bytes: &[u8]) -> bool {
    matches!(
        feature_flags(bytes),
        Some(flags) if flags & FEATURE_PRIVATE_SUBMISSION_REQUIRED != 0
    )
}

/// Recompute the header checksum after the body has been edited.
///
/// For tools and for tests that need a corrupted body to reach the decoder *behind* the corruption
/// check. Does nothing to bytes that are not an X3BC header.
pub fn reseal(bytes: &mut [u8]) {
    if bytes.len() < HEADER_LEN || &bytes[0..4] != MAGIC {
        return;
    }
    let sum = checksum(&bytes[HEADER_LEN..]);
    bytes[CHECKSUM_OFFSET..CHECKSUM_OFFSET + 4].copy_from_slice(&sum.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec::Vec;

    /// A header whose feature word the caller chooses, with a body whose checksum is genuine so the
    /// reader under test is exercised rather than a malformed blob.
    fn header_with_features(features: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(&VERSION.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes()); // flags
        bytes.extend_from_slice(&checksum(&[]).to_le_bytes());
        bytes.extend_from_slice(&VERSION.to_le_bytes()); // min_version
        bytes.extend_from_slice(&features.to_le_bytes());
        assert_eq!(bytes.len(), HEADER_LEN);
        bytes
    }

    #[test]
    fn the_private_submission_bit_is_read_from_the_feature_word() {
        // Both directions, because a reader that always answers `false` is exactly the bug this flag
        // exists to fix: the demand would be carried and ignored.
        assert!(requires_private_submission(&header_with_features(
            FEATURE_PRIVATE_SUBMISSION_REQUIRED
        )));
        assert!(!requires_private_submission(&header_with_features(0)));
        // A different flag in the same word must not be mistaken for this one.
        assert!(!requires_private_submission(&header_with_features(1 << 7)));
        assert!(!requires_private_submission(&header_with_features(
            FEATURE_TRADING_EXTENSION
        )));
        // A neighbouring bit inside the same word must not mask the demand either.
        assert!(requires_private_submission(&header_with_features(
            FEATURE_PRIVATE_SUBMISSION_REQUIRED | (1 << 7)
        )));
    }

    #[test]
    fn a_payload_that_is_not_a_module_is_not_a_demand() {
        assert_eq!(feature_flags(&[]), None);
        assert_eq!(feature_flags(b"X3BC"), None);
        assert_eq!(feature_flags(&[0u8; HEADER_LEN]), None, "wrong magic");
        let mut short = header_with_features(FEATURE_PRIVATE_SUBMISSION_REQUIRED);
        short.truncate(HEADER_LEN - 1);
        assert_eq!(feature_flags(&short), None);
        assert_eq!(min_version(&short), None);
        assert!(!requires_private_submission(b"not a program at all"));
    }

    #[test]
    fn the_checksum_is_order_sensitive_and_deterministic() {
        assert_ne!(checksum(b"ab"), checksum(b"ba"));
        assert_eq!(checksum(b"ab"), checksum(b"ab"));
        assert_eq!(checksum(&[]), 0);
    }

    #[test]
    fn a_patch_bump_is_readable_and_an_unknown_minor_is_not() {
        // The field layout: major << 16 | minor << 8 | patch.
        assert!(
            version_is_readable(VERSION + 1),
            "a patch bump is compatible"
        );
        assert!(
            version_is_readable(READER_VERSION + 1),
            "a patch bump of the newest minor too"
        );
        assert!(
            !version_is_readable(READER_VERSION + 0x100),
            "a minor newer than this loader knows is not readable"
        );
        assert!(
            !version_is_readable(VERSION + 0x1_0000),
            "a newer major is not readable by this loader"
        );
    }

    #[test]
    fn version_bounds_match_the_packing() {
        assert_eq!(VERSION, 0x0001_0000);
        assert_eq!(TRADING_VERSION, 0x0001_0100);
        assert_eq!(READER_VERSION, TRADING_VERSION);
        assert_eq!(MAX_SUPPORTED_VERSION, 0x0002_0000);
        assert!(version_is_readable(VERSION));
        assert!(version_is_readable(TRADING_VERSION));
        assert!(version_is_readable(MIN_SUPPORTED_VERSION));
        assert!(
            !version_is_readable(MAX_SUPPORTED_VERSION),
            "the next major is read by a loader that knows it"
        );
        assert!(!version_is_readable((1 << 16) | (2 << 8)));
        assert!(loader_satisfies(VERSION));
        assert!(loader_satisfies(TRADING_VERSION));
        assert!(!loader_satisfies((1 << 16) | (2 << 8)));
        assert!(!loader_satisfies(MAX_SUPPORTED_VERSION));
    }

    #[test]
    fn a_base_loader_refuses_a_trading_module() {
        assert!(loader_satisfies_base(VERSION));
        assert!(!loader_satisfies_base(TRADING_VERSION));
    }

    #[test]
    fn reseal_repairs_only_the_checksum() {
        let mut bytes = header_with_features(0);
        bytes.extend_from_slice(b"body");
        bytes[CHECKSUM_OFFSET] ^= 0xFF;
        reseal(&mut bytes);
        let mut expected = [0u8; 4];
        expected.copy_from_slice(&bytes[CHECKSUM_OFFSET..CHECKSUM_OFFSET + 4]);
        assert_eq!(u32::from_le_bytes(expected), checksum(b"body"));
        let mut not_a_module = *b"not a module at all, longer than a header";
        let before = not_a_module;
        reseal(&mut not_a_module);
        assert_eq!(not_a_module, before);
    }
}
