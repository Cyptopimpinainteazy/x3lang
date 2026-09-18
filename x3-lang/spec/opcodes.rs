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
pub const ATOMIC_CHOICE: u8 = 0x53;

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

pub const ROUTE_SCORE: u8 = 0xA0;
pub const SOLVER_BID: u8 = 0xA1;
pub const RELAYER_ATTEST: u8 = 0xA2;
pub const RPC_CONSENSUS: u8 = 0xA3;
pub const RISK_SCORE: u8 = 0xA4;
pub const INVARIANT_CHECK: u8 = 0xA5;
pub const PRIVACY_COMMIT: u8 = 0xA6;
pub const PROOF_REQUIRED: u8 = 0xA7;
pub const VM_ADAPTER_CALL: u8 = 0xA8;
pub const MODE_CHECK: u8 = 0xA9;
pub const PACKAGE_IMPORT: u8 = 0xAA;
pub const REFUND_POLICY: u8 = 0xAB;

pub const TRADING_BEGIN: u8 = 0xB0;
pub const TRADING_OPEN_DEBT: u8 = 0xB1;
pub const TRADING_EXECUTE_SWAP: u8 = 0xB2;
pub const TRADING_CLOSE_DEBT: u8 = 0xB3;
pub const TRADING_ASSERT_MIN_PROFIT: u8 = 0xB4;
pub const TRADING_ASSERT_ALL_DEBTS: u8 = 0xB5;
pub const TRADING_EMIT_RECEIPT: u8 = 0xB6;
pub const TRADING_COMMIT: u8 = 0xB7;
pub const TRADING_ABORT: u8 = 0xB8;
pub const TRADING_ASSERT_INVARIANT: u8 = 0xB9;
pub const TRADING_BRIDGE: u8 = 0xBA;

pub const NOP: u8 = 0x00;
pub const BYTECODE_VERSION_1: u8 = 0x01;
pub const META_NONCE: u8 = 0x10;
pub const META_CHAIN_ID: u8 = 0x11;
pub const HALT: u8 = 0xFF;

/// Comparison codes for `REQUIRE`, carried in the instruction's flags byte.
///
/// `REQUIRE`'s operand field holds a threshold rather than the packed
/// register/register/immediate triples other instructions use, because a guard
/// always tests register `r0` — the one a declaration instruction leaves the
/// guarded quantity in.
///
/// `STATIC` means the guard asserts something about the artifact's
/// configuration rather than about run-time state. The compiler has already
/// checked it, so the instruction records the guard in the artifact and has
/// nothing to test. The executor must not invent a test for it: that is how
/// every guard in the language came to depend on whatever an unrelated previous
/// instruction happened to leave in `r0`.
pub const REQUIRE_COMPARE_STATIC: u8 = 0;
/// `r0 >= operand`.
pub const REQUIRE_COMPARE_GE: u8 = 1;

/// Criterion codes for `ATOMIC_CHOICE`, carried in the instruction's flags
/// byte; the operand packs `paths << 8 | selected`.
///
/// The artifact records which criterion ranked the branches and which index
/// won, so a reader can tell that the emitted body is one of a verified set
/// rather than the only branch the program had. Any code outside this set is a
/// verifier failure — the criterion set is closed precisely so that an
/// unrecognised one cannot be interpreted as some default.
pub const CHOICE_CRITERION_HIGHEST_NET_OUTPUT: u8 = 0;
pub const CHOICE_CRITERION_FEWEST_HOPS: u8 = 1;

/// Whether an instruction carries a length-prefixed payload —
/// `[opcode][u16 len][payload]`, the whole thing padded to four bytes.
///
/// This is the single definition. It previously existed twice, once in the
/// compiler's disassembler and once in the VM's verifier, and the two had
/// drifted apart: the disassembler listed `0x66`, which no arm of the emitter
/// produces, while omitting `CALL_HOST` (`0x61`), which does carry a payload; and
/// the verifier listed neither `EMIT` nor `CALL_HOST`. A reader that classifies a
/// payload-carrying instruction as fixed-width advances four bytes and then reads
/// bytes that are not instructions, which is how the same defect has surfaced
/// four times in this format.
///
/// `compiler_stream` distinguishes the two encodings the VM accepts: emitter
/// output frames the asset and bridge operations with a payload, while raw
/// bytecode (hand-assembled, or emitted before this framing existed) does not.
/// The compiler's disassembler only ever reads emitter output, so it passes
/// `true`.
pub const fn is_payload_opcode(opcode: u8, compiler_stream: bool) -> bool {
    (compiler_stream && matches!(opcode, LOCK | MINT | BURN | RELEASE | SWAP | BRIDGE))
        || matches!(
            opcode,
            EMIT | CALL_HOST
                | GPU_DISPATCH..=SUB_EXEC
                | ROUTE_SCORE..=REFUND_POLICY
                | TRADING_BEGIN..=TRADING_BRIDGE
        )
}
