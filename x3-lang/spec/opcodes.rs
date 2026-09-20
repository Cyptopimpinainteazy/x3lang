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
pub const ROUTE_FALLBACK: u8 = 0x54;
pub const PARALLEL_PLAN: u8 = 0x55;
pub const FEATURE_ALLOW: u8 = 0x56;
pub const STRATEGY_LICENSE: u8 = 0x57;
/// How a declared venue's leg settles (`[VENUE_SETTLEMENT][u16 len][venue:shape]`, the
/// whole record padded to four bytes). The payload is the venue's declared name, a
/// colon, and the shape word from PHASE 39's closed set — or nothing after the colon
/// for a venue that states none.
///
/// The colon cannot appear in a venue name (a name is an identifier), so the two
/// fields cannot be confused with one; and `guarantee: None` is spelled as an empty
/// second field rather than by omitting the separator, so a record that is merely
/// truncated is refused instead of read as "no guarantee stated".
pub const VENUE_SETTLEMENT: u8 = 0x58;

/// Feature codes for `FEATURE_ALLOW`.
///
/// The artifact records which execution modes the program consented to. Consent
/// that only exists in the source cannot be shown to a runtime that is deciding
/// whether it may net this intent against another.
///
/// A byte, not a `u16`, and that is a constraint of the frame rather than a
/// preference: a fixed instruction here is three bytes, the reader takes the
/// second as the flags byte and the *third* as the low half of the operand, so
/// the third byte is where a small code can live. Writing the code as a `u16`
/// puts it at bytes two and three, and the reader then sees flags = the code's
/// low byte and operand = zero — which is how this instruction first failed to
/// verify.
pub const FEATURE_INTENT_FUSION: u8 = 1;

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
/// An order to a venue: open a spot or perp leg, liquidate a position, receive its
/// collateral.
///
/// The instruction a hedge and a liquidation need to execute at all. `Operation::Call`
/// would have carried it as an untyped host call, and the VM routes `CALL_HOST` to
/// `BridgeAdapter::svm_call` — so a perp short on Ethereum would arrive at a host as an
/// SVM call and be refused for a reason that has nothing to do with it. A venue order
/// says what it is, with a quantity the artifact can check.
pub const VENUE_ORDER: u8 = 0x9D;
/// A target portfolio: move this account to these weights, ranked by this criterion.
///
/// What a `rebalance` can honestly be. The declaration decides the target and the criterion
/// — a portfolio whose weights sum to exactly 100%, and a target the optimizer can rank —
/// and neither the language nor the compiler holds the *current* portfolio, which every
/// trade that reaches the target depends on. So the artifact carries the target as an
/// instruction a host acts on, with its own current holdings, and the compiler does not
/// pretend to have generated the trades it cannot compute (TICKET-070).
pub const REBALANCE_TARGET: u8 = 0x9E;

/// Whether a nonce has been used, and the recording of it.
///
/// The one instruction that leaves a guard's quantity in `r0` from a *run-time*
/// fact rather than a declaration: `require nonce unused <id>` is a claim about
/// the chain's history, so the compiler cannot decide it and the VM has to. Its
/// payload carries the identifier; the executor sets `r0` to 1 when the nonce is
/// new and records it, and the guard that follows compares `r0` against 1.
pub const NONCE_UNUSED: u8 = 0x9C;

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

/// The two arithmetic opcodes have no named constant of their own because nothing
/// in `opcodes.yaml` declares them; they are here so the cost table below can name
/// every code it prices (the table used to be a list of bare hex).
pub const ADD: u8 = 0x01;
pub const SUB: u8 = 0x02;
pub const BYTECODE_VERSION_1: u8 = 0x01;
pub const META_NONCE: u8 = 0x10;
pub const META_CHAIN_ID: u8 = 0x11;
/// The record that binds an artifact to the versions that produced and run it — spec
/// PHASE 45.
///
/// Five `u16`s after the tag: language, compiler, IR, VM, economic policy. The first,
/// third and fourth decide **compatibility**, and `verify` rejects an artifact whose
/// versions are not the ones this runtime supports; the compiler and policy numbers are
/// carried so a reader can say which build produced the artifact without guessing from
/// its shape.
///
/// A missing record is a rejection too, and that is the fail-closed direction: an
/// artifact that binds to nothing cannot be shown to be one this runtime may execute.
pub const META_VERSIONS: u8 = 0x12;
/// The language version this compiler emits and this VM accepts.
pub const LANGUAGE_VERSION: u16 = 1;
/// The compiler's own format version. Carried, not compared: it does not decide whether
/// an artifact may run.
pub const COMPILER_FORMAT_VERSION: u16 = 1;
/// The X3IR version.
pub const IR_VERSION: u16 = 1;
/// The VM version an artifact requires.
pub const VM_VERSION: u16 = 1;
/// The economic policy schema version.
pub const POLICY_VERSION: u16 = 1;
/// The bytes a versions record occupies: the tag and five `u16`s.
pub const VERSIONS_RECORD_LEN: usize = 11;
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
/// `r0 >= operand`, where `r0` is the **profit a host measured**, in basis points.
///
/// A measured mode rather than a flag beside `GE`, because the two facts belong
/// together: what to compare, and that comparing is only meaningful against a
/// measurement. The executor refuses the instruction outright when no host reported
/// one, so a measured guard can never pass on whatever `r0` happened to hold.
pub const REQUIRE_COMPARE_MEASURED_PROFIT: u8 = 2;
/// `r0 <= operand`, where `r0` is the **slippage a host measured**, in basis points.
pub const REQUIRE_COMPARE_MEASURED_SLIPPAGE: u8 = 3;
/// Mask for the comparison-mode bits of a `REQUIRE` flags byte.
pub const REQUIRE_COMPARE_MASK: u8 = 0x03;

/// A capability reply's first byte, marking "a measured quantity follows".
///
/// After it come a unit byte and a 16-byte little-endian value, **in the unit of the
/// guard it answers**: a profit in basis points of the capital committed, or a
/// slippage in basis points of the price. The unit is the guard's own because the
/// `REQUIRE` operand is two bytes — an absolute floor would not fit, and comparing a
/// basis-point floor against an absolute amount would be a units mismatch dressed as
/// enforcement.
///
/// Any other reply is opaque: the VM still puts it in `r0` and records that no
/// measurement arrived, which is what lets a measured guard refuse rather than
/// compare bytes that mean something else.
pub const CAPABILITY_REPLY_MEASURED_TAG: u8 = 0x01;
/// The unit byte a reply carries when it reports a **profit** in basis points.
pub const MEASURED_UNIT_PROFIT_BPS: u8 = 1;
/// The unit byte a reply carries when it reports a **slippage** in basis points.
pub const MEASURED_UNIT_SLIPPAGE_BPS: u8 = 2;
/// The unit byte a reply carries when it reports a hedge's residual **delta** in basis
/// points of the notional being hedged.
///
/// A hedge asks a venue for each leg, so the delta is the quantity the venue's answer is
/// about: the compiler computes the delta from the legs the program *declared*, and this is
/// how the venue says what it actually filled. Without it the bound is a constraint on the
/// declaration rather than a post-condition on the trade, and a venue that filled something
/// else is not caught (TICKET-068).
pub const MEASURED_UNIT_DELTA_BPS: u8 = 3;

/// Pack a measurement reply: the tag, the unit, and the value.
pub fn measured_reply(unit: u8, value: u128) -> Vec<u8> {
    let mut reply = Vec::with_capacity(18);
    reply.push(CAPABILITY_REPLY_MEASURED_TAG);
    reply.push(unit);
    reply.extend_from_slice(&value.to_le_bytes());
    reply
}

/// Read every measurement a reply carries, as `(unit, value)` pairs.
///
/// A reply is a **sequence** of 18-byte records, because one trade answers two
/// questions: a plan's profit floor and its slippage ceiling are both about the same
/// call, and a host that had to answer twice would have to say which reply went with
/// which guard. Anything that is not a record is not a measurement, and a caller cannot
/// mistake an opaque reply for one — the list is simply empty.
pub fn read_measured_replies(reply: &[u8]) -> Vec<(u8, u128)> {
    let mut found = Vec::new();
    let mut rest = reply;
    while rest.first() == Some(&CAPABILITY_REPLY_MEASURED_TAG) {
        if rest.len() < 18 {
            // A truncated record is not a measurement. Dropping it is the fail-closed
            // direction: a measured guard refuses when nothing reported one.
            break;
        }
        let mut bytes = [0u8; 16];
        bytes.copy_from_slice(&rest[2..18]);
        found.push((rest[1], u128::from_le_bytes(bytes)));
        rest = &rest[18..];
    }
    found
}

/// Read a single measurement, for a reply that carries exactly one.
pub fn read_measured_reply(reply: &[u8]) -> Option<(u8, u128)> {
    let found = read_measured_replies(reply);
    if found.len() == 1 {
        found.into_iter().next()
    } else {
        None
    }
}

/// The comparison the *guard* makes, in bits 2-4 of the same flags byte.
///
/// The two are different questions and used to share one byte's worth of
/// meaning: bits 0-1 say whether the VM should test a run-time quantity, bits
/// 2-4 record what the program wrote (`slippage <= 7`). Recording the guard's
/// own operator is what stops `<=` and `>=` from being the same program.
pub const GUARD_OP_NONE: u8 = 0;
pub const GUARD_OP_LT: u8 = 1;
pub const GUARD_OP_LE: u8 = 2;
pub const GUARD_OP_GT: u8 = 3;
pub const GUARD_OP_GE: u8 = 4;
pub const GUARD_OP_EQ: u8 = 5;
pub const GUARD_OP_NE: u8 = 6;

/// Pack a `REQUIRE` flags byte: comparison mode in bits 0-1, guard operator in
/// bits 2-4.
pub const fn require_flags(comparison_mode: u8, guard_operator: u8) -> u8 {
    (comparison_mode & REQUIRE_COMPARE_MASK) | ((guard_operator & 0x07) << 2)
}

/// Where a measured `REQUIRE` keeps **which** quantity it compares, in the flags byte.
///
/// The comparison-mode field is two bits and its four values were spent before a third
/// measured quantity existed: `STATIC`, `GE`, and the two measured modes. So the mode says
/// *that* a measurement is compared, and the unit code in bits 5-7 says which one.
///
/// The profit's code is **zero**, which is what this field held before it meant anything —
/// an artifact emitted earlier carries zeroes there and still reads as the guard it was, so
/// the encoding grew without a format version.
pub const MEASURED_UNIT_CODE_SHIFT: u8 = 5;
/// Mask for the unit-code bits of a `REQUIRE` flags byte.
pub const MEASURED_UNIT_CODE_MASK: u8 = 0xE0;
/// `r0 >= operand`, where `r0` is the profit a host measured.
pub const MEASURED_UNIT_CODE_PROFIT_BPS: u8 = 0;
/// `r0 <= operand`, where `r0` is the hedge delta a host measured.
pub const MEASURED_UNIT_CODE_DELTA_BPS: u8 = 1;

/// One of the unit codes a measured `REQUIRE` may carry.
pub const fn is_known_measured_unit_code(code: u8) -> bool {
    matches!(code, MEASURED_UNIT_CODE_PROFIT_BPS | MEASURED_UNIT_CODE_DELTA_BPS)
}

/// Pack a `REQUIRE` flags byte whose measured comparison names its unit.
pub const fn require_flags_measured(comparison_mode: u8, guard_operator: u8, unit_code: u8) -> u8 {
    require_flags(comparison_mode, guard_operator)
        | ((unit_code << MEASURED_UNIT_CODE_SHIFT) & MEASURED_UNIT_CODE_MASK)
}

/// The measured quantity a `REQUIRE` flags byte names.
pub const fn require_measured_unit_code(flags: u8) -> u8 {
    (flags & MEASURED_UNIT_CODE_MASK) >> MEASURED_UNIT_CODE_SHIFT
}

/// The guard operator recorded in a `REQUIRE` flags byte.
pub const fn require_guard_operator(flags: u8) -> u8 {
    (flags >> 2) & 0x07
}

/// The comparison mode recorded in a `REQUIRE` flags byte.
///
/// The reader that belongs with [`require_flags`]. The mode and the operator share the
/// byte, so reading the mode is a mask, and a reader that masks it by hand is a reader
/// that can forget to: the VM, the disassembler and the simulation's floor reader each
/// did, and the simulation's compared the raw byte, so it found no measured guard in an
/// artifact that had two. One mask, in one place.
pub const fn require_comparison(flags: u8) -> u8 {
    flags & REQUIRE_COMPARE_MASK
}

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
/// The path whose venues' declared fees sum lowest. Named for what it computes:
/// declared fees from venue attributes, never profit — a profit ranking would need
/// prices the opportunity graph does not hold (`compiler/src/arb.rs`).
pub const CHOICE_CRITERION_LOWEST_DECLARED_FEE: u8 = 2;

/// Maximum approved substitutions a route `fallback` may declare.
///
/// "Every fallback must be statically bounded" is the constraint, so the bound
/// is shared by the compiler that enforces it and the VM that refuses a record
/// outside it — one definition, so an artifact cannot be emitted against a
/// bound the runtime does not apply.
pub const MAX_ROUTE_FALLBACKS: usize = 8;

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
            EMIT | CALL_HOST | ATOMIC_CHOICE | ROUTE_FALLBACK | PARALLEL_PLAN | STRATEGY_LICENSE
                | VENUE_SETTLEMENT
                | NONCE_UNUSED
                | GPU_DISPATCH..=REBALANCE_TARGET
                | ROUTE_SCORE..=REFUND_POLICY
                | TRADING_BEGIN..=TRADING_BRIDGE
        )
}

/// How many bytes a fixed-frame instruction's own record occupies in a compiler
/// stream: three, or four for `REQUIRE`.
///
/// `REQUIRE` writes `[opcode][flags][threshold u16]`, all four bytes
/// meaningful — the flags byte carries the comparison mode and the guard's
/// operator, and the operand is a real `u16` threshold. Every other fixed frame
/// writes `[opcode][flags][operand]` and lets the padding byte the emitter adds
/// complete the operand.
///
/// The distinction only shows up when a frame starts at an offset congruent to
/// one mod four, which is where the first instruction of a compiler stream
/// without metadata sits (byte zero is the version byte). There the two widths
/// round to different boundaries: `align4(1 + 3) == 4`, but `align4(1 + 4) == 8`.
/// A reader that assumed three bytes advanced one byte into the guard's own
/// operand and then onto its padding, recorded a boundary that is not an
/// instruction, and read the padding — and the payload after it — as opcodes:
/// `x3c explain` printed eighteen lines of `UNKNOWN` for a six-instruction
/// program whose first instruction is a `risk_policy` guard.
///
/// Not meaningful for a payload opcode: those are `3 + payload_len` bytes and
/// the caller has the length. `is_payload_opcode` answers that question.
pub const fn fixed_frame_content_len(opcode: u8) -> usize {
    if opcode == REQUIRE {
        4
    } else {
        3
    }
}

/// The `u16` operand of a fixed-frame instruction, as the frame carries it.
///
/// In a compiler stream a three-byte frame stores only the operand's low byte:
/// the high byte is the padding the emitter writes to reach the next four-byte
/// boundary, so reading four bytes takes it from *outside* the frame. That
/// byte is zero for every frame that got its padding, which is why the bug
/// stayed invisible until a reader met a three-byte frame at an offset
/// congruent to one mod four — the first instruction of a stream without
/// metadata. There the high half came from the next instruction's opcode: a
/// `feature_allow` guard at offset 1 presented itself as "feature code
/// 0x5683" and the VM refused its own compiler's artifact.
///
/// Do not call this for a payload opcode: its second and third bytes are the
/// length prefix, and callers that read a payload use the payload reader.
pub const fn fixed_frame_operand(opcode: u8, compiler_stream: bool, bytes_lo: u8, bytes_hi: u8) -> u16 {
    if !compiler_stream || fixed_frame_content_len(opcode) >= 4 {
        (bytes_lo as u16) | ((bytes_hi as u16) << 8)
    } else {
        bytes_lo as u16
    }
}

/// The name of an instruction, as the disassembler, the executor's diagnostics and
/// any tooling should print it.
///
/// One table, in the file both crates include, because it used to be two: the
/// compiler's disassembler had one and the executor's diagnostics had another, and
/// an instruction added to one was missing from the other. The nonce instruction
/// was, and only a test noticed — which is what a table written twice buys.
///
/// `UNKNOWN` is the only value that means "no instruction here"; a caller that
/// needs its own spelling of that maps it.
pub const fn opcode_name(opcode: u8) -> &'static str {
    match opcode {
        // The two arithmetic opcodes have no named constant of their own; every
        // other arm is written with the constant so the value and the name cannot
        // drift apart.
        0x01 => "ADD",
        0x02 => "SUB",
        META_NONCE => "META_NONCE",
        META_CHAIN_ID => "META_CHAIN_ID",
        META_VERSIONS => "META_VERSIONS",
        LOCK => "LOCK",
        MINT => "MINT",
        BURN => "BURN",
        RELEASE => "RELEASE",
        SWAP => "SWAP",
        BRIDGE => "BRIDGE",
        IF => "IF",
        LOOP => "LOOP",
        CALL => "CALL",
        RET => "RET",
        REQUIRE => "REQUIRE",
        ON_FAIL => "ON_FAIL",
        ON_TIMEOUT => "ON_TIMEOUT",
        ATOMIC_BEGIN => "ATOMIC_BEGIN",
        ATOMIC_END => "ATOMIC_END",
        ATOMIC_ROLLBACK => "ATOMIC_ROLLBACK",
        ATOMIC_CHOICE => "ATOMIC_CHOICE",
        ROUTE_FALLBACK => "ROUTE_FALLBACK",
        VENUE_SETTLEMENT => "VENUE_SETTLEMENT",
        PARALLEL_PLAN => "PARALLEL_PLAN",
        STRATEGY_LICENSE => "STRATEGY_LICENSE",
        EMIT => "EMIT",
        CALL_HOST => "CALL_HOST",
        GPU_DISPATCH => "GPU_DISPATCH",
        SIMULATE => "SIMULATE",
        SCHEDULED_DISPATCH => "SCHEDULED_DISPATCH",
        INTENT_RESOLVE => "INTENT_RESOLVE",
        CRDT_OP => "CRDT_OP",
        PROOF_VERIFY => "PROOF_VERIFY",
        STORAGE_OP => "STORAGE_OP",
        PATHFIND => "PATHFIND",
        MEMPOOL_SCAN => "MEMPOOL_SCAN",
        ORACLE_REQUEST => "ORACLE_REQUEST",
        EMERGENCY_CONTROL => "EMERGENCY_CONTROL",
        LIFECYCLE => "LIFECYCLE",
        SERIALIZE => "SERIALIZE",
        DESERIALIZE => "DESERIALIZE",
        GAS_ESTIMATE => "GAS_ESTIMATE",
        CHAIN_METRIC => "CHAIN_METRIC",
        EVENT_PROVENANCE => "EVENT_PROVENANCE",
        MULTI_HOP_SWAP => "MULTI_HOP_SWAP",
        VENUE_ORDER => "VENUE_ORDER",
        REBALANCE_TARGET => "REBALANCE_TARGET",
        VECTOR_MATH => "VECTOR_MATH",
        ROLE_CHECK => "ROLE_CHECK",
        MULTISIG_CHECK => "MULTISIG_CHECK",
        VERSION_META => "VERSION_META",
        STORAGE_NAMESPACE => "STORAGE_NAMESPACE",
        ABI_EXPORT => "ABI_EXPORT",
        DOC_EMBED => "DOC_EMBED",
        GAS_ADAPTIVE => "GAS_ADAPTIVE",
        BOUNTY => "BOUNTY",
        SUB_EXEC => "SUB_EXEC",
        NONCE_UNUSED => "NONCE_UNUSED",
        ROUTE_SCORE => "ROUTE_SCORE",
        SOLVER_BID => "SOLVER_BID",
        RELAYER_ATTEST => "RELAYER_ATTEST",
        RPC_CONSENSUS => "RPC_CONSENSUS",
        RISK_SCORE => "RISK_SCORE",
        INVARIANT_CHECK => "INVARIANT_CHECK",
        PRIVACY_COMMIT => "PRIVACY_COMMIT",
        PROOF_REQUIRED => "PROOF_REQUIRED",
        VM_ADAPTER_CALL => "VM_ADAPTER_CALL",
        MODE_CHECK => "MODE_CHECK",
        PACKAGE_IMPORT => "PACKAGE_IMPORT",
        REFUND_POLICY => "REFUND_POLICY",
        TRADING_BEGIN => "TRADING_BEGIN",
        TRADING_OPEN_DEBT => "TRADING_OPEN_DEBT",
        TRADING_EXECUTE_SWAP => "TRADING_EXECUTE_SWAP",
        TRADING_CLOSE_DEBT => "TRADING_CLOSE_DEBT",
        TRADING_ASSERT_MIN_PROFIT => "TRADING_ASSERT_MIN_PROFIT",
        TRADING_ASSERT_ALL_DEBTS => "TRADING_ASSERT_ALL_DEBTS",
        TRADING_EMIT_RECEIPT => "TRADING_EMIT_RECEIPT",
        TRADING_COMMIT => "TRADING_COMMIT",
        TRADING_ABORT => "TRADING_ABORT",
        TRADING_ASSERT_INVARIANT => "TRADING_ASSERT_INVARIANT",
        TRADING_BRIDGE => "TRADING_BRIDGE",
        HALT => "HALT",
        _ => "UNKNOWN",
    }
}

/// The weight this VM charges to fetch and execute one instruction, before any
/// operand-dependent surcharge.
///
/// This table *is* the charge: `vm/src/executor.rs` reads it rather than keeping a
/// copy, so a compile-time estimate (PHASE 35) can never disagree with what a run
/// costs. It used to live in the executor as a list of bare hex codes, which is a
/// second statement of the opcode catalogue and drifts from it silently — the
/// comments there had to explain what `0x0A` and `0x70` were, and no opcode in this
/// file has ever had those values, so both are kept only because this is a move of
/// the table rather than a redesign of what the VM charges.
///
/// The numbers are consensus-relevant: changing one changes what a program costs.
/// What the estimate report prints as the basis of its weight figure: the charge
/// and the estimate are the same table, so the two cannot disagree.
pub const BASE_WEIGHT_TABLE_IS_THE_CHARGE: &str = "the table vm/src/executor.rs charges from";

pub const fn base_gas_cost(opcode: u8) -> u128 {
    match opcode {
        // 0x0A: no instruction in this catalogue. Kept so the table is the VM's
        // own, unchanged.
        0x0A => 50,
        ADD | SUB => 1,
        META_NONCE | META_CHAIN_ID => 5,
        META_VERSIONS => 11,
        LOCK | MINT => 1,
        BRIDGE => 100,
        IF | LOOP => 2,
        CALL | RET => 5,
        REQUIRE => 10,
        ATOMIC_BEGIN | ATOMIC_END => 250,
        EMIT | CALL_HOST => 100,
        // 0x70: no instruction in this catalogue, for the same reason as 0x0A.
        0x70 => 2,
        GPU_DISPATCH => 500,
        SIMULATE => 200,
        SCHEDULED_DISPATCH => 100,
        INTENT_RESOLVE => 150,
        CRDT_OP => 10,
        PROOF_VERIFY => 500,
        STORAGE_OP => 50,
        PATHFIND => 100,
        MEMPOOL_SCAN | ORACLE_REQUEST => 50,
        EMERGENCY_CONTROL => 10,
        LIFECYCLE => 500,
        SERIALIZE | DESERIALIZE => 20,
        GAS_ESTIMATE => 30,
        CHAIN_METRIC => 10,
        EVENT_PROVENANCE => 20,
        MULTI_HOP_SWAP => 200,
        VENUE_ORDER => 200,
        REBALANCE_TARGET => 200,
        VECTOR_MATH => 5,
        ROLE_CHECK | MULTISIG_CHECK | VERSION_META | STORAGE_NAMESPACE | ABI_EXPORT | DOC_EMBED => 10,
        GAS_ADAPTIVE => 50,
        BOUNTY => 50,
        SUB_EXEC => 50,
        // A nonce test is a membership check and a recording: a scan over the run's
        // nonces, priced like the other cheap capabilities.
        NONCE_UNUSED => 50,
        ROUTE_SCORE..=REFUND_POLICY => 50,
        TRADING_BEGIN..=TRADING_BRIDGE => 50,
        HALT => 0,
        _ => 1,
    }
}

/// The metadata record at `pc`, as `(bytes it occupies, label, rendered value)`.
///
/// **One walker, because five places used to parse this set and all five drifted.** The
/// verifier's metadata skip, the executor's, the disassembler's and the trading decoder's
/// each had their own `match` on the tags, and adding one meant finding all of them:
/// `META_VERSIONS` was read as instructions by three of the four on the day it was added,
/// which the compiler's own tests caught only because they assert agreement between the
/// writer's boundaries and the readers'.
///
/// `None` means the byte is not a metadata tag, which is where the header ends and the
/// instructions begin.
pub fn metadata_record(bytes: &[u8], pc: usize) -> Option<(usize, &'static str, String)> {
    let tag = *bytes.get(pc)?;
    match tag {
        META_NONCE => {
            let len = u16::from_le_bytes([*bytes.get(pc + 1)?, *bytes.get(pc + 2)?]) as usize;
            let end = pc.checked_add(3 + len)?;
            if end > bytes.len() {
                return None;
            }
            let value = String::from_utf8_lossy(&bytes[pc + 3..end]).to_string();
            Some((3 + len, "meta.nonce", format!("{value:?}")))
        }
        META_CHAIN_ID => {
            let end = pc.checked_add(9)?;
            if end > bytes.len() {
                return None;
            }
            let mut eight = [0u8; 8];
            eight.copy_from_slice(&bytes[pc + 1..pc + 9]);
            Some((9, "meta.chain_id", u64::from_le_bytes(eight).to_string()))
        }
        META_VERSIONS => {
            let end = pc.checked_add(VERSIONS_RECORD_LEN)?;
            if end > bytes.len() {
                return None;
            }
            let read = |offset: usize| -> u16 {
                let at = pc + 1 + offset * 2;
                u16::from_le_bytes([bytes[at], bytes[at + 1]])
            };
            Some((
                VERSIONS_RECORD_LEN,
                "meta.versions",
                format!(
                    "language {} compiler {} IR {} VM {} policy {}",
                    read(0),
                    read(1),
                    read(2),
                    read(3),
                    read(4)
                ),
            ))
        }
        _ => None,
    }
}

/// The versions an artifact binds, as `(language, compiler, IR, VM, policy)`.
///
/// `None` when the stream is not a compiler stream or carries no binding — which
/// `vm/src/verifier.rs` refuses, because an artifact that binds to nothing cannot be shown
/// to be one this runtime may execute.
pub fn version_binding(bytes: &[u8]) -> Option<(u16, u16, u16, u16, u16)> {
    if bytes.first() != Some(&BYTECODE_VERSION_1) {
        return None;
    }
    let mut pc = 1usize;
    while let Some((len, label, _)) = metadata_record(bytes, pc) {
        if label == "meta.versions" {
            let read = |offset: usize| -> u16 {
                let at = pc + 1 + offset * 2;
                u16::from_le_bytes([bytes[at], bytes[at + 1]])
            };
            return Some((read(0), read(1), read(2), read(3), read(4)));
        }
        pc += len;
    }
    None
}
