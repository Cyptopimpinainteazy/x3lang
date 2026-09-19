//! X3 compiler library.
//!
//! The reconcilIED pipeline layers three complementary verifiers so no
//! single check is duplicated:
//!
//! 1. [`numeric::verify_numeric_policy`] — X3Lang 1.0 integer literal and
//!    direct-call coercion policy, checked over the parsed AST.
//! 2. [`verify::verify_ir`] — structural IR invariants (atomic scoping,
//!    non-zero amounts/iterations, empty-field safety) that must hold
//!    before bytecode emission.
//! 3. [`semantic`] — whole-program semantic safety (chain/adapter/asset
//!    allow-lists, refund paths, explicit finality/proofs, route scores,
//!    invariant rules, compile-mode gating, and risk scoring).

pub mod dag;
pub mod diagnostic;
pub mod emitter;
pub mod formatter;
pub mod fusion;
pub mod intent_emit;
pub mod ir;
pub mod linter;
pub mod lowering;
pub mod metadata;
pub mod numeric;
pub mod objective;
pub mod opportunity;
pub mod optimizer;
pub mod parser;
pub mod regalloc;
pub mod risk;
pub mod semantic;
pub mod strategy;
pub mod trading_lowering;
pub mod trading_semantic;
pub mod trading_verify;
pub mod verify;
pub mod spec {
    pub mod opcodes {
        include!("../../spec/opcodes.rs");
    }
}

use diagnostic::{CompilerDiagnostic, DiagnosticSeverity};
use emitter::emit_x3ir;
use lowering::{lower_program, lower_program_with_mode, LowerCtx};
use parser::parse_source;
use regalloc::{allocate, AllocationResult};
use semantic::verify_atomic_swap_decls;
use semantic::verify_with_config as verify_semantics;
use semantic::{
    verify_atomic_choice_decls, verify_parallel_decls, verify_privacy_decls, verify_relayer_quorum_declared,
    verify_route_fallbacks, verify_solver_bond_declared, verify_venue_decls,
};
use x3_lang_ast::ast::Program;
use x3_lang_common::{ErrorAccumulator, Span, X3Error};

// Re-export IR types
pub use ir::{Condition, FailureAction, Operation, ProgramMetadata, RequireKind, X3IR};

// Re-export semantic types
pub use semantic::{CompilationMode, InvariantRule, RiskScore, VerifyOutcome};

pub use ir::{AssetKey, TradingOperation, ValueRef};
pub use trading_lowering::{lower_atomic_trade, LowerError};
pub use trading_semantic::{amount_base_units, analyze_trading, decimal_to_base_units, TradingSymbols, TypedAmount};
pub use trading_verify::{verify_atomic_trade, verify_trading_program, DebtFlowState};

// Re-export register-allocation entry points so callers (and tests) can run
// allocation as a standalone pass without going through the full pipeline.
pub use regalloc::{allocate as allocate_registers, AllocationResult as RegisterAllocationResult};

/// Everything that has to be checked before bytecode may be emitted, and the IR
/// those checks ran against.
pub(crate) struct PreEmission {
    pub ir: crate::ir::X3IR,
    pub errors: Vec<X3Error>,
    pub warnings: Vec<X3Error>,
}

/// Run every verification layer that precedes bytecode emission.
///
/// This exists because the layers used to be wired per entry point, and they
/// were not wired the same way in each: two of the three layers documented at
/// the top of this file were reachable from none of them, the AST-level pass was
/// reached by one `check` variant and not by its sibling, and two entry points
/// ran no AST or IR checks at all. A single funnel makes "every entry point runs
/// the same checks" true by construction, instead of something to re-audit after
/// every change.
///
/// The order is deliberate: a program the AST-level or trading checks already
/// refused is not lowered, so the caller gets the real reasons instead of a
/// cascade of consequences.
pub(crate) fn run_pre_emission_layers(program: &Program, mode: CompilationMode) -> Result<PreEmission, X3Error> {
    run_pre_emission_layers_with_context(program, LowerCtx::new(), mode)
}

/// The same funnel, for callers that lower with an explicit context (replay
/// protection, chain id).
pub(crate) fn run_pre_emission_layers_with_context(
    program: &Program,
    ctx: LowerCtx,
    mode: CompilationMode,
) -> Result<PreEmission, X3Error> {
    let mut errors = ast_level_errors(program);

    let trading_symbols = match analyze_trading(program, mode) {
        Ok(symbols) => symbols,
        Err(trading_errors) => {
            return Ok(PreEmission {
                ir: crate::ir::X3IR::new(),
                errors: trading_errors,
                warnings: Vec::new(),
            })
        }
    };
    errors.extend(verify_trading_program(program, &trading_symbols, mode));

    if !errors.is_empty() {
        return Ok(PreEmission {
            ir: crate::ir::X3IR::new(),
            errors,
            warnings: Vec::new(),
        });
    }

    let ir = lower_program_with_mode(program, ctx, mode)?;
    errors.extend(ir_level_errors(&ir));

    // A guard whose claim is about the program's own operations is decided here,
    // where both the guards (from the AST) and the operations (from the flat IR)
    // are in hand. See `semantic::verify_canonical_supply`.
    errors.extend(semantic::verify_canonical_supply(program, &ir));

    let outcome = semantic::verify_collect(
        &ir,
        semantic::DEFAULT_MAX_ATOMIC_OPS,
        semantic::DEFAULT_MAX_ROUTE_HOPS,
        Some(mode),
    );
    errors.extend(outcome.errors);

    Ok(PreEmission {
        ir,
        errors,
        warnings: outcome.warnings,
    })
}

/// Report a batch of pipeline errors as one error carrying every message.
///
/// Every violation is listed, not just a count: a bare "3 error(s)" gives a
/// caller nothing to act on and hides which layer refused the program.
fn format_pipeline_errors(errors: &[X3Error]) -> X3Error {
    X3Error::SemanticError {
        message: format!(
            "compilation failed with {} error(s): {}",
            errors.len(),
            errors
                .iter()
                .map(|error| error.to_string())
                .collect::<Vec<_>>()
                .join("; ")
        ),
        span: Span::DUMMY,
    }
}

/// Diagnostics that can only be seen on the AST, before lowering.
///
/// Both entry points run this. It used to be reachable only from the
/// `check`-style entry points, so `x3c build` skipped AST-level validation
/// entirely — the same "a check that is not on the path that matters" shape that
/// keeps turning up in this crate.
fn ast_level_errors(program: &Program) -> Vec<X3Error> {
    let mut errors = Vec::new();
    let mut acc = ErrorAccumulator::new();
    verify_atomic_swap_decls(program, &mut acc);
    verify_atomic_choice_decls(program, &mut acc);
    verify_route_fallbacks(program, &mut acc);
    verify_privacy_decls(program, &mut acc);
    verify_venue_decls(program, &mut acc);
    strategy::verify_strategy_modules(program, &mut acc);
    objective::verify_objective_decls(program, &mut acc);
    verify_parallel_decls(program, &mut acc);
    verify_solver_bond_declared(program, &mut acc);
    verify_relayer_quorum_declared(program, &mut acc);
    errors.extend(acc.errors().iter().cloned());

    // Layer 1 of the pipeline described at the top of this file. It was
    // documented there and called from nowhere but its own tests, so the
    // language's integer-literal and coercion policy was never applied to a
    // real program.
    errors.extend(
        numeric::verify_numeric_policy(program)
            .iter()
            .filter(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error)
            .map(diagnostic_to_error),
    );

    errors
}

/// Layer 2 of the pipeline: structural IR invariants that must hold before
/// bytecode emission — atomic scoping balance, non-zero amounts and iterations,
/// empty-field safety. Documented at the top of this file and, like layer 1,
/// called from nowhere but its own tests.
pub(crate) fn ir_level_errors(ir: &crate::ir::X3IR) -> Vec<X3Error> {
    match verify::verify_ir(ir) {
        Ok(()) => Vec::new(),
        Err(diagnostics) => diagnostics.iter().map(diagnostic_to_error).collect(),
    }
}

/// A pipeline diagnostic as an error the rest of the compiler can carry,
/// keeping its stable code visible so tooling can still key on it.
fn diagnostic_to_error(diagnostic: &CompilerDiagnostic) -> X3Error {
    X3Error::SemanticError {
        message: format!("{}: {}", diagnostic.code.as_str(), diagnostic.message),
        span: diagnostic.primary_span,
    }
}

/// Compile an X3 AST program to bytecode
///
/// Pipeline: AST → X3IR → Bytecode
pub fn compile_program(program: &Program) -> Result<Vec<u8>, X3Error> {
    compile_program_with_context(program, LowerCtx::new())
}

/// Compile an X3 AST program with the register-allocation pass wired in.
///
/// Pipeline: AST → X3IR → register-allocate (assigns physical regs/spill
/// slots to logical temporaries) → Bytecode. The allocation result is
/// returned alongside the bytecode so callers can inspect the register
/// pressure (`registers_used`, `spills_used`) for diagnostics.
///
/// This is the entry point that promotes `regalloc::allocate` from a
/// library-only function to a real pass in the production compilation
/// pipeline. Without it, the linear-scan allocator at
/// `x3-lang/compiler/src/regalloc.rs` is dead code as far as the compiled
/// binary is concerned.
pub fn compile_program_with_regalloc(program: &Program) -> Result<(Vec<u8>, AllocationResult), X3Error> {
    let pre = run_pre_emission_layers(program, CompilationMode::Dev)?;
    if !pre.errors.is_empty() {
        return Err(format_pipeline_errors(&pre.errors));
    }
    let ir = pre.ir;
    let _alloc = allocate(&ir.operations);
    // The v0.1 pipeline records allocation metadata without rewriting
    // the IR (see `patch_operation` in `regalloc.rs`). Future versions
    // will mutate `ir.operations` to carry `Operand::Reg(r)` /
    // `Operand::Spill(s)` slots directly. Until then the allocation
    // result is the canonical record of physical-register decisions.
    let bytecode = emit_x3ir(&ir)?;
    verify_bytecode(&bytecode)?;
    Ok((bytecode, _alloc))
}

/// Parse, verify, lower, and compile X3 source for the currently supported
/// capability subset.
///
/// The semantic verifier runs on this path, not only on the `check`-style
/// entry points. This used to lower and emit directly, which made the
/// *default* build path the unverified one: `cmd_build` in `x3c` selects this
/// function whenever the mode is `dev`, so the compiler emitted bytecode for
/// programs the verifier rejects. Measured before the fix: 280 bytes / 70 ops
/// of bytecode for `tests/conformance/invalid/routes/same_chain_bridge.x3`,
/// which `x3c check` refuses with four safety errors including "bridge
/// from_chain == to_chain; cross-VM bridge must target a different chain".
///
/// `lower_program(program, ctx)` is exactly `lower_program_with_mode(program,
/// ctx, CompilationMode::Dev)`, so routing through [`compile_with_mode`] does
/// not change how anything lowers — it only adds the verification that was
/// missing.
pub fn compile_source(source: &str) -> Result<Vec<u8>, X3Error> {
    compile_with_mode(source, CompilationMode::Dev)
}

/// Parse, lower, and run the semantic verifier, keeping the warnings.
///
/// Use this when the warnings matter — tooling should. `check_source` and
/// `check_source_with_mode` remain as pass/fail wrappers for callers that only
/// need the errors.
///
/// The verifier stops at the semantic pass without emitting bytecode, exposing
/// every production-safety problem in a single shot.
pub fn check_source_diagnostics(source: &str) -> Result<(Program, crate::ir::X3IR, semantic::VerifyOutcome), X3Error> {
    check_source_diagnostics_with_mode(source, CompilationMode::Dev)
}

/// `check_source_diagnostics` with an explicit compilation mode, so
/// mode-gated safety checks run too.
pub fn check_source_diagnostics_with_mode(
    source: &str,
    mode: CompilationMode,
) -> Result<(Program, crate::ir::X3IR, semantic::VerifyOutcome), X3Error> {
    let program = parse_source(source)?;
    let pre = run_pre_emission_layers(&program, mode)?;
    Ok((
        program,
        pre.ir,
        semantic::VerifyOutcome {
            errors: pre.errors,
            warnings: pre.warnings,
        },
    ))
}

/// Parse, lower, and run the semantic verifier.
///
/// Returns the IR plus the list of semantic errors (empty list = clean).
/// Warnings are dropped — use [`check_source_diagnostics`] to see them.
pub fn check_source(source: &str) -> Result<(Program, crate::ir::X3IR, Vec<X3Error>), X3Error> {
    let (program, ir, outcome) = check_source_diagnostics(source)?;
    Ok((program, ir, outcome.errors))
}

/// Compile with an explicit mode, keeping the verifier's warnings.
///
/// `compile_with_mode` returns bytecode alone, so a caller that wanted the
/// warnings had no way to get them from the build path. That is how `x3c build
/// --deny-warnings` came to accept the flag and ignore it: `check` could see
/// the warnings and `build` could not, so the same source produced "clean" from
/// one command and a warning from the other. `compile_with_mode` now delegates
/// here, so the two cannot drift.
///
/// Warnings are returned even when compilation succeeds — that is the point.
/// Errors still take the failure path.
pub fn compile_with_mode_diagnostics(
    source: &str,
    mode: CompilationMode,
) -> Result<(Vec<u8>, semantic::VerifyOutcome), X3Error> {
    let program = parse_source(source)?;
    let mut pre = run_pre_emission_layers(&program, mode)?;
    if !pre.errors.is_empty() {
        return Err(format_pipeline_errors(&pre.errors));
    }
    let bytecode = emit_x3ir(&pre.ir)?;
    Ok((
        bytecode,
        semantic::VerifyOutcome {
            errors: Vec::new(),
            warnings: std::mem::take(&mut pre.warnings),
        },
    ))
}

/// Compile with an explicit compilation mode for mode-gated safety checks.
///
/// Use [`compile_with_mode_diagnostics`] when the warnings matter.
pub fn compile_with_mode(source: &str, mode: CompilationMode) -> Result<Vec<u8>, X3Error> {
    compile_with_mode_diagnostics(source, mode).map(|(bytecode, _)| bytecode)
}

/// Check source with an explicit compilation mode. Returns the program, IR,
/// and list of semantic errors. When mode is Mainnet, mainnet-specific safety
/// checks are also run.
///
/// Warnings are dropped — use [`check_source_diagnostics_with_mode`] to see
/// them.
pub fn check_source_with_mode(
    source: &str,
    mode: CompilationMode,
) -> Result<(Program, crate::ir::X3IR, Vec<X3Error>), X3Error> {
    let (program, ir, outcome) = check_source_diagnostics_with_mode(source, mode)?;
    Ok((program, ir, outcome.errors))
}

/// Run the semantic verifier against an X3IR program.
pub fn check_ir(ir: &crate::ir::X3IR) -> Result<(), Vec<X3Error>> {
    // The documented production entry for "verify with the default budgets".
    // This called `verify_with_config(.., DEFAULT, DEFAULT, None)` directly,
    // which is exactly `verify_with_defaults` — so that function was public,
    // documented as the one callers should use, and called by nobody.
    semantic::verify_with_defaults(ir)
}

/// Compile with explicit lowering context (for replay protection, chain_id, etc.)
pub fn compile_program_with_context(program: &Program, ctx: LowerCtx) -> Result<Vec<u8>, X3Error> {
    // Every verification layer, on the same funnel the source entry points use.
    // These two entry points used to lower and emit with no AST or IR checks at
    // all.
    let pre = run_pre_emission_layers_with_context(program, ctx, CompilationMode::Dev)?;
    if !pre.errors.is_empty() {
        return Err(format_pipeline_errors(&pre.errors));
    }

    // AST → X3IR
    let ir = pre.ir;

    // X3IR → Bytecode
    let bytecode = emit_x3ir(&ir)?;

    // Verify bytecode is valid (basic sanity checks)
    verify_bytecode(&bytecode)?;

    Ok(bytecode)
}

/// Get the IR without emitting to bytecode (useful for analysis/optimization)
pub fn compile_to_ir(program: &Program) -> Result<X3IR, X3Error> {
    lower_program(program, LowerCtx::new())
}

/// Re-export the cross-chain intent adapter boundary.
///
/// `x3-lang` does not depend on the cross-chain intent crate (no
/// dependency cycle). Instead the language compiler builds an
/// `IntentSpecDraft` (a JSON-serializable value) at the compiler
/// boundary, and the main workspace's
/// `x3-crosschain-intent::adapter::intent_spec_to_crosschain_intent`
/// is the single canonical consumer that converts the draft into
/// a fully-validated `CrossChainIntent` and stamps the canonical
/// hash.
pub use intent_emit::{IntentSpecDraft, SourceConstraint};

/// Verify bytecode is properly formed (basic checks)
fn verify_bytecode(bytecode: &[u8]) -> Result<(), X3Error> {
    if bytecode.is_empty() {
        return Err(X3Error::CodegenError {
            message: "bytecode is empty".to_string(),
            span: None,
        });
    }

    // Check version byte
    if bytecode[0] != 0x01 {
        return Err(X3Error::CodegenError {
            message: format!("invalid bytecode version: {}", bytecode[0]),
            span: None,
        });
    }

    // Check alignment
    if bytecode.len() % 4 != 0 {
        return Err(X3Error::CodegenError {
            message: "bytecode not 4-byte aligned".to_string(),
            span: None,
        });
    }

    Ok(())
}

#[cfg(test)]
mod regalloc_wiring_tests {
    use super::*;
    use x3_lang_ast::ast::{AssetRef, AtomicSwapDecl, ChainRef, Expression, HashlockSpec, Item, LiteralExpr, Program};
    use x3_lang_common::Spanned;

    /// The new `compile_program_with_regalloc` entry point runs the full
    /// pipeline (parse → lower → register-allocate → emit → verify). This
    /// test builds a valid `Program` directly so it doesn't depend on the
    /// surface syntax changing across compiler versions — we just need to
    /// prove the regalloc pass is reachable from the public API.
    #[test]
    fn compile_program_with_regalloc_runs_full_pipeline() {
        let swap = AtomicSwapDecl {
            name: "test_swap".into(),
            from_asset: AssetRef::new(ChainRef("eth".into()), "USDC".into()),
            to_asset: AssetRef::new(ChainRef("sol".into()), "USDC".into()),
            source_vm: None,
            dest_vm: None,
            amount: Some(Expression::Literal(LiteralExpr::Int {
                value: 100,
                base: x3_lang_common::IntBase::Decimal,
                suffix: None,
            })),
            receiver: None,
            hashlock: Some(HashlockSpec {
                hash_fn: "sha256".into(),
                secret: Box::new(Expression::Literal(LiteralExpr::String("my_secret".into()))),
            }),
            body: vec![],
            requires: vec![],
            on_fail: None,
            timeout_source: Some(Expression::Literal(LiteralExpr::Duration {
                value: 3600,
                unit: x3_lang_common::DurationUnit::Seconds,
            })),
            timeout_destination: Some(Expression::Literal(LiteralExpr::Duration {
                value: 1800,
                unit: x3_lang_common::DurationUnit::Seconds,
            })),
        };
        let program = Program {
            items: vec![Spanned::dummy(Item::AtomicSwap(swap))],
        };

        // This fixture used to be rejected by lowering, so the call below
        // always returned `Err` and everything the `Ok` arm asserted went
        // unexercised — including `alloc.len() == 0`, which the arm only
        // stopped agreeing with once the pipeline started succeeding. The
        // fixture is valid, so the pipeline must succeed; tolerating `Err` was
        // how the untested arm survived.
        let (bytecode, alloc) = compile_program_with_regalloc(&program)
            .expect("a valid hand-built program must compile through the regalloc-wired pipeline");
        assert!(!bytecode.is_empty(), "bytecode must be non-empty");
        assert_eq!(bytecode[0], 0x01, "bytecode version must be 0x01");
        assert_eq!(bytecode.len() % 4, 0, "bytecode must be 4-byte aligned");
        // What the allocation record owes is to mirror the IR the pass was
        // handed. `registers_used` and `spills_used` are legitimately zero
        // here: the fixture is Lock/Release/OnTimeout, which carry no register
        // operands, so there is no temporary to place. Asserting those two are
        // zero would be asserting the pass found nothing to do; asserting the
        // record is empty was asserting the same, and was false.
        let ir_len = compile_to_ir(&program).expect("lowering must succeed").operations.len();
        assert_eq!(
            alloc.operations.len(),
            ir_len,
            "the allocation record must mirror the IR operation list"
        );
    }

    /// Fails fast on a syntactically broken source, proving the parse →
    /// regalloc path is wired into the same error surface as the standard
    /// `compile_program` entry point.
    #[test]
    fn compile_program_with_regalloc_propagates_parse_errors() {
        let result = compile_program_with_regalloc_str("this is not x3-lang");
        assert!(result.is_err(), "garbage source must error out");
    }

    /// String-source convenience wrapper that mirrors `compile_source` but
    /// goes through the regalloc-wired pipeline.
    pub fn compile_program_with_regalloc_str(source: &str) -> Result<Vec<u8>, X3Error> {
        let program = parse_source(source)?;
        compile_program_with_regalloc(&program).map(|(bc, _)| bc)
    }
}
