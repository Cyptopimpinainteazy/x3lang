//! X3 Bytecode Binary Format
//!
//! Defines the structure of compiled X3 bytecode modules with semantic versioning.
//!
//! # Binary Layout (v2+)
//!
//! ```text
//! ┌────────────────────────────────────────┐
//! │ Header (24 bytes)                      │
//! │   Magic: "X3BC" (4 bytes)              │
//! │   Version: u32 (packed semver)         │
//! │   Flags: u32                           │
//! │   Checksum: u32                        │
//! │   MinVersion: u32 (min required)       │
//! │   FeatureFlags: u32                    │
//! ├────────────────────────────────────────┤
//! │ Section Table                          │
//! │   Section count: u16                   │
//! │   [Section entries...]                 │
//! ├────────────────────────────────────────┤
//! │ Constant Pool Section                  │
//! │   Entry count: u32                     │
//! │   [Constant entries...]                │
//! ├────────────────────────────────────────┤
//! │ Function Table Section                 │
//! │   Entry count: u32                     │
//! │   [Function entries...]                │
//! ├────────────────────────────────────────┤
//! │ Global Table Section                   │
//! │   Entry count: u32                     │
//! │   [Global entries...]                  │
//! ├────────────────────────────────────────┤
//! │ Instruction Stream Section             │
//! │   Size: u32                            │
//! │   [Encoded instructions...]            │
//! ├────────────────────────────────────────┤
//! │ Debug Info Section (optional)          │
//! │   Source map entries                   │
//! │   Symbol names                         │
//! └────────────────────────────────────────┘
//! ```
//!
//! # Version Compatibility
//!
//! - Major version changes are breaking (new loader required)
//! - Minor version changes add features (backward compatible)
//! - Patch version changes are bug fixes (fully compatible)
//!
//! Bytecode declares a `min_version` - the loader must be at least that version.

use crate::prelude::*;
use alloc::collections::BTreeMap;
use serde::{Deserialize, Serialize};

use crate::error::{
    FormatError as BackendError, FormatErrorKind as BackendErrorKind, FormatResult as BackendResult,
};
use crate::opcode::{ConstIdx, FuncIdx};

/// Magic bytes identifying X3 bytecode files.
///
/// The header's constants and checksum live in `x3-common::bytecode` now: the no-std decoder in
/// `x3-integration::mini_x3` speaks the same format and cannot depend on this crate (TICKET-108).
pub const MAGIC: &[u8; 4] = crate::bytecode::MAGIC;

/// Current bytecode format version (semantic: major.minor.patch packed as u32).
/// Format: (major << 16) | (minor << 8) | patch
pub const VERSION: u32 = crate::bytecode::VERSION;

/// Minimum version this loader can read.
pub const MIN_SUPPORTED_VERSION: u32 = crate::bytecode::MIN_SUPPORTED_VERSION;

/// Maximum version this loader can read (exclusive next major).
pub const MAX_SUPPORTED_VERSION: u32 = crate::bytecode::MAX_SUPPORTED_VERSION;

/// Semantic version information for bytecode format.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct VersionInfo {
    /// Major version - breaking changes.
    pub major: u8,
    /// Minor version - backward compatible additions.
    pub minor: u8,
    /// Patch version - bug fixes.
    pub patch: u8,
}

impl VersionInfo {
    /// Create a new version.
    pub const fn new(major: u8, minor: u8, patch: u8) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }

    /// Pack version into u32 for storage.
    pub const fn to_packed(self) -> u32 {
        ((self.major as u32) << 16) | ((self.minor as u32) << 8) | (self.patch as u32)
    }

    /// Unpack version from u32.
    pub const fn from_packed(packed: u32) -> Self {
        Self {
            major: ((packed >> 16) & 0xFF) as u8,
            minor: ((packed >> 8) & 0xFF) as u8,
            patch: (packed & 0xFF) as u8,
        }
    }

    /// The format version a base module is written as (1.0.0).
    pub const fn current() -> Self {
        Self::from_packed(VERSION)
    }

    /// The newest format this loader reads and satisfies (1.1.0, the trading extension). Reading
    /// and loading decisions are taken against this, never against [`VersionInfo::current`].
    pub const fn reader() -> Self {
        Self::from_packed(crate::bytecode::READER_VERSION)
    }

    /// Check if this version is compatible with another (can read its bytecode).
    pub fn can_read(&self, bytecode_version: VersionInfo) -> bool {
        // Same major version required
        if self.major != bytecode_version.major {
            return false;
        }
        // We can read older minor versions
        if self.minor < bytecode_version.minor {
            return false;
        }
        true
    }

    /// Check if bytecode needs this version or newer.
    pub fn satisfies(&self, min_required: VersionInfo) -> bool {
        if self.major > min_required.major {
            return true;
        }
        if self.major < min_required.major {
            return false;
        }
        // Same major
        if self.minor > min_required.minor {
            return true;
        }
        if self.minor < min_required.minor {
            return false;
        }
        // Same minor
        self.patch >= min_required.patch
    }
}

impl core::fmt::Display for VersionInfo {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

impl Default for VersionInfo {
    fn default() -> Self {
        Self::current()
    }
}

/// Feature flags indicating which bytecode features are used.
/// These allow forward compatibility - a loader can skip unknown features.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeatureFlags(pub u32);

impl FeatureFlags {
    /// No special features.
    pub const NONE: u32 = 0;
    /// Uses extended opcodes (v1.1+).
    pub const EXTENDED_OPCODES: u32 = 1 << 0;
    /// Uses typed constants (v1.1+).
    pub const TYPED_CONSTANTS: u32 = 1 << 1;
    /// Uses inline caching hints (v1.2+).
    pub const INLINE_CACHE: u32 = 1 << 2;
    /// Uses cross-VM call encoding (v1.2+).
    pub const CROSS_VM_CALLS: u32 = 1 << 3;
    /// Uses gas metering annotations (v1.0+).
    pub const GAS_METERING: u32 = 1 << 4;
    /// Uses custom sections (v1.1+).
    pub const CUSTOM_SECTIONS: u32 = 1 << 5;
    /// Uses compressed constants (v1.2+).
    pub const COMPRESSED_CONSTS: u32 = 1 << 6;
    /// Uses simd operations (v1.3+).
    pub const SIMD_OPS: u32 = 1 << 7;
    /// The program's compiled policy requires private submission.
    ///
    /// The bit is defined in `x3-common::bytecode` and aliased here so the std writer and the
    /// no-std reader name the same bit; see that definition for why it is a capability rather
    /// than a format version.
    pub const PRIVATE_SUBMISSION_REQUIRED: u32 =
        crate::bytecode::FEATURE_PRIVATE_SUBMISSION_REQUIRED;
    /// The module carries a trading section (format 1.1).
    pub const TRADING_EXTENSION: u32 = crate::bytecode::FEATURE_TRADING_EXTENSION;

    pub fn new() -> Self {
        Self(0)
    }

    pub fn set(&mut self, flag: u32) {
        self.0 |= flag;
    }

    pub fn has(&self, flag: u32) -> bool {
        (self.0 & flag) != 0
    }

    /// Get the minimum version required to support these features.
    pub fn min_version_required(&self) -> VersionInfo {
        if self.has(Self::SIMD_OPS) {
            return VersionInfo::new(1, 3, 0);
        }
        if self.has(Self::INLINE_CACHE)
            || self.has(Self::CROSS_VM_CALLS)
            || self.has(Self::COMPRESSED_CONSTS)
        {
            return VersionInfo::new(1, 2, 0);
        }
        if self.has(Self::EXTENDED_OPCODES)
            || self.has(Self::TYPED_CONSTANTS)
            || self.has(Self::CUSTOM_SECTIONS)
        {
            return VersionInfo::new(1, 1, 0);
        }
        VersionInfo::new(1, 0, 0)
    }

    /// Check if the current runtime supports all these features.
    pub fn supported_by_current(&self) -> bool {
        let current = VersionInfo::reader();
        current.satisfies(self.min_version_required())
    }
}

/// Maximum bytecode size (16 MB).
pub const MAX_BYTECODE_SIZE: usize = 16 * 1024 * 1024;

/// Maximum constant pool entries.
pub const MAX_CONST_POOL: u32 = 65536;

/// Maximum functions per module.
pub const MAX_FUNCTIONS: u32 = 65536;

/// Maximum string length in constant pool.
pub const MAX_STRING_LEN: usize = 65535;

/// Bytecode module flags.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModuleFlags(pub u32);

impl ModuleFlags {
    /// Module contains debug information.
    pub const DEBUG_INFO: u32 = 1 << 0;
    /// Module uses EVM intrinsics.
    pub const USES_EVM: u32 = 1 << 1;
    /// Module uses SVM intrinsics.
    pub const USES_SVM: u32 = 1 << 2;
    /// Module uses atomic blocks.
    pub const USES_ATOMIC: u32 = 1 << 3;
    /// Module is deterministic (no randomness/timing).
    pub const DETERMINISTIC: u32 = 1 << 4;

    pub fn new() -> Self {
        Self(0)
    }

    pub fn set(&mut self, flag: u32) {
        self.0 |= flag;
    }

    pub fn has(&self, flag: u32) -> bool {
        (self.0 & flag) != 0
    }
}

/// A complete bytecode module ready for execution.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BytecodeModule {
    /// Module version (semantic versioned).
    pub version: VersionInfo,
    /// Minimum version required to load this module.
    pub min_version: VersionInfo,
    /// Module flags.
    pub flags: ModuleFlags,
    /// Feature flags indicating which IR features are used.
    pub features: FeatureFlags,
    /// Constant pool.
    pub const_pool: ConstPool,
    /// Function table.
    pub functions: Vec<FunctionEntry>,
    /// Global variable table.
    pub globals: Vec<GlobalEntry>,
    /// Encoded instruction stream.
    pub code: Vec<u8>,
    /// Debug information (if present).
    pub debug_info: Option<DebugInfo>,
    /// Optional metadata (compiler info, timestamps, etc.).
    pub metadata: Option<ModuleMetadata>,
    /// The trading section (format 1.1), present exactly when the header's
    /// `FEATURE_TRADING_EXTENSION` bit is set.
    #[serde(skip)]
    pub trading: Option<crate::trading::TradingSection>,
}

/// Module metadata for debugging and tooling.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ModuleMetadata {
    /// Name of the compiler that produced this module.
    pub compiler: String,
    /// Compiler version.
    pub compiler_version: String,
    /// Compilation timestamp (Unix epoch seconds).
    pub compiled_at: u64,
    /// Original source file name.
    pub source_file: Option<String>,
    /// Git commit hash of source.
    pub source_hash: Option<String>,
    /// Optimization level (0-3).
    pub opt_level: u8,
    /// Custom key-value annotations.
    pub annotations: BTreeMap<String, String>,
}

impl BytecodeModule {
    /// Create a new empty module.
    pub fn new() -> Self {
        Self {
            version: VersionInfo::current(),
            min_version: VersionInfo::new(1, 0, 0),
            flags: ModuleFlags::new(),
            features: FeatureFlags::new(),
            const_pool: ConstPool::new(),
            functions: Vec::new(),
            globals: Vec::new(),
            code: Vec::new(),
            debug_info: None,
            metadata: None,
            trading: None,
        }
    }

    /// Create a module with specific version.
    pub fn with_version(version: VersionInfo) -> Self {
        Self {
            version,
            min_version: version,
            ..Self::new()
        }
    }

    /// Set the features used and auto-compute minimum version.
    pub fn set_features(&mut self, features: FeatureFlags) {
        self.features = features;
        // Auto-update min_version based on features
        let required = features.min_version_required();
        if required > self.min_version {
            self.min_version = required;
        }
    }

    /// Check if this module can be loaded by the current runtime.
    pub fn is_compatible(&self) -> bool {
        let current = VersionInfo::reader();
        current.can_read(self.version) && current.satisfies(self.min_version)
    }

    /// Serialize module to bytes (v2 format with extended header).
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::new();

        // Header (24 bytes)
        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(&self.version.to_packed().to_le_bytes());
        bytes.extend_from_slice(&self.flags.0.to_le_bytes());
        // Placeholder for checksum (filled at end)
        let checksum_pos = bytes.len();
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&self.min_version.to_packed().to_le_bytes());
        bytes.extend_from_slice(&self.features.0.to_le_bytes());

        // Constant pool
        self.write_const_pool(&mut bytes);

        // Function table
        self.write_functions(&mut bytes);

        // Global table
        self.write_globals(&mut bytes);

        // Code section
        bytes.extend_from_slice(&(self.code.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&self.code);

        // Debug info (optional)
        if let Some(ref debug) = self.debug_info {
            bytes.push(1); // Has debug info
            self.write_debug_info(&mut bytes, debug);
        } else {
            bytes.push(0); // No debug info
        }

        // Metadata (optional, v1.1+)
        if let Some(ref meta) = self.metadata {
            bytes.push(1); // Has metadata
            self.write_metadata(&mut bytes, meta);
        } else {
            bytes.push(0); // No metadata
        }

        // Trading section (format 1.1): length-prefixed canonical bytes, last in the module.
        if let Some(ref section) = self.trading {
            let encoded = crate::trading::encode_section(section);
            bytes.extend_from_slice(&(encoded.len() as u32).to_le_bytes());
            bytes.extend_from_slice(&encoded);
        }

        // Compute and write checksum
        let checksum = self.compute_checksum(&bytes[24..]); // Skip header
        bytes[checksum_pos..checksum_pos + 4].copy_from_slice(&checksum.to_le_bytes());

        bytes
    }

    /// Deserialize module from bytes.
    pub fn from_bytes(bytes: &[u8]) -> BackendResult<Self> {
        if bytes.len() < 24 {
            return Err(BackendError::without_span(BackendErrorKind::UnexpectedEof));
        }

        // Verify magic
        if &bytes[0..4] != MAGIC {
            return Err(BackendError::without_span(BackendErrorKind::InvalidMagic));
        }

        let version_packed = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
        let version = VersionInfo::from_packed(version_packed);

        // Check if we support this version. The gate is `x3-common`'s, on the full `u32`: the
        // unpacked `VersionInfo` keeps eight bits per field, so `0x4301_0000` read as 1.0.0 here while
        // the runtime's reader (which applies this same gate) refused it — found by the
        // `x3bc_engines` fuzz target.
        if !crate::bytecode::version_is_readable(version_packed) {
            return Err(BackendError::without_span(
                BackendErrorKind::UnsupportedVersion(version_packed),
            ));
        }

        let flags = ModuleFlags(u32::from_le_bytes([
            bytes[8], bytes[9], bytes[10], bytes[11],
        ]));
        // The writer computes this over everything after the header; a reader that ignores it
        // accepts a body corrupted in transit (TICKET-108). Verified here now.
        let checksum = u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]);
        let expected = crate::bytecode::checksum(&bytes[crate::bytecode::HEADER_LEN..]);
        if checksum != expected {
            return Err(BackendError::without_span(
                BackendErrorKind::ChecksumMismatch {
                    expected,
                    found: checksum,
                },
            ));
        }
        let min_version_packed = u32::from_le_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
        let min_version = VersionInfo::from_packed(min_version_packed);
        let features = FeatureFlags(u32::from_le_bytes([
            bytes[20], bytes[21], bytes[22], bytes[23],
        ]));

        // Check minimum version requirement
        // Also on the full `u32`, for the same reason as the version gate above.
        if !crate::bytecode::loader_satisfies(min_version_packed) {
            return Err(BackendError::without_span(
                BackendErrorKind::UnsupportedVersion(min_version_packed),
            ));
        }

        let mut offset = 24;

        // Read constant pool
        let (const_pool, new_offset) = Self::read_const_pool(bytes, offset)?;
        offset = new_offset;

        // Read function table
        let (functions, new_offset) = Self::read_functions(bytes, offset)?;
        offset = new_offset;

        // Read global table
        let (globals, new_offset) = Self::read_globals(bytes, offset)?;
        offset = new_offset;

        // Read code section
        if offset + 4 > bytes.len() {
            return Err(BackendError::without_span(BackendErrorKind::UnexpectedEof));
        }
        let code_len = u32::from_le_bytes([
            bytes[offset],
            bytes[offset + 1],
            bytes[offset + 2],
            bytes[offset + 3],
        ]) as usize;
        offset += 4;

        if offset + code_len > bytes.len() {
            return Err(BackendError::without_span(BackendErrorKind::UnexpectedEof));
        }
        let code = bytes[offset..offset + code_len].to_vec();
        offset += code_len;

        // A trading module is read strictly: both optional-section markers must be present, so the
        // trading section's position is fixed and nothing before it can be skipped by accident.
        let trading_module = features.has(FeatureFlags::TRADING_EXTENSION);
        let strict_marker = |offset: usize| -> BackendResult<()> {
            match bytes.get(offset) {
                Some(0) | Some(1) => Ok(()),
                Some(_) => Err(BackendError::without_span(
                    BackendErrorKind::CorruptedBytecode { offset },
                )),
                None => Err(BackendError::without_span(BackendErrorKind::UnexpectedEof)),
            }
        };
        if trading_module {
            strict_marker(offset)?;
        }

        // Read debug info
        let debug_info = if offset < bytes.len() && bytes[offset] == 1 {
            offset += 1;
            let (debug, new_offset) = Self::read_debug_info(bytes, offset)?;
            offset = new_offset;
            Some(debug)
        } else {
            if offset < bytes.len() {
                offset += 1;
            }
            None
        };

        if trading_module {
            strict_marker(offset)?;
        }

        // Read metadata (v1.1+)
        let metadata = if offset < bytes.len() && bytes[offset] == 1 {
            offset += 1;
            let (meta, new_offset) = Self::read_metadata(bytes, offset)?;
            offset = new_offset;
            Some(meta)
        } else {
            if offset < bytes.len() {
                offset += 1;
            }
            None
        };

        // Trading section: present exactly when the header says so, and the last thing in the module.
        let trading = if trading_module {
            if offset + 4 > bytes.len() {
                return Err(BackendError::without_span(BackendErrorKind::UnexpectedEof));
            }
            let len = u32::from_le_bytes([
                bytes[offset],
                bytes[offset + 1],
                bytes[offset + 2],
                bytes[offset + 3],
            ]) as usize;
            offset += 4;
            let end = offset
                .checked_add(len)
                .ok_or(BackendError::without_span(BackendErrorKind::UnexpectedEof))?;
            if end != bytes.len() {
                return Err(BackendError::without_span(
                    BackendErrorKind::TradingSection {
                        offset,
                        reason: "section length does not end the module".into(),
                    },
                ));
            }
            let section = crate::trading::decode_section(&bytes[offset..end]).map_err(|error| {
                BackendError::without_span(BackendErrorKind::TradingSection {
                    offset,
                    reason: format!("{error:?}"),
                })
            })?;
            if min_version.to_packed() < crate::bytecode::TRADING_VERSION {
                return Err(BackendError::without_span(
                    BackendErrorKind::TradingSection {
                        offset,
                        reason: "a trading module must demand format 1.1".into(),
                    },
                ));
            }
            Some(section)
        } else {
            None
        };

        Ok(Self {
            version,
            min_version,
            flags,
            features,
            const_pool,
            functions,
            globals,
            code,
            debug_info,
            metadata,
            trading,
        })
    }

    fn write_const_pool(&self, bytes: &mut Vec<u8>) {
        bytes.extend_from_slice(&(self.const_pool.entries.len() as u32).to_le_bytes());
        for entry in &self.const_pool.entries {
            match entry {
                ConstValue::Integer(v) => {
                    bytes.push(0);
                    bytes.extend_from_slice(&v.to_le_bytes());
                }
                ConstValue::Float(v) => {
                    bytes.push(1);
                    bytes.extend_from_slice(&v.to_le_bytes());
                }
                ConstValue::String(s) => {
                    bytes.push(2);
                    bytes.extend_from_slice(&(s.len() as u32).to_le_bytes());
                    bytes.extend_from_slice(s.as_bytes());
                }
                ConstValue::Bool(b) => {
                    bytes.push(3);
                    bytes.push(if *b { 1 } else { 0 });
                }
                ConstValue::Bytes(b) => {
                    bytes.push(4);
                    bytes.extend_from_slice(&(b.len() as u32).to_le_bytes());
                    bytes.extend_from_slice(b);
                }
            }
        }
    }

    fn read_const_pool(bytes: &[u8], mut offset: usize) -> BackendResult<(ConstPool, usize)> {
        if offset + 4 > bytes.len() {
            return Err(BackendError::without_span(BackendErrorKind::UnexpectedEof));
        }
        let count = u32::from_le_bytes([
            bytes[offset],
            bytes[offset + 1],
            bytes[offset + 2],
            bytes[offset + 3],
        ]) as usize;
        offset += 4;

        let mut pool = ConstPool::new();
        for _ in 0..count {
            if offset >= bytes.len() {
                return Err(BackendError::without_span(BackendErrorKind::UnexpectedEof));
            }
            let tag = bytes[offset];
            offset += 1;

            let value = match tag {
                0 => {
                    // Integer
                    if offset + 8 > bytes.len() {
                        return Err(BackendError::without_span(BackendErrorKind::UnexpectedEof));
                    }
                    let v = i64::from_le_bytes([
                        bytes[offset],
                        bytes[offset + 1],
                        bytes[offset + 2],
                        bytes[offset + 3],
                        bytes[offset + 4],
                        bytes[offset + 5],
                        bytes[offset + 6],
                        bytes[offset + 7],
                    ]);
                    offset += 8;
                    ConstValue::Integer(v)
                }
                1 => {
                    // Float
                    if offset + 8 > bytes.len() {
                        return Err(BackendError::without_span(BackendErrorKind::UnexpectedEof));
                    }
                    let v = f64::from_le_bytes([
                        bytes[offset],
                        bytes[offset + 1],
                        bytes[offset + 2],
                        bytes[offset + 3],
                        bytes[offset + 4],
                        bytes[offset + 5],
                        bytes[offset + 6],
                        bytes[offset + 7],
                    ]);
                    offset += 8;
                    ConstValue::Float(v)
                }
                2 => {
                    // String
                    if offset + 4 > bytes.len() {
                        return Err(BackendError::without_span(BackendErrorKind::UnexpectedEof));
                    }
                    let len = u32::from_le_bytes([
                        bytes[offset],
                        bytes[offset + 1],
                        bytes[offset + 2],
                        bytes[offset + 3],
                    ]) as usize;
                    offset += 4;
                    if offset + len > bytes.len() {
                        return Err(BackendError::without_span(BackendErrorKind::UnexpectedEof));
                    }
                    let s = String::from_utf8_lossy(&bytes[offset..offset + len]).to_string();
                    offset += len;
                    ConstValue::String(s)
                }
                3 => {
                    // Bool
                    if offset >= bytes.len() {
                        return Err(BackendError::without_span(BackendErrorKind::UnexpectedEof));
                    }
                    let b = bytes[offset] != 0;
                    offset += 1;
                    ConstValue::Bool(b)
                }
                4 => {
                    // Bytes
                    if offset + 4 > bytes.len() {
                        return Err(BackendError::without_span(BackendErrorKind::UnexpectedEof));
                    }
                    let len = u32::from_le_bytes([
                        bytes[offset],
                        bytes[offset + 1],
                        bytes[offset + 2],
                        bytes[offset + 3],
                    ]) as usize;
                    offset += 4;
                    if offset + len > bytes.len() {
                        return Err(BackendError::without_span(BackendErrorKind::UnexpectedEof));
                    }
                    let b = bytes[offset..offset + len].to_vec();
                    offset += len;
                    ConstValue::Bytes(b)
                }
                _ => {
                    return Err(BackendError::without_span(
                        BackendErrorKind::CorruptedBytecode { offset: offset - 1 },
                    ));
                }
            };
            pool.entries.push(value);
        }

        Ok((pool, offset))
    }

    fn write_functions(&self, bytes: &mut Vec<u8>) {
        bytes.extend_from_slice(&(self.functions.len() as u32).to_le_bytes());
        for func in &self.functions {
            // Name length + name
            bytes.extend_from_slice(&(func.name.len() as u16).to_le_bytes());
            bytes.extend_from_slice(func.name.as_bytes());
            // Entry point
            bytes.extend_from_slice(&func.entry_point.to_le_bytes());
            // Parameter count
            bytes.push(func.param_count);
            // Local count
            bytes.extend_from_slice(&func.local_count.to_le_bytes());
            // Max stack
            bytes.extend_from_slice(&func.max_stack.to_le_bytes());
            // Return type tag (simplified: 0=void, 1=int, 2=float, 3=bool, 4=other)
            bytes.push(func.return_type_tag);
        }
    }

    fn read_functions(
        bytes: &[u8],
        mut offset: usize,
    ) -> BackendResult<(Vec<FunctionEntry>, usize)> {
        if offset + 4 > bytes.len() {
            return Err(BackendError::without_span(BackendErrorKind::UnexpectedEof));
        }
        let count = u32::from_le_bytes([
            bytes[offset],
            bytes[offset + 1],
            bytes[offset + 2],
            bytes[offset + 3],
        ]) as usize;
        offset += 4;

        // Bounded by what is left in the module: a count is four bytes read from input,
        // and an entry cannot be smaller than a byte, so without this a crafted count
        // asks for a 159 GB allocation ("memory allocation of 171127603240 bytes
        // failed", measured by crates/x3-integration/tests/bytecode_robustness.rs).
        let mut functions = Vec::with_capacity(count.min(bytes.len().saturating_sub(offset)));
        for _ in 0..count {
            // Name
            if offset + 2 > bytes.len() {
                return Err(BackendError::without_span(BackendErrorKind::UnexpectedEof));
            }
            let name_len = u16::from_le_bytes([bytes[offset], bytes[offset + 1]]) as usize;
            offset += 2;
            if offset + name_len > bytes.len() {
                return Err(BackendError::without_span(BackendErrorKind::UnexpectedEof));
            }
            let name = String::from_utf8_lossy(&bytes[offset..offset + name_len]).to_string();
            offset += name_len;

            // Entry point
            if offset + 4 > bytes.len() {
                return Err(BackendError::without_span(BackendErrorKind::UnexpectedEof));
            }
            let entry_point = u32::from_le_bytes([
                bytes[offset],
                bytes[offset + 1],
                bytes[offset + 2],
                bytes[offset + 3],
            ]);
            offset += 4;

            // Param count
            if offset >= bytes.len() {
                return Err(BackendError::without_span(BackendErrorKind::UnexpectedEof));
            }
            let param_count = bytes[offset];
            offset += 1;

            // Local count
            if offset + 2 > bytes.len() {
                return Err(BackendError::without_span(BackendErrorKind::UnexpectedEof));
            }
            let local_count = u16::from_le_bytes([bytes[offset], bytes[offset + 1]]);
            offset += 2;

            // Max stack
            if offset + 2 > bytes.len() {
                return Err(BackendError::without_span(BackendErrorKind::UnexpectedEof));
            }
            let max_stack = u16::from_le_bytes([bytes[offset], bytes[offset + 1]]);
            offset += 2;

            // Return type tag
            if offset >= bytes.len() {
                return Err(BackendError::without_span(BackendErrorKind::UnexpectedEof));
            }
            let return_type_tag = bytes[offset];
            offset += 1;

            functions.push(FunctionEntry {
                name,
                entry_point,
                param_count,
                local_count,
                max_stack,
                return_type_tag,
            });
        }

        Ok((functions, offset))
    }

    fn write_globals(&self, bytes: &mut Vec<u8>) {
        bytes.extend_from_slice(&(self.globals.len() as u32).to_le_bytes());
        for global in &self.globals {
            bytes.extend_from_slice(&(global.name.len() as u16).to_le_bytes());
            bytes.extend_from_slice(global.name.as_bytes());
            bytes.push(global.type_tag);
            bytes.push(if global.mutable { 1 } else { 0 });
            bytes.extend_from_slice(&global.init_const.0.to_le_bytes());
        }
    }

    fn read_globals(bytes: &[u8], mut offset: usize) -> BackendResult<(Vec<GlobalEntry>, usize)> {
        if offset + 4 > bytes.len() {
            return Err(BackendError::without_span(BackendErrorKind::UnexpectedEof));
        }
        let count = u32::from_le_bytes([
            bytes[offset],
            bytes[offset + 1],
            bytes[offset + 2],
            bytes[offset + 3],
        ]) as usize;
        offset += 4;

        // Bounded by what is left in the module: a count is four bytes read from input,
        // and an entry cannot be smaller than a byte, so without this a crafted count
        // asks for a 159 GB allocation ("memory allocation of 171127603240 bytes
        // failed", measured by crates/x3-integration/tests/bytecode_robustness.rs).
        let mut globals = Vec::with_capacity(count.min(bytes.len().saturating_sub(offset)));
        for _ in 0..count {
            // Name
            if offset + 2 > bytes.len() {
                return Err(BackendError::without_span(BackendErrorKind::UnexpectedEof));
            }
            let name_len = u16::from_le_bytes([bytes[offset], bytes[offset + 1]]) as usize;
            offset += 2;
            if offset + name_len > bytes.len() {
                return Err(BackendError::without_span(BackendErrorKind::UnexpectedEof));
            }
            let name = String::from_utf8_lossy(&bytes[offset..offset + name_len]).to_string();
            offset += name_len;

            if offset + 6 > bytes.len() {
                return Err(BackendError::without_span(BackendErrorKind::UnexpectedEof));
            }
            let type_tag = bytes[offset];
            let mutable = bytes[offset + 1] != 0;
            let init_const = ConstIdx(u32::from_le_bytes([
                bytes[offset + 2],
                bytes[offset + 3],
                bytes[offset + 4],
                bytes[offset + 5],
            ]));
            offset += 6;

            globals.push(GlobalEntry {
                name,
                type_tag,
                mutable,
                init_const,
            });
        }

        Ok((globals, offset))
    }

    fn write_debug_info(&self, bytes: &mut Vec<u8>, debug: &DebugInfo) {
        // Source map entries
        bytes.extend_from_slice(&(debug.source_map.len() as u32).to_le_bytes());
        for entry in &debug.source_map {
            bytes.extend_from_slice(&entry.code_offset.to_le_bytes());
            bytes.extend_from_slice(&entry.source_line.to_le_bytes());
            bytes.extend_from_slice(&entry.source_column.to_le_bytes());
        }

        // Symbol names
        bytes.extend_from_slice(&(debug.symbol_names.len() as u32).to_le_bytes());
        for (idx, name) in &debug.symbol_names {
            bytes.extend_from_slice(&idx.to_le_bytes());
            bytes.extend_from_slice(&(name.len() as u16).to_le_bytes());
            bytes.extend_from_slice(name.as_bytes());
        }
    }

    fn read_debug_info(bytes: &[u8], mut offset: usize) -> BackendResult<(DebugInfo, usize)> {
        // Source map
        if offset + 4 > bytes.len() {
            return Err(BackendError::without_span(BackendErrorKind::UnexpectedEof));
        }
        let map_count = u32::from_le_bytes([
            bytes[offset],
            bytes[offset + 1],
            bytes[offset + 2],
            bytes[offset + 3],
        ]) as usize;
        offset += 4;

        // Bounded by what is left in the module: a count is four bytes read from input,
        // and an entry cannot be smaller than a byte, so without this a crafted count
        // asks for a 159 GB allocation ("memory allocation of 171127603240 bytes
        // failed", measured by crates/x3-integration/tests/bytecode_robustness.rs).
        let mut source_map = Vec::with_capacity(map_count.min(bytes.len().saturating_sub(offset)));
        for _ in 0..map_count {
            if offset + 8 > bytes.len() {
                return Err(BackendError::without_span(BackendErrorKind::UnexpectedEof));
            }
            let code_offset = u32::from_le_bytes([
                bytes[offset],
                bytes[offset + 1],
                bytes[offset + 2],
                bytes[offset + 3],
            ]);
            let source_line = u16::from_le_bytes([bytes[offset + 4], bytes[offset + 5]]);
            let source_column = u16::from_le_bytes([bytes[offset + 6], bytes[offset + 7]]);
            offset += 8;
            source_map.push(SourceMapEntry {
                code_offset,
                source_line,
                source_column,
            });
        }

        // Symbol names
        if offset + 4 > bytes.len() {
            return Err(BackendError::without_span(BackendErrorKind::UnexpectedEof));
        }
        let name_count = u32::from_le_bytes([
            bytes[offset],
            bytes[offset + 1],
            bytes[offset + 2],
            bytes[offset + 3],
        ]) as usize;
        offset += 4;

        let mut symbol_names = BTreeMap::new();
        for _ in 0..name_count {
            if offset + 6 > bytes.len() {
                return Err(BackendError::without_span(BackendErrorKind::UnexpectedEof));
            }
            let idx = u32::from_le_bytes([
                bytes[offset],
                bytes[offset + 1],
                bytes[offset + 2],
                bytes[offset + 3],
            ]);
            let name_len = u16::from_le_bytes([bytes[offset + 4], bytes[offset + 5]]) as usize;
            offset += 6;

            if offset + name_len > bytes.len() {
                return Err(BackendError::without_span(BackendErrorKind::UnexpectedEof));
            }
            let name = String::from_utf8_lossy(&bytes[offset..offset + name_len]).to_string();
            offset += name_len;
            symbol_names.insert(idx, name);
        }

        Ok((
            DebugInfo {
                source_map,
                symbol_names,
            },
            offset,
        ))
    }

    fn write_metadata(&self, bytes: &mut Vec<u8>, meta: &ModuleMetadata) {
        // Compiler name
        bytes.extend_from_slice(&(meta.compiler.len() as u16).to_le_bytes());
        bytes.extend_from_slice(meta.compiler.as_bytes());

        // Compiler version
        bytes.extend_from_slice(&(meta.compiler_version.len() as u16).to_le_bytes());
        bytes.extend_from_slice(meta.compiler_version.as_bytes());

        // Timestamp
        bytes.extend_from_slice(&meta.compiled_at.to_le_bytes());

        // Source file (optional)
        if let Some(ref source) = meta.source_file {
            bytes.push(1);
            bytes.extend_from_slice(&(source.len() as u16).to_le_bytes());
            bytes.extend_from_slice(source.as_bytes());
        } else {
            bytes.push(0);
        }

        // Source hash (optional)
        if let Some(ref hash) = meta.source_hash {
            bytes.push(1);
            bytes.extend_from_slice(&(hash.len() as u16).to_le_bytes());
            bytes.extend_from_slice(hash.as_bytes());
        } else {
            bytes.push(0);
        }

        // Opt level
        bytes.push(meta.opt_level);

        // Annotations (BTreeMap for determinism)
        bytes.extend_from_slice(&(meta.annotations.len() as u32).to_le_bytes());
        for (key, value) in &meta.annotations {
            bytes.extend_from_slice(&(key.len() as u16).to_le_bytes());
            bytes.extend_from_slice(key.as_bytes());
            bytes.extend_from_slice(&(value.len() as u16).to_le_bytes());
            bytes.extend_from_slice(value.as_bytes());
        }
    }

    fn read_metadata(bytes: &[u8], mut offset: usize) -> BackendResult<(ModuleMetadata, usize)> {
        // Compiler name
        if offset + 2 > bytes.len() {
            return Err(BackendError::without_span(BackendErrorKind::UnexpectedEof));
        }
        let compiler_len = u16::from_le_bytes([bytes[offset], bytes[offset + 1]]) as usize;
        offset += 2;
        if offset + compiler_len > bytes.len() {
            return Err(BackendError::without_span(BackendErrorKind::UnexpectedEof));
        }
        let compiler = String::from_utf8_lossy(&bytes[offset..offset + compiler_len]).to_string();
        offset += compiler_len;

        // Compiler version
        if offset + 2 > bytes.len() {
            return Err(BackendError::without_span(BackendErrorKind::UnexpectedEof));
        }
        let version_len = u16::from_le_bytes([bytes[offset], bytes[offset + 1]]) as usize;
        offset += 2;
        if offset + version_len > bytes.len() {
            return Err(BackendError::without_span(BackendErrorKind::UnexpectedEof));
        }
        let compiler_version =
            String::from_utf8_lossy(&bytes[offset..offset + version_len]).to_string();
        offset += version_len;

        // Timestamp
        if offset + 8 > bytes.len() {
            return Err(BackendError::without_span(BackendErrorKind::UnexpectedEof));
        }
        let compiled_at = u64::from_le_bytes([
            bytes[offset],
            bytes[offset + 1],
            bytes[offset + 2],
            bytes[offset + 3],
            bytes[offset + 4],
            bytes[offset + 5],
            bytes[offset + 6],
            bytes[offset + 7],
        ]);
        offset += 8;

        // Source file
        if offset >= bytes.len() {
            return Err(BackendError::without_span(BackendErrorKind::UnexpectedEof));
        }
        let source_file = if bytes[offset] == 1 {
            offset += 1;
            if offset + 2 > bytes.len() {
                return Err(BackendError::without_span(BackendErrorKind::UnexpectedEof));
            }
            let len = u16::from_le_bytes([bytes[offset], bytes[offset + 1]]) as usize;
            offset += 2;
            if offset + len > bytes.len() {
                return Err(BackendError::without_span(BackendErrorKind::UnexpectedEof));
            }
            let s = String::from_utf8_lossy(&bytes[offset..offset + len]).to_string();
            offset += len;
            Some(s)
        } else {
            offset += 1;
            None
        };

        // Source hash
        if offset >= bytes.len() {
            return Err(BackendError::without_span(BackendErrorKind::UnexpectedEof));
        }
        let source_hash = if bytes[offset] == 1 {
            offset += 1;
            if offset + 2 > bytes.len() {
                return Err(BackendError::without_span(BackendErrorKind::UnexpectedEof));
            }
            let len = u16::from_le_bytes([bytes[offset], bytes[offset + 1]]) as usize;
            offset += 2;
            if offset + len > bytes.len() {
                return Err(BackendError::without_span(BackendErrorKind::UnexpectedEof));
            }
            let s = String::from_utf8_lossy(&bytes[offset..offset + len]).to_string();
            offset += len;
            Some(s)
        } else {
            offset += 1;
            None
        };

        // Opt level
        if offset >= bytes.len() {
            return Err(BackendError::without_span(BackendErrorKind::UnexpectedEof));
        }
        let opt_level = bytes[offset];
        offset += 1;

        // Annotations
        if offset + 4 > bytes.len() {
            return Err(BackendError::without_span(BackendErrorKind::UnexpectedEof));
        }
        let anno_count = u32::from_le_bytes([
            bytes[offset],
            bytes[offset + 1],
            bytes[offset + 2],
            bytes[offset + 3],
        ]) as usize;
        offset += 4;

        let mut annotations = BTreeMap::new();
        for _ in 0..anno_count {
            if offset + 2 > bytes.len() {
                return Err(BackendError::without_span(BackendErrorKind::UnexpectedEof));
            }
            let key_len = u16::from_le_bytes([bytes[offset], bytes[offset + 1]]) as usize;
            offset += 2;
            if offset + key_len > bytes.len() {
                return Err(BackendError::without_span(BackendErrorKind::UnexpectedEof));
            }
            let key = String::from_utf8_lossy(&bytes[offset..offset + key_len]).to_string();
            offset += key_len;

            if offset + 2 > bytes.len() {
                return Err(BackendError::without_span(BackendErrorKind::UnexpectedEof));
            }
            let val_len = u16::from_le_bytes([bytes[offset], bytes[offset + 1]]) as usize;
            offset += 2;
            if offset + val_len > bytes.len() {
                return Err(BackendError::without_span(BackendErrorKind::UnexpectedEof));
            }
            let value = String::from_utf8_lossy(&bytes[offset..offset + val_len]).to_string();
            offset += val_len;

            annotations.insert(key, value);
        }

        Ok((
            ModuleMetadata {
                compiler,
                compiler_version,
                compiled_at,
                source_file,
                source_hash,
                opt_level,
                annotations,
            },
            offset,
        ))
    }

    fn compute_checksum(&self, data: &[u8]) -> u32 {
        // One implementation, shared with the no-std reader through `x3-common`.
        crate::bytecode::checksum(data)
    }

    /// Get function by index.
    pub fn get_function(&self, idx: FuncIdx) -> Option<&FunctionEntry> {
        self.functions.get(idx.0 as usize)
    }

    /// Get function by name.
    pub fn find_function(&self, name: &str) -> Option<(FuncIdx, &FunctionEntry)> {
        self.functions
            .iter()
            .enumerate()
            .find(|(_, f)| f.name == name)
            .map(|(i, f)| (FuncIdx(i as u32), f))
    }

    /// Get constant from pool.
    pub fn get_const(&self, idx: ConstIdx) -> Option<&ConstValue> {
        self.const_pool.get(idx)
    }
}

#[cfg(test)]
mod version_rule_parity {
    //! The shared rule and this crate's rule have to agree version by version.
    //!
    //! They did not for a patch bump: `crate::bytecode::version_is_readable` compared
    //! `version <= VERSION` (refusing `1.0.1`) while `VersionInfo::can_read` ignores the patch field
    //! (accepting it) — and because the no-std reader that executes on chain uses the shared one, a
    //! module the format calls compatible could be compiled and then refused where it matters
    //! (TICKET-137). The list is written out rather than computed so no case is skipped.

    use super::*;

    #[test]
    fn the_shared_version_rule_agrees_with_can_read() {
        let current = VersionInfo::reader();
        let reader = crate::bytecode::READER_VERSION;
        for (label, packed) in [
            ("the base version", VERSION),
            ("the reader's version", reader),
            ("a patch bump", reader + 1),
            ("a minor bump", reader + 0x100),
            ("a major bump", reader + 0x1_0000),
            ("an older patch", VERSION.saturating_sub(1)),
            ("an older minor", reader.saturating_sub(0x100)),
        ] {
            assert_eq!(
                crate::bytecode::version_is_readable(packed),
                current.can_read(VersionInfo::from_packed(packed)),
                "the shared rule and `can_read` must agree about {label} (0x{packed:08x})"
            );
        }
    }
}

impl Default for BytecodeModule {
    fn default() -> Self {
        Self::new()
    }
}

/// Constant pool for immediate values.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ConstPool {
    pub entries: Vec<ConstValue>,
    /// Deduplication map for integers.
    #[serde(skip)]
    int_map: BTreeMap<i64, ConstIdx>,
    /// Deduplication map for floats (using bits).
    #[serde(skip)]
    float_map: BTreeMap<u64, ConstIdx>,
    /// Deduplication map for strings.
    #[serde(skip)]
    string_map: BTreeMap<String, ConstIdx>,
}

impl ConstPool {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            int_map: BTreeMap::new(),
            float_map: BTreeMap::new(),
            string_map: BTreeMap::new(),
        }
    }

    /// Add an integer constant, deduplicating.
    pub fn add_integer(&mut self, value: i64) -> BackendResult<ConstIdx> {
        if let Some(&idx) = self.int_map.get(&value) {
            return Ok(idx);
        }
        let idx = self.add_entry(ConstValue::Integer(value))?;
        self.int_map.insert(value, idx);
        Ok(idx)
    }

    /// Add a float constant, deduplicating.
    pub fn add_float(&mut self, value: f64) -> BackendResult<ConstIdx> {
        let bits = value.to_bits();
        if let Some(&idx) = self.float_map.get(&bits) {
            return Ok(idx);
        }
        let idx = self.add_entry(ConstValue::Float(value))?;
        self.float_map.insert(bits, idx);
        Ok(idx)
    }

    /// Add a string constant, deduplicating.
    pub fn add_string(&mut self, value: String) -> BackendResult<ConstIdx> {
        if value.len() > MAX_STRING_LEN {
            return Err(BackendError::without_span(
                BackendErrorKind::StringTooLong {
                    len: value.len(),
                    max: MAX_STRING_LEN,
                },
            ));
        }
        if let Some(&idx) = self.string_map.get(&value) {
            return Ok(idx);
        }
        let idx = self.add_entry(ConstValue::String(value.clone()))?;
        self.string_map.insert(value, idx);
        Ok(idx)
    }

    /// Add a bool constant.
    pub fn add_bool(&mut self, value: bool) -> BackendResult<ConstIdx> {
        // Bools are not deduplicated (they're tiny)
        self.add_entry(ConstValue::Bool(value))
    }

    /// Add raw bytes.
    pub fn add_bytes(&mut self, value: Vec<u8>) -> BackendResult<ConstIdx> {
        self.add_entry(ConstValue::Bytes(value))
    }

    fn add_entry(&mut self, value: ConstValue) -> BackendResult<ConstIdx> {
        if self.entries.len() >= MAX_CONST_POOL as usize {
            return Err(BackendError::without_span(
                BackendErrorKind::ConstPoolOverflow {
                    max: MAX_CONST_POOL,
                },
            ));
        }
        let idx = ConstIdx(self.entries.len() as u32);
        self.entries.push(value);
        Ok(idx)
    }

    /// Get constant by index.
    pub fn get(&self, idx: ConstIdx) -> Option<&ConstValue> {
        self.entries.get(idx.0 as usize)
    }

    /// Number of entries in pool.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Values stored in the constant pool.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ConstValue {
    Integer(i64),
    Float(f64),
    String(String),
    Bool(bool),
    Bytes(Vec<u8>),
}

impl ConstValue {
    pub fn as_integer(&self) -> Option<i64> {
        match self {
            ConstValue::Integer(v) => Some(*v),
            _ => None,
        }
    }

    pub fn as_float(&self) -> Option<f64> {
        match self {
            ConstValue::Float(v) => Some(*v),
            _ => None,
        }
    }

    pub fn as_string(&self) -> Option<&str> {
        match self {
            ConstValue::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            ConstValue::Bool(b) => Some(*b),
            _ => None,
        }
    }
}

/// Function table entry.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FunctionEntry {
    /// Function name.
    pub name: String,
    /// Byte offset in instruction stream.
    pub entry_point: u32,
    /// Number of parameters.
    pub param_count: u8,
    /// Number of local variables (excluding params).
    pub local_count: u16,
    /// Maximum stack depth needed.
    pub max_stack: u16,
    /// Return type tag (0=void, 1=int, 2=float, 3=bool, 4=other).
    pub return_type_tag: u8,
}

/// Global variable entry.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GlobalEntry {
    /// Global name.
    pub name: String,
    /// Type tag.
    pub type_tag: u8,
    /// Whether this global is mutable.
    pub mutable: bool,
    /// Initial value constant index.
    pub init_const: ConstIdx,
}

/// Debug information for source mapping.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DebugInfo {
    /// Maps code offsets to source locations.
    pub source_map: Vec<SourceMapEntry>,
    /// Maps symbol indices to names (BTreeMap for determinism).
    pub symbol_names: BTreeMap<u32, String>,
}

/// Single entry in the source map.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SourceMapEntry {
    /// Byte offset in instruction stream.
    pub code_offset: u32,
    /// Source line number (1-based).
    pub source_line: u16,
    /// Source column number (1-based).
    pub source_column: u16,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn const_pool_dedup() {
        let mut pool = ConstPool::new();

        let idx1 = pool.add_integer(42).unwrap();
        let idx2 = pool.add_integer(42).unwrap();
        assert_eq!(idx1, idx2);

        let idx3 = pool.add_integer(100).unwrap();
        assert_ne!(idx1, idx3);

        assert_eq!(pool.len(), 2);
    }

    #[test]
    fn module_roundtrip() {
        let mut module = BytecodeModule::new();
        module.const_pool.add_integer(123).unwrap();
        module.const_pool.add_string("hello".to_string()).unwrap();
        module.functions.push(FunctionEntry {
            name: "main".to_string(),
            entry_point: 0,
            param_count: 0,
            local_count: 2,
            max_stack: 4,
            return_type_tag: 1,
        });
        module.code = vec![0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x05]; // LoadConst r0, c0; Ret r0

        let bytes = module.to_bytes();
        let decoded = BytecodeModule::from_bytes(&bytes).unwrap();

        assert_eq!(decoded.const_pool.len(), 2);
        assert_eq!(decoded.functions.len(), 1);
        assert_eq!(decoded.functions[0].name, "main");
        assert_eq!(decoded.code.len(), module.code.len());
    }

    #[test]
    fn module_flags() {
        let mut flags = ModuleFlags::new();
        assert!(!flags.has(ModuleFlags::DEBUG_INFO));

        flags.set(ModuleFlags::DEBUG_INFO);
        flags.set(ModuleFlags::USES_EVM);

        assert!(flags.has(ModuleFlags::DEBUG_INFO));
        assert!(flags.has(ModuleFlags::USES_EVM));
        assert!(!flags.has(ModuleFlags::USES_SVM));
    }

    #[test]
    fn version_info_packing() {
        let v = VersionInfo::new(1, 2, 3);
        let packed = v.to_packed();
        let unpacked = VersionInfo::from_packed(packed);
        assert_eq!(v, unpacked);
        assert_eq!(v.major, 1);
        assert_eq!(v.minor, 2);
        assert_eq!(v.patch, 3);
    }

    #[test]
    fn version_compatibility() {
        let v1_0_0 = VersionInfo::new(1, 0, 0);
        let v1_1_0 = VersionInfo::new(1, 1, 0);
        let v1_2_0 = VersionInfo::new(1, 2, 0);
        let v2_0_0 = VersionInfo::new(2, 0, 0);

        // Same major version - can read older minor
        assert!(v1_1_0.can_read(v1_0_0));
        assert!(v1_2_0.can_read(v1_1_0));

        // Cannot read newer minor
        assert!(!v1_0_0.can_read(v1_1_0));

        // Cannot read different major
        assert!(!v2_0_0.can_read(v1_0_0));
        assert!(!v1_0_0.can_read(v2_0_0));
    }

    #[test]
    fn version_satisfies() {
        let v1_0_0 = VersionInfo::new(1, 0, 0);
        let v1_1_0 = VersionInfo::new(1, 1, 0);
        let v1_1_5 = VersionInfo::new(1, 1, 5);

        assert!(v1_1_0.satisfies(v1_0_0));
        assert!(v1_1_5.satisfies(v1_1_0));
        assert!(!v1_0_0.satisfies(v1_1_0));
    }

    #[test]
    fn feature_flags_min_version() {
        let mut f = FeatureFlags::new();
        assert_eq!(f.min_version_required(), VersionInfo::new(1, 0, 0));

        f.set(FeatureFlags::EXTENDED_OPCODES);
        assert_eq!(f.min_version_required(), VersionInfo::new(1, 1, 0));

        f.set(FeatureFlags::CROSS_VM_CALLS);
        assert_eq!(f.min_version_required(), VersionInfo::new(1, 2, 0));

        f.set(FeatureFlags::SIMD_OPS);
        assert_eq!(f.min_version_required(), VersionInfo::new(1, 3, 0));
    }

    #[test]
    fn the_private_submission_bit_is_the_shared_one_and_is_not_a_version_change() {
        // One bit, two crates: `x3-common::bytecode` is where the no-std reader gets it, and this
        // alias is what the writer sets. If they drift, a module the compiler marks as demanding
        // privacy is read by the chain as an ordinary one — the demand would be carried and
        // ignored, which is the failure this flag exists to prevent.
        assert_eq!(
            FeatureFlags::PRIVATE_SUBMISSION_REQUIRED,
            crate::bytecode::FEATURE_PRIVATE_SUBMISSION_REQUIRED
        );

        let mut f = FeatureFlags::new();
        f.set(FeatureFlags::PRIVATE_SUBMISSION_REQUIRED);
        // Deliberately 1.0.0: the demand is interpreted by the loader, not a format change, so a
        // reader that ignores the bit is not saved by a version bound it cannot meet.
        assert_eq!(f.min_version_required(), VersionInfo::new(1, 0, 0));
        assert!(f.has(FeatureFlags::PRIVATE_SUBMISSION_REQUIRED));
        // The bit sits in the same word as the version-significant flags without colliding with
        // any of them.
        for other in [
            FeatureFlags::EXTENDED_OPCODES,
            FeatureFlags::TYPED_CONSTANTS,
            FeatureFlags::INLINE_CACHE,
            FeatureFlags::CROSS_VM_CALLS,
            FeatureFlags::GAS_METERING,
            FeatureFlags::CUSTOM_SECTIONS,
            FeatureFlags::COMPRESSED_CONSTS,
            FeatureFlags::SIMD_OPS,
        ] {
            assert_eq!(
                other & FeatureFlags::PRIVATE_SUBMISSION_REQUIRED,
                0,
                "the private-submission bit must not overlap {other:#x}"
            );
        }
    }

    #[test]
    fn module_with_metadata() {
        let mut module = BytecodeModule::new();
        module.const_pool.add_integer(42).unwrap();
        module.functions.push(FunctionEntry {
            name: "test".to_string(),
            entry_point: 0,
            param_count: 0,
            local_count: 1,
            max_stack: 2,
            return_type_tag: 1,
        });
        module.code = vec![0x00, 0x01];
        module.metadata = Some(ModuleMetadata {
            compiler: "x3c".to_string(),
            compiler_version: "1.0.0".to_string(),
            compiled_at: 1700000000,
            source_file: Some("test.x3".to_string()),
            source_hash: Some("abc123".to_string()),
            opt_level: 2,
            annotations: {
                let mut m = BTreeMap::new();
                m.insert("target".to_string(), "evm".to_string());
                m
            },
        });

        let bytes = module.to_bytes();
        let decoded = BytecodeModule::from_bytes(&bytes).unwrap();

        assert!(decoded.metadata.is_some());
        let meta = decoded.metadata.unwrap();
        assert_eq!(meta.compiler, "x3c");
        assert_eq!(meta.compiler_version, "1.0.0");
        assert_eq!(meta.compiled_at, 1700000000);
        assert_eq!(meta.source_file.as_deref(), Some("test.x3"));
        assert_eq!(meta.source_hash.as_deref(), Some("abc123"));
        assert_eq!(meta.opt_level, 2);
        assert_eq!(meta.annotations.get("target"), Some(&"evm".to_string()));
    }

    #[test]
    fn module_version_roundtrip() {
        let mut module = BytecodeModule::with_version(VersionInfo::new(1, 0, 0));
        module.set_features({
            let mut f = FeatureFlags::new();
            f.set(FeatureFlags::GAS_METERING);
            f
        });
        module.const_pool.add_integer(1).unwrap();
        module.functions.push(FunctionEntry {
            name: "main".to_string(),
            entry_point: 0,
            param_count: 0,
            local_count: 0,
            max_stack: 1,
            return_type_tag: 0,
        });
        module.code = vec![0x00];

        let bytes = module.to_bytes();
        let decoded = BytecodeModule::from_bytes(&bytes).unwrap();

        assert_eq!(decoded.version, VersionInfo::new(1, 0, 0));
        assert_eq!(decoded.min_version, VersionInfo::new(1, 0, 0));
        assert!(decoded.features.has(FeatureFlags::GAS_METERING));
        assert!(decoded.is_compatible());
    }

    #[test]
    fn the_shared_version_predicates_agree_with_version_info() {
        // `x3-common::bytecode` expresses the same rules on the packed integer, because the
        // no-std reader has no access to `VersionInfo`. If the two ever disagree, one of the two
        // decoders is lying about what it accepts.
        use crate::bytecode as shared;

        let current = VersionInfo::reader();
        for packed in [
            0u32,
            1,
            shared::MIN_SUPPORTED_VERSION,
            shared::VERSION,
            shared::TRADING_VERSION,
            (1 << 16) | (2 << 8),
            shared::MAX_SUPPORTED_VERSION,
            u32::MAX,
        ] {
            let version = VersionInfo::from_packed(packed);
            assert_eq!(
                current.can_read(version),
                shared::version_is_readable(packed),
                "can_read disagrees at {packed:#010x}"
            );
            assert_eq!(
                current.satisfies(version),
                shared::loader_satisfies(packed),
                "satisfies disagrees at {packed:#010x}"
            );
        }
    }

    #[test]
    fn a_corrupted_body_fails_the_checksum() {
        let mut module = BytecodeModule::new();
        module.const_pool.add_integer(123).unwrap();
        let bytes = module.to_bytes();
        assert!(
            BytecodeModule::from_bytes(&bytes).is_ok(),
            "the module this test corrupts must load first"
        );

        // Change one byte after the header. The structure still parses; the checksum does not.
        let mut corrupted = bytes.clone();
        let last = corrupted.len() - 1;
        corrupted[last] ^= 0xFF;
        match BytecodeModule::from_bytes(&corrupted) {
            Err(error) => assert!(
                matches!(error.kind, BackendErrorKind::ChecksumMismatch { .. }),
                "expected a checksum mismatch, got {:?}",
                error.kind
            ),
            Ok(_) => panic!("a body corrupted after compilation must not load"),
        }
    }
}
