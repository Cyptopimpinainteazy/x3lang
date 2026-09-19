//! PHASE 37 end to end: an `arb` block is checked against the program's own graph,
//! lowered into the decided contract, and refused at build time because the plan
//! generator does not exist yet.

use x3_lang_compiler::diagnostic::DiagnosticCode;
use x3_lang_compiler::emitter::emit_x3ir;
use x3_lang_compiler::formatter::X3Formatter;
use x3_lang_compiler::ir::{Operation, ProgramMetadata, X3IR};
use x3_lang_compiler::verify::verify_ir;
use x3_lang_compiler::{check_source_diagnostics_with_mode, compile_source, compile_to_ir, CompilationMode};

/// Two chains and three venues, one of them a flash lender: the graph the `arb` block
/// below has to agree with.
const GRAPH: &str = r#"
venue deep_pool {
    kind pool
    chain ethereum
    domain evm
    asset_in ethereum.USDC
    asset_out ethereum.ETH
    fee_bps 7
    liquidity 4_000_000
    slippage_bps 8
    latency_ms 12
    finality_blocks 12
    risk 2
}

venue flash_lender {
    kind flash
    chain ethereum
    domain evm
    asset_in ethereum.USDC
    asset_out ethereum.USDC
    fee_bps 9
    liquidity 50_000_000
    slippage_bps 1
    latency_ms 12
    finality_blocks 12
    risk 1
}

venue base_pool {
    kind pool
    chain base
    domain evm
    asset_in base.USDC
    asset_out base.ETH
    fee_bps 5
    liquidity 2_000_000
    slippage_bps 9
    latency_ms 11
    finality_blocks 20
    risk 3
}
"#;

const SOUND: &str = r#"
arb {
    discover {
        chains = [ethereum, base];
        max_hops = 4;
        liquidity_min = 500_000 USDC;
    }
    capital {
        flash = enabled;
        max = 2_000_000 USDC;
    }
    execution {
        atomic = true;
        parallel = true;
    }
    risk {
        min_profit = 20bps;
        max_slippage = 10bps;
        max_total_fee = 40bps;
        deadline = 220ms;
    }
}
"#;

fn source(arb: &str) -> String {
    format!("{GRAPH}\n{arb}")
}

fn arb_plan(program: &x3_lang_ast::ast::Program) -> Operation {
    let ir = compile_to_ir(program).expect("the fixture lowers");
    ir.operations
        .into_iter()
        .find(|operation| matches!(operation, Operation::ArbPlan { .. }))
        .expect("the fixture carries an arb plan")
}

#[test]
fn a_sound_arb_block_decides_its_contract_and_says_the_plan_cannot_run() {
    let text = source(SOUND);
    let (program, _, outcome) =
        check_source_diagnostics_with_mode(&text, CompilationMode::Dev).expect("the fixture parses");
    // The bounds are decided — the contract is asserted below — and the program is
    // still refused, because no runtime follows the plan. `check` must not accept what
    // `build` then refuses: the same shape PHASE 11's rebalance takes.
    assert_eq!(outcome.errors.len(), 1, "{:?}", outcome.errors);
    let refusal = format!("{}", outcome.errors[0]);
    assert!(refusal.contains("`arb` plan cannot be executed"), "{refusal}");

    let Operation::ArbPlan {
        chains,
        max_hops,
        depth_floor,
        flash_max,
        parallel,
        private,
        min_profit_bps,
        max_slippage_bps,
        max_total_fee_bps,
        deadline_ms,
        admitted,
    } = arb_plan(&program)
    else {
        unreachable!("the helper found an arb plan")
    };
    assert_eq!(chains, vec!["ethereum", "base"]);
    assert_eq!(max_hops, 4);
    // The committed size is the flash ceiling, not the discovery floor.
    assert_eq!(depth_floor, ("USDC".to_string(), 2_000_000));
    assert_eq!(flash_max, Some(("USDC".to_string(), 2_000_000)));
    assert!(parallel && !private);
    assert_eq!(
        (min_profit_bps, max_slippage_bps, max_total_fee_bps, deadline_ms),
        (20, 10, 40, 220)
    );
    assert_eq!(admitted, vec!["deep_pool", "flash_lender", "base_pool"]);
}

#[test]
fn a_decided_contract_is_refused_at_build_because_the_generator_does_not_exist() {
    let error = compile_source(&source(SOUND)).expect_err("no artifact can exist yet");
    let message = format!("{error}");
    assert!(message.contains("`arb` plan cannot be executed"), "{message}");
    assert!(message.contains("cannot yet generate the plan"), "{message}");
}

#[test]
fn check_refuses_a_chain_the_graph_does_not_host() {
    let text = source(&SOUND.replace("[ethereum, base]", "[ethereum, solana]"));
    let (_, _, outcome) = check_source_diagnostics_with_mode(&text, CompilationMode::Dev).expect("the fixture parses");
    let messages: Vec<String> = outcome.errors.iter().map(|error| format!("{error}")).collect();
    assert!(
        messages
            .iter()
            .any(|message| message.contains("hosts no declared venue")),
        "{messages:?}"
    );
}

fn plan_ir() -> X3IR {
    X3IR {
        operations: vec![Operation::ArbPlan {
            chains: vec!["ethereum".to_string()],
            max_hops: 2,
            depth_floor: ("USDC".to_string(), 1_000),
            flash_max: None,
            parallel: false,
            private: false,
            min_profit_bps: 20,
            max_slippage_bps: 10,
            max_total_fee_bps: 40,
            deadline_ms: 220,
            admitted: vec!["deep_pool".to_string()],
        }],
        metadata: ProgramMetadata {
            nonce: Some("nonce-1".to_owned()),
            chain_id: Some(1),
            timeout_blocks: Some(10),
        },
    }
}

#[test]
fn both_layers_refuse_an_arb_plan_because_no_runtime_follows_it() {
    // The IR verifier is where `check` reaches the refusal, and the emitter is public
    // API: a caller that emits without verifying must hit the same wall, because a
    // `check` that accepts what `build` refuses is the split the IR verifier exists to
    // prevent.
    let diagnostics = verify_ir(&plan_ir()).expect_err("the plan cannot be executed");
    assert_eq!(
        diagnostics.iter().map(|diagnostic| diagnostic.code).collect::<Vec<_>>(),
        vec![DiagnosticCode::UnsafeIr]
    );
    assert!(
        diagnostics[0].message.contains("`arb` plan cannot be executed"),
        "{}",
        diagnostics[0].message
    );

    let error = emit_x3ir(&plan_ir()).expect_err("the emitter refuses the same plan");
    let message = format!("{error}");
    assert!(message.contains("cannot emit the `arb` plan"), "{message}");
    assert!(message.contains("TICKET-071"), "{message}");
}

#[test]
fn the_parser_refuses_a_bare_deadline_a_bare_rate_and_a_repeated_clause() {
    let bare_deadline = source(&SOUND.replace("deadline = 220ms", "deadline = 220"));
    let error = check_source_diagnostics_with_mode(&bare_deadline, CompilationMode::Dev)
        .expect_err("a bare number is a count of blocks");
    let message = format!("{error}");
    assert!(message.contains("count of blocks"), "{message}");

    let bare_rate = source(&SOUND.replace("min_profit = 20bps", "min_profit = 20"));
    let error =
        check_source_diagnostics_with_mode(&bare_rate, CompilationMode::Dev).expect_err("a rate needs its unit");
    let message = format!("{error}");
    assert!(message.contains("write `20bps`"), "{message}");

    let twice = source(&SOUND.replace("max_hops = 4;", "max_hops = 4;\n        max_hops = 5;"));
    let error = check_source_diagnostics_with_mode(&twice, CompilationMode::Dev)
        .expect_err("the second copy would replace the first");
    let message = format!("{error}");
    assert!(message.contains("written twice"), "{message}");
}

#[test]
fn the_formatter_writes_an_arb_block_the_parser_reads_back_identically() {
    // The bytecode round-trip in `test_formatter_roundtrip.rs` cannot cover this
    // surface — an `arb` program is refused at emit by design — so the property is
    // checked on the AST: what the formatter wrote has to parse back to what it was
    // handed, or `x3c fmt` would be rewriting a declaration.
    let program = x3_lang_compiler::parser::parse_source(&source(SOUND)).expect("the fixture parses");
    let formatted = X3Formatter::new().format_program(&program);
    let reparsed = x3_lang_compiler::parser::parse_source(&formatted).unwrap_or_else(|error| {
        panic!("the formatter wrote text the parser does not read: {error}\n--- formatted ---\n{formatted}")
    });
    // The declaration nodes, not the file's byte offsets: the formatted text is laid
    // out differently, so the spans differ by construction while the declarations have
    // to match field for field.
    let before: Vec<&x3_lang_ast::ast::Item> = program.items.iter().map(|item| &item.node).collect();
    let after: Vec<&x3_lang_ast::ast::Item> = reparsed.items.iter().map(|item| &item.node).collect();
    assert_eq!(
        serde_json::to_value(before).expect("the AST serializes"),
        serde_json::to_value(after).expect("the AST serializes"),
        "--- formatted ---\n{formatted}"
    );
    // And formatting what it just wrote is a no-op, or `--check` is not stable.
    assert_eq!(X3Formatter::new().format_program(&reparsed), formatted);
}
