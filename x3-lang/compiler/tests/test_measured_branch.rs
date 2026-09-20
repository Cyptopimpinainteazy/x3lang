//! An `if` on a quantity a host measured is compiled, written as a walkable record, and decided at
//! run time by the figure a venue reported (TICKET-106).
//!
//! `if`/`while` were refused because the VM branches on a register and this compiler emits no
//! arithmetic, so no source-level condition could be put in one — and the quantities a `.x3` program
//! *can* name are the three a host measures, which the VM already holds. That is the class this
//! file holds: the condition reaches the IR as [`Condition::Measured`], the emitter writes two
//! records with the bodies between them (there is no unconditional jump in this format), the
//! artifact's version byte moves to 2 because that is where the instruction was introduced, and the
//! VM forks on the comparison — while a quantity *nothing* reported refuses, in a branch as in a
//! guard.

use x3_lang_compiler::emitter::{disassemble, emit_x3ir, instructions};
use x3_lang_compiler::ir::{
    ComparisonOp, Condition, FailureAction, MeasuredQuantity, Operation, ProgramMetadata, X3IR,
};
use x3_lang_compiler::semantic::CompilationMode;
use x3_lang_compiler::spec::opcodes::{BYTECODE_VERSION_1, BYTECODE_VERSION_2, IF_MEASURED};
use x3_lang_vm::x3_lang_vm::{VMConfig, VM};

/// A strategy module whose `execute` body holds `body`, with the swap and guards its effect
/// declarations require.
fn module(body: &str) -> String {
    format!(
        r#"strategy Measured {{
    input ethereum.USDC amount 25_000_000 max 50_000_000
    output ethereum.ETH
    effects [swap]
    guarantees [min_profit]
    domains [ethereum]
    risk {{ max_slippage_bps 50 max_total_fee_bps 8 }}
    bounds {{ max_steps 10 max_gas 200_000 }}
    execute {{
        swap uniswap ethereum.USDC -> ethereum.ETH amount 1_000 min_output 1
        require slippage <= 50
        require profit >= 5
{body}
        on_fail refund ethereum.USDC to sender
    }}
}}
"#
    )
}

/// Lower a program, expecting it to check.
fn lowered(source: &str) -> X3IR {
    let (_, ir, outcome) = x3_lang_compiler::check_source_diagnostics_with_mode(source, CompilationMode::Dev)
        .unwrap_or_else(|error| panic!("the program must lower: {error}"));
    assert!(outcome.errors.is_empty(), "{:?}", outcome.errors);
    ir
}

/// The measured comparison the first branch in a program carries, as its three fields.
///
/// `Condition` has no `PartialEq` (the IR is compared through its rendering), so the fields are
/// compared rather than the value.
fn measured_branch_of(ir: &X3IR) -> Option<(MeasuredQuantity, ComparisonOp, u16)> {
    ir.operations.iter().find_map(|operation| match operation {
        Operation::If {
            condition:
                Condition::Measured {
                    quantity,
                    comparison,
                    threshold_bps,
                },
            ..
        } => Some((*quantity, *comparison, *threshold_bps)),
        _ => None,
    })
}

#[test]
fn a_comparison_of_a_measured_quantity_reaches_the_ir_as_one() {
    let ir = lowered(&module(
        "        if profit >= 20 {\n            require profit >= 20\n        } else {\n            require profit >= 5\n        }",
    ));
    assert_eq!(
        measured_branch_of(&ir),
        Some((MeasuredQuantity::ProfitBps, ComparisonOp::GreaterOrEqual, 20)),
        "the branch must say which quantity, which comparison and which bound"
    );
}

#[test]
fn the_negation_of_a_quantitys_own_comparison_is_the_other_comparison_it_supports() {
    // A branch needs both a body and the way around it, so `profit < 20` — the negation of the profit
    // floor — is the second shape, and `slippage > 50` the ceiling's.
    let ir = lowered(&module(
        "        if profit < 20 {\n            require profit >= 5\n        }",
    ));
    assert_eq!(
        measured_branch_of(&ir),
        Some((MeasuredQuantity::ProfitBps, ComparisonOp::Less, 20))
    );
    let ir = lowered(&module(
        "        if slippage > 50 {\n            require profit >= 5\n        }",
    ));
    assert_eq!(
        measured_branch_of(&ir),
        Some((MeasuredQuantity::SlippageBps, ComparisonOp::Greater, 50))
    );
}

#[test]
fn a_comparison_no_mode_makes_is_refused_by_name_rather_than_left_undecidable() {
    // The generic refusal ("the condition is not decidable at compile time") would send this
    // program's author looking for a way to make it decidable. The quantity *is* one the runtime
    // holds; the comparison is what nothing executes.
    let source = module("        if profit == 20 {\n            require profit >= 5\n        }");
    let error = x3_lang_compiler::check_source_diagnostics_with_mode(&source, CompilationMode::Dev)
        .err()
        .map(|error| error.to_string())
        .unwrap_or_else(|| "the program must be refused".to_string());
    assert!(
        error.contains("`profit == …`") && error.contains("`profit >= <bound>`"),
        "the refusal must name the comparison and the two it can be: {error}"
    );
}

#[test]
fn a_bound_that_is_not_a_whole_basis_point_is_refused() {
    // The guards' own exact rule, so the two cannot disagree: `0.005%` is half a basis point, which
    // this runtime cannot compare, and a second converter here would be a second answer.
    let source = module("        if profit >= 0.005% {\n            require profit >= 5\n        }");
    let error = x3_lang_compiler::check_source_diagnostics_with_mode(&source, CompilationMode::Dev)
        .err()
        .map(|error| error.to_string())
        .unwrap_or_else(|| "the program must be refused".to_string());
    assert!(
        error.contains("basis points") && error.contains("0.005%"),
        "the refusal must name the bound and the unit: {error}"
    );
}

#[test]
fn the_artifact_carries_two_records_with_the_bodies_between_them() {
    let ir = lowered(&module(
        "        if profit >= 20 {\n            require profit >= 20\n        } else {\n            require profit >= 5\n            require profit >= 6\n        }",
    ));
    let bytecode = emit_x3ir(&ir).expect("a measured branch must emit");
    let walked: Vec<_> = instructions(&bytecode)
        .expect("the artifact must be walkable")
        .into_iter()
        .filter(|instruction| instruction.opcode == IF_MEASURED)
        .collect();
    assert_eq!(walked.len(), 2, "a branch with an `else` needs both records");
    let first = x3_lang_compiler::spec::opcodes::parse_if_measured(walked[0].payload).expect("well formed");
    let second = x3_lang_compiler::spec::opcodes::parse_if_measured(walked[1].payload).expect("well formed");
    assert_eq!(
        (first.0, first.1, first.2),
        (0, false, 20),
        "the first record is the profit floor in its own direction (unit 0 is the profit)"
    );
    assert_eq!(
        (second.0, second.1, second.2),
        (0, true, 20),
        "the second is the inversion, which is what makes the then body run when it holds"
    );
    // The bodies are one `require` record each in this program, and both skips land on an
    // instruction rather than between two — the property, not the number.
    assert_eq!(first.3, 1, "the then body is one record");
    assert_eq!(second.3, 2, "and the else body is two");
    assert_skip_lands_on_an_instruction(&bytecode, walked[0].pc, first.3);
    assert_skip_lands_on_an_instruction(&bytecode, walked[1].pc, second.3);
    // And the reader renders it as the comparison a reviewer can check against the source.
    let trace = disassemble(&bytecode).expect("disassembly");
    assert!(trace.contains("IF_MEASURED profit >= 20bps, skip 1"), "{trace}");
    assert!(trace.contains("IF_MEASURED profit < 20bps, skip 2"), "{trace}");
}

#[test]
fn the_artifacts_version_byte_is_the_greatest_version_it_contains() {
    // TICKET-105's decision, held where it can fail: the writer writes the version its own opcodes
    // need — 2, because that is where `IF_MEASURED` was introduced — and a program that uses nothing
    // new stays version 1, so a reader that never learned version 2 keeps reading it.
    let measured = emit_x3ir(&lowered(&module(
        "        if profit >= 20 {\n            require profit >= 5\n        }",
    )))
    .expect("the branch must emit");
    assert_eq!(measured[0], BYTECODE_VERSION_2);
    assert!(
        instructions(&measured).is_ok(),
        "and the writer's own reader must accept what it wrote"
    );

    let plain = emit_x3ir(&lowered(&module("        require profit >= 6"))).expect("the program must emit");
    assert_eq!(
        plain[0], BYTECODE_VERSION_1,
        "a program with no version-2 opcode must not claim version 2"
    );
}

/// A program whose then body performs an asset operation, so *which* body ran is visible in the VM's
/// state.
///
/// Built as IR rather than source because a statement-position asset operation is the route and
/// atomic-choice grammars' to place, not a generic block's — the branch the parser can write is
/// covered above, and what this file needs to observe is the skip.
fn branch_with_an_asset_operation(comparison: ComparisonOp, threshold_bps: u16) -> X3IR {
    X3IR {
        operations: vec![
            Operation::AtomicBegin,
            Operation::If {
                condition: Condition::Measured {
                    quantity: MeasuredQuantity::ProfitBps,
                    comparison,
                    threshold_bps,
                },
                then_ops: vec![Operation::Lock {
                    chain: "ethereum".to_owned(),
                    asset: "USDC".to_owned(),
                    amount: 100,
                    from: "sender".to_owned(),
                }],
                else_ops: None,
            },
            Operation::OnFail {
                action: FailureAction::Rollback,
            },
            Operation::AtomicEnd,
        ],
        metadata: ProgramMetadata {
            nonce: Some("measured-branch".to_owned()),
            chain_id: Some(1),
            timeout_blocks: Some(10),
        },
    }
}

#[test]
fn the_vm_runs_the_body_only_when_the_comparison_holds() {
    let bytecode =
        emit_x3ir(&branch_with_an_asset_operation(ComparisonOp::GreaterOrEqual, 20)).expect("the branch must emit");

    let run = |profit: Option<u128>| {
        let mut vm = VM::new(bytecode.clone(), VMConfig::default(), 1_000_000);
        vm.report_outcome(profit, None, None);
        vm.execute()
    };

    // 25 >= 20: the body runs, and its `lock` reaches the VM's asset operations.
    let mut vm = VM::new(bytecode.clone(), VMConfig::default(), 1_000_000);
    vm.report_outcome(Some(25), None, None);
    vm.execute().expect("a profit above the bound must run");
    assert_eq!(
        vm.state.asset_ops.len(),
        1,
        "the then body's `lock` must have run: {:?}",
        vm.state.asset_ops
    );

    // 5 < 20: the body is skipped, and nothing reaches the state.
    let mut vm = VM::new(bytecode.clone(), VMConfig::default(), 1_000_000);
    vm.report_outcome(Some(5), None, None);
    vm.execute()
        .expect("a profit below the bound must run the empty branch");
    assert!(
        vm.state.asset_ops.is_empty(),
        "the body must have been skipped, so no asset operation may appear: {:?}",
        vm.state.asset_ops
    );

    // And the boundary is the comparison's own: at the bound the floor holds.
    assert!(run(Some(20)).is_ok(), "the floor is inclusive");
    let unmeasured = VM::new(bytecode.clone(), VMConfig::default(), 1_000_000)
        .execute()
        .expect_err("a branch on a quantity nothing reported must refuse");
    assert!(
        format!("{unmeasured:?}").contains("X3_GUARD_UNMEASURED"),
        "and say that nothing measured it: {unmeasured:?}"
    );
}

#[test]
fn the_negated_record_forks_the_other_way() {
    // `if profit < 20` is the same comparison with the inversion on the first record, and the VM must
    // honour the inversion rather than the quantity's own direction.
    let bytecode = emit_x3ir(&branch_with_an_asset_operation(ComparisonOp::Less, 20)).expect("the branch must emit");
    let walked: Vec<_> = instructions(&bytecode)
        .expect("walkable")
        .into_iter()
        .filter(|instruction| instruction.opcode == IF_MEASURED)
        .collect();
    assert_eq!(walked.len(), 1, "no `else`, so one record");
    let (unit, invert, threshold_bps, skip) =
        x3_lang_compiler::spec::opcodes::parse_if_measured(walked[0].payload).expect("well formed");
    assert_eq!((unit, invert, threshold_bps), (0, true, 20), "the inversion is set");
    assert!(skip > 0, "and the record skips the body it holds");
    assert_skip_lands_on_an_instruction(&bytecode, walked[0].pc, skip);

    // 5 < 20 holds → the body runs; 25 < 20 fails → the body is skipped.
    let mut vm = VM::new(bytecode.clone(), VMConfig::default(), 1_000_000);
    vm.report_outcome(Some(5), None, None);
    vm.execute().expect("run");
    assert_eq!(vm.state.asset_ops.len(), 1, "the body must run when the negation holds");

    let mut vm = VM::new(bytecode, VMConfig::default(), 1_000_000);
    vm.report_outcome(Some(25), None, None);
    vm.execute().expect("run");
    assert!(vm.state.asset_ops.is_empty(), "and be skipped when it does not");
}

/// The property a skip has to have, and the one a hardcoded figure cannot check: from the record's
/// own end, `skip` four-byte units lands exactly on a later instruction's start.
///
/// A skip that lands *between* instructions is the misparse this format has produced before; the
/// body's length is what the emitter computes, so this is the assertion that would fail if that
/// arithmetic were wrong.
fn assert_skip_lands_on_an_instruction(bytecode: &[u8], pc: usize, skip: u32) {
    let walked = instructions(bytecode).expect("walkable");
    let record = walked
        .iter()
        .find(|instruction| instruction.pc == pc)
        .unwrap_or_else(|| panic!("pc {pc} is not an instruction"));
    let target = record.next_pc + (skip as usize) * 4;
    assert!(
        walked.iter().any(|instruction| instruction.pc == target) || target == bytecode.len(),
        "the skip from pc {pc} lands at {target}, which is neither an instruction start nor the end \
         of the stream: {:?}",
        walked.iter().map(|instruction| instruction.pc).collect::<Vec<_>>()
    );
}
