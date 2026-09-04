// Auto-generated opcode constants derived from `opcodes.yaml`.
// This file provides a single source of truth for opcode values used
// throughout the compiler, verifier, and executor.

pub const LOCK: u8 = 0x20;
pub const MINT: u8 = 0x21;
pub const BURN: u8 = 0x22;
pub const RELEASE: u8 = 0x23;
pub const SWAP: u8 = 0x24;
pub const BRIDGE: u8 = 0x25;

pub const IF: u8 = 0x30;
pub const LOOP: u8 = 0x31;
pub const CALL: u8 = 0x32;
pub const RET: u8 = 0x33;

pub const REQUIRE: u8 = 0x40;
pub const ON_FAIL: u8 = 0x41;
pub const ON_TIMEOUT: u8 = 0x42;

pub const ATOMIC_BEGIN: u8 = 0x50;
pub const ATOMIC_END: u8 = 0x51;
pub const ATOMIC_ROLLBACK: u8 = 0x52;

pub const EMIT: u8 = 0x60;
pub const CALL_HOST: u8 = 0x61;

pub const GPU_DISPATCH: u8 = 0x80;
pub const SIMULATE: u8 = 0x81;
pub const SCHEDULED_DISPATCH: u8 = 0x82;
pub const INTENT_RESOLVE: u8 = 0x83;
pub const CRDT_OP: u8 = 0x84;
pub const PROOF_VERIFY: u8 = 0x85;
pub const STORAGE_OP: u8 = 0x86;
pub const PATHFIND: u8 = 0x87;
pub const MEMPOOL_SCAN: u8 = 0x88;
pub const ORACLE_REQUEST: u8 = 0x89;
pub const EMERGENCY_CONTROL: u8 = 0x8A;
pub const LIFECYCLE: u8 = 0x8B;
pub const SERIALIZE: u8 = 0x8C;
pub const DESERIALIZE: u8 = 0x8D;
pub const GAS_ESTIMATE: u8 = 0x8E;
pub const CHAIN_METRIC: u8 = 0x8F;
pub const EVENT_PROVENANCE: u8 = 0x90;
pub const MULTI_HOP_SWAP: u8 = 0x91;
pub const VECTOR_MATH: u8 = 0x92;
pub const ROLE_CHECK: u8 = 0x93;
pub const MULTISIG_CHECK: u8 = 0x94;
pub const VERSION_META: u8 = 0x95;
pub const STORAGE_NAMESPACE: u8 = 0x96;
pub const ABI_EXPORT: u8 = 0x97;
pub const DOC_EMBED: u8 = 0x98;
pub const GAS_ADAPTIVE: u8 = 0x99;
pub const BOUNTY: u8 = 0x9A;
pub const SUB_EXEC: u8 = 0x9B;

pub const NOP: u8 = 0x00;
pub const BYTECODE_VERSION_1: u8 = 0x01;
pub const META_NONCE: u8 = 0x10;
pub const META_CHAIN_ID: u8 = 0x11;
pub const HALT: u8 = 0xFF;
