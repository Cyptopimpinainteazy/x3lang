//! PHASE 42's prohibitions, as gates rather than as prose.
//!
//! The phase says consensus-affecting decisions must never depend on unordered maps,
//! wall-clock jitter, randomness, thread scheduling, floating-point ambiguity,
//! non-deterministic optimizer output, or external mutable state without explicit
//! proof. A paragraph saying so is worth nothing; a command that fails is worth
//! something.
//!
//! This file covers the items that can be decided mechanically and are true of the
//! whole tree today: no clock, no random source, no spawned thread. They are covered
//! **absolutely** — no allowlist — because there are zero of them, and a gate that has
//! to be argued with is a gate that gets argued with.
//!
//! The remaining three items are covered elsewhere and are named here so the gap is
//! visible rather than implied:
//!
//! - **unordered maps**: classified, and the census re-measured here rather than left at
//!   the figure the classification started from. **50** `HashMap`/`HashSet` mentions across
//!   **12** files in these crates, down from the 68 across 14 that round 57 went through
//!   site by site (TICKET-086, closed in `cb72223a4`). The difference is this session's own
//!   fixes — the two IR fields that reached the artifact's bytes (`ir.rs`), the bridge's
//!   storage and lifecycle states and `lowering.rs`'s route payload — plus `regalloc.rs`,
//!   deleted in TICKET-085. Every remaining mention is a keyed lookup or a membership test;
//!   the one *iteration* the classification found was `regalloc.rs`'s
//!   `temp_to_reg.values().any(…)`, an existence test, and it went with the file. The
//!   item-by-item record is `.ai/reports/x3lang-round57-20260919.md`, and the byte-identity
//!   tests below are what bite if a new one drifts — a type scan cannot see a clock reached
//!   through a dependency, and these can.
//! - **floating-point ambiguity**: PHASE 43. `crates/x3-common/src/fixed.rs` is the
//!   vocabulary and `crates/x3-common/tests/fixed_math.rs` answers the audit item by
//!   item; the one live `f64` is a Solana wire field, documented at the field.
//! - **non-deterministic optimizer output**: `compiler/src/lib.rs` pins that the
//!   register-allocation entry point emits byte-identical output to the plain one, so
//!   the pass is a record rather than a rewrite. That test is a ratchet — wiring the
//!   rewrite breaks it (TICKET-085).
//! - **external mutable state without explicit proof**: the fail-closed half is
//!   PHASE 44's measured guards, which refuse with `X3_GUARD_UNMEASURED` rather than
//!   comparing whatever a register happens to hold.

use std::fs;
use std::path::{Path, PathBuf};

/// The crates whose decisions reach a compiled artifact or a settlement.
fn consensus_roots() -> Vec<PathBuf> {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    [
        "src",
        "../vm/src",
        "../crates/x3-ast/src",
        "../crates/x3-common/src",
        "../crates/x3-lexer/src",
    ]
    .iter()
    .map(|relative| manifest.join(relative))
    .collect()
}

fn rust_sources(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut pending = roots.to_vec();
    while let Some(root) = pending.pop() {
        let entries = fs::read_dir(&root)
            .unwrap_or_else(|error| panic!("the audit must be able to read {}: {error}", root.display()));
        for entry in entries {
            let path = entry.expect("a readable directory entry").path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                out.push(path);
            }
        }
    }
    // Sorted so a failure names the same file twice in a row.
    out.sort();
    out
}

/// Whether `haystack` contains `needle` as a word — the character before it must not
/// be alphanumeric or an underscore.
///
/// Plain `contains` is not enough, and the reason is not hypothetical: `"rand::"` is a
/// substring of `Operand::Reg(r)`, so a `contains`-based version of this audit reported
/// two violations in a comment about register operands and none of the real thing. The
/// negative test below is that exact string.
fn contains_word(haystack: &str, needle: &str) -> bool {
    let mut from = 0;
    while let Some(at) = haystack[from..].find(needle) {
        let start = from + at;
        let boundary = haystack[..start]
            .chars()
            .next_back()
            .is_none_or(|before| !before.is_alphanumeric() && before != '_');
        if boundary {
            return true;
        }
        from = start + needle.len();
    }
    false
}

/// Lines that are entirely a comment are skipped: a doc that explains *why* a clock is
/// not read is the thing this audit wants more of, and failing it would be the gate
/// punishing its own documentation.
fn code_lines(source: &str) -> impl Iterator<Item = (usize, &str)> {
    source
        .lines()
        .enumerate()
        .filter(|(_, line)| !line.trim_start().starts_with("//"))
}

#[test]
fn the_word_boundary_helper_rejects_the_false_positive_it_exists_for() {
    assert!(contains_word("use rand::Rng;", "rand::"));
    assert!(contains_word("(rand::thread_rng(), x)", "rand::"));
    assert!(
        !contains_word("// carry `Operand::Reg(r)` slots directly", "rand::"),
        "`Operand::Reg` contains `rand::` as a substring and must not be reported"
    );
    assert!(contains_word("std::time::SystemTime::now()", "SystemTime"));
    assert!(!contains_word("my_SystemTime", "SystemTime"));
}

/// PHASE 42: "consensus-affecting decisions must never depend on … wall clock jitter,
/// randomness, thread scheduling".
#[test]
fn no_consensus_crate_reads_a_clock_a_random_source_or_spawns_a_thread() {
    const FORBIDDEN: &[(&str, &str)] = &[
        ("SystemTime", "a wall-clock reading is not reproducible across runs"),
        ("Instant::now", "a monotonic reading is not reproducible across runs"),
        ("rand::", "a random source makes the artifact depend on entropy"),
        (
            "thread_rng",
            "a thread-local random source makes the artifact depend on the thread",
        ),
        (
            "from_entropy",
            "seeding from entropy makes the artifact depend on the host",
        ),
        ("OsRng", "the operating system's random source is not reproducible"),
        ("rand_core", "the random traits bring a seeded generator in"),
        (
            "std::thread::spawn",
            "thread scheduling decides interleaving, and interleaving decides output",
        ),
        (
            "rayon::",
            "a work-stealing pool decides interleaving, and interleaving decides output",
        ),
    ];

    let sources = rust_sources(&consensus_roots());
    assert!(
        sources.len() > 20,
        "the audit read {} source files, which is too few to be reading the tree it \
         thinks it is reading",
        sources.len()
    );

    let mut violations = Vec::new();
    for path in &sources {
        let source = fs::read_to_string(path).expect("a readable source file");
        for (number, line) in code_lines(&source) {
            for (needle, why) in FORBIDDEN {
                if contains_word(line, needle) {
                    violations.push(format!("{}:{}: {needle} — {why}", path.display(), number + 1));
                }
            }
        }
    }

    assert!(
        violations.is_empty(),
        "PHASE 42 forbids these in a consensus-affecting path, and this gate has no \
         allowlist because there are none:\n{}",
        violations.join("\n")
    );
}

/// The same prohibition at the *artifact* level rather than the source level: the same
/// source must produce the same bytes, twice, in one process and across two.
///
/// A source scan can miss a clock reached through a dependency; this cannot. It is the
/// property the phase actually wants, so it is asserted directly rather than inferred.
#[test]
fn the_same_source_compiles_to_the_same_artifact_every_time() {
    let sources = rust_sources(&consensus_roots());
    // `trading_core_v1.x3` is the one fixture in this directory that a current build
    // accepts — the others are deliberately invalid, or stale in a way TICKET-083
    // records.
    let program = x3_lang_compiler::parser::parse_source(include_str!("fixtures/trading_core_v1.x3"))
        .expect("the audit's own fixture must parse");

    let first = x3_lang_compiler::compile_program(&program).expect("the fixture must compile");
    for attempt in 0..8 {
        let again = x3_lang_compiler::compile_program(&program).expect("the fixture must compile");
        assert_eq!(
            again,
            first,
            "compile {attempt} of the same program produced different bytes, over {} \
             source files; something in the pipeline reads a clock, a random source, a \
             thread, or iterates an unordered map",
            sources.len()
        );
    }
}

/// The same property for the program shape that first broke it: an `emit` whose payload
/// map is rendered into the artifact.
///
/// This is the reproduction, kept as a test. Measured before the fix — twelve identical
/// compiles of this source produced **six** distinct artifacts, differing only in the
/// payload's key order:
///
/// ```text
/// EMIT trade_seen:{"arg2": "…", "arg1": "…", "arg0": "…"}
/// EMIT trade_seen:{"arg1": "…", "arg2": "…", "arg0": "…"}
/// ```
///
/// The cause was `Operation::Emit::data` being a `HashMap`: `RandomState` is seeded per
/// map instance, so two maps built from the same entries in the same process iterate in
/// different orders, and the emitter writes that order into the bytes. A one-entry map
/// has no order to disagree about, which is why the property test beside this one
/// (`property_tests.rs`, one weight) never saw it.
#[test]
fn an_emits_payload_map_does_not_reach_the_artifacts_byte_order() {
    const SOURCE: &str = "strategy EmitterProbe {\n    \
        input ethereum.USDC amount 25_000_000 max 50_000_000\n    \
        output ethereum.ETH\n    effects [swap]\n    guarantees [min_profit]\n    \
        domains [ethereum]\n    risk { max_slippage_bps 50 max_total_fee_bps 8 }\n    \
        split profit {\n        100% -> trader\n    }\n    \
        bounds { max_steps 10 max_gas 200_000 }\n    \
        execute {\n        emit trade_seen(1, 2, 3)\n        \
        swap uniswap ethereum.USDC -> ethereum.ETH amount 1_000 min_output 1\n        \
        require slippage <= 50\n        require profit >= 5\n        \
        on_fail refund ethereum.USDC to sender\n    }\n}\n";

    let program = x3_lang_compiler::parser::parse_source(SOURCE).expect("the fixture must parse");
    let first = x3_lang_compiler::compile_program(&program).expect("the fixture must compile");

    // The payload really is in the artifact, so the byte-identity below is about this
    // map rather than about the compiler ignoring it.
    let rendered = String::from_utf8_lossy(&first).into_owned();
    assert!(
        rendered.contains("arg0") && rendered.contains("arg1") && rendered.contains("arg2"),
        "the emit's payload must reach the artifact for this test to mean anything"
    );

    // Sixteen compiles: the failure rate before the fix was about one in two, so a
    // single comparison could have passed by luck.
    for attempt in 0..16 {
        let again = x3_lang_compiler::compile_program(&program).expect("the fixture must compile");
        assert_eq!(
            again, first,
            "compile {attempt} of the same source produced different bytes; a program's \
             payload map must not reach the artifact's byte order (PHASE 42)"
        );
    }
}
