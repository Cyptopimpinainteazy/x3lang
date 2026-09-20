//! End-to-end tests: compile x3 source programs → bytecode → VM execution.
//!
//! What this file holds is the round trip — a source the compiler accepts produces bytecode the VM
//! verifies and executes, and the program's operations reach the VM's state. It used to claim
//! "source-level IF, LOOP, REQUIRE, and ATOMIC constructs produce executable bytecode that behaves
//! correctly in the VM", which is not what it tested: no test here writes an `if` or a `loop`. Those
//! are covered by `test_branch_folding.rs` (a decided `if`), `test_loop_condition.rs` (a loop's
//! condition), and the bytecode-level tests in `vm/src/executor.rs` that assemble `IF`/`LOOP` by
//! hand (TICKET-108).
//!
//! Two of its tests also asserted nothing at all: one built a VM and discarded it, and one ended in
//! `assert!(result.is_ok() || result.is_err())`, which cannot fail. Every assertion below is the
//! outcome measured on the artifact the test itself builds — an `Err` from `execute`, a count of
//! asset operations, an opcode byte that has to be there — so each one can fail.

use x3_lang_compiler::compile_source;
use x3_lang_vm::x3_lang_vm::{VMConfig, VM};

const GAS: u128 = 1_000_000;

/// Compile `source` and execute it in a fresh VM, returning the VM that ran it.
fn run(source: &str) -> VM {
    let bytecode = compile_source(source).unwrap_or_else(|e| panic!("source compilation failed: {e:?}"));
    let mut vm = VM::new(bytecode, VMConfig::default(), GAS);
    // A program with an economic guard is judged against what a host measured — that is what makes
    // the guard enforced rather than recorded — so a dry run has to *state* the market outcome the
    // way a host would. The figures are the ones these fixtures' guards ask for: no slippage, and
    // the 5bps floor the strategy below writes, because a run that realised nothing does not
    // satisfy a floor and the VM says so.
    vm.report_outcome(Some(10), Some(0), None);
    vm.execute().unwrap_or_else(|e| panic!("VM execution failed: {e:?}"));
    vm
}

/// The source of a program whose `execute` body holds `body`, which the tests below run end to end.
fn strategy(body: &str) -> String {
    format!(
        r#"strategy RoundTrip {{
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

#[test]
fn a_compiled_program_reaches_the_vm_and_runs() {
    // Compiling is not enough: the artifact has to survive the verifier and run to the end of its
    // stream. Both are asserted, and the gas figure is what proves instructions were executed rather
    // than skipped — a program the compiler dropped would charge nothing.
    let vm = run(&strategy("        mempool_scan(max_results=10);"));
    assert!(
        vm.state.gas < GAS,
        "the VM must have executed instructions and charged for them: {} of {GAS} gas left",
        vm.state.gas
    );
    // Measured: the module's own `swap` is the one asset operation, and the extra statement is not
    // one. Asserting the count rather than "no operations" is the difference between a claim about
    // the program and a claim about the helper that built it.
    assert_eq!(
        vm.state.asset_ops.len(),
        1,
        "the route's swap is the program's one asset operation: {:?}",
        vm.state.asset_ops
    );
}

#[test]
fn a_let_binding_is_refused_rather_than_compiled_to_nothing() {
    // This test used to be `e2e_compiled_bytecode_executes_arithmetic` and asserted **nothing** — it
    // built a VM and dropped it. The program it built was `let x = 10; let y = 20;`, which the
    // compiler accepted and lowered to two `Nop`s, written as eight zero bytes that every reader
    // skips as padding: the artifact of that program was byte-identical to an empty program's.
    // It is refused by name now (TICKET-109), which is what this asserts — the arithmetic subject
    // stays covered, one stage earlier and visibly.
    let error = compile_source("fn add_fn() {\n    let x = 10;\n    let y = 20;\n}")
        .expect_err("a `let` binding has nowhere to go in this compiler");
    assert!(
        error.to_string().contains("`let x = …`"),
        "the refusal must name the binding: {error}"
    );
}

#[test]
fn a_role_guarded_program_runs_to_the_end_of_its_stream() {
    // The assertion here used to be `assert!(result.is_ok() || result.is_err())`, which is true for
    // every `Result` that exists. The measured outcome of this program is `Ok`.
    let source = r#"
        @role("keeper")
        fn scan() {
            mempool_scan(max_results=10);
        }
    "#;
    let bytecode = compile_source(source).expect("role-based source should compile");
    assert!(bytecode.contains(&0x93), "role check opcode");
    assert!(bytecode.contains(&0x88), "mempool scan opcode");

    let mut vm = VM::new(bytecode, VMConfig::default(), GAS);
    assert!(
        vm.execute().is_ok(),
        "the program must run to the end of its stream rather than fail part way"
    );
    assert!(vm.state.gas < GAS, "and be charged for what it executed");
}

#[test]
fn a_swap_intent_runs_and_its_asset_operations_reach_the_state() {
    let source = r#"
        intent guarded_swap {
            route {
                swap uniswap ethereum.USDC -> ethereum.ETH amount 1000 min_output 777;
            }
            require slippage <= 50
            timeout 30s refund ethereum.USDC to sender
            on_fail rollback
        }
    "#;
    let bytecode = compile_source(source).expect("swap intent should compile");
    assert!(!bytecode.is_empty(), "swap bytecode should not be empty");

    let mut vm = VM::new(bytecode, VMConfig::default(), GAS);
    // `require slippage <= 50` is a measured guard, so the caller states the slippage the run
    // realised; without one the VM refuses rather than assuming a number nobody measured.
    vm.report_outcome(Some(0), Some(0), None);
    vm.execute()
        .expect("the intent runs; a bridge would answer a dry-run host call, not fail the VM");
    // The property: the intent's operations reach the VM's state rather than being recorded and
    // dropped. Measured, the route's swap is one and its `on_fail refund` is the other.
    assert_eq!(
        vm.state.asset_ops.len(),
        2,
        "the route's swap and the refund path are two asset operations: {:?}",
        vm.state.asset_ops
    );
}

#[test]
fn a_multisig_program_runs() {
    let source = r#"
        @multisig(2, 3)
        fn guarded_op() {
            storage_store("test-data");
        }
    "#;
    let bytecode = compile_source(source).expect("multisig source should compile");
    assert!(bytecode.contains(&0x94), "multisig opcode");
    assert!(bytecode.contains(&0x86), "storage opcode");

    let mut vm = VM::new(bytecode, VMConfig::default(), GAS);
    assert!(
        vm.execute().is_ok(),
        "a multisig-guarded program must run to the end of its stream"
    );
}
