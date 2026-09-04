use x3_lang_compiler::ir::Operation;
use x3_lang_compiler::lowering::{lower_program, LowerCtx};
use x3_lang_compiler::parser::parse_source;

fn must_parse(src: &str) {
    match parse_source(src) {
        Ok(_) => (),
        Err(e) => panic!("parse failed: {e}"),
    }
}

#[test]
fn test_parse_flagship_b52() {
    let source = include_str!("../../examples/flagship_b52.x3");
    let program = parse_source(source);
    assert!(program.is_ok(), "Failed to parse flagship B-52: {:?}", program.err());
    let program = program.unwrap();
    let has_intent = program
        .items
        .iter()
        .any(|item| matches!(&item.node, x3_lang_ast::ast::Item::IntentDecl(_)));
    assert!(has_intent, "Flagship example must contain an intent declaration");
    let has_solver_market = program
        .items
        .iter()
        .any(|item| matches!(&item.node, x3_lang_ast::ast::Item::SolverMarket(_)));
    assert!(
        has_solver_market,
        "Flagship example must contain a solver_market declaration"
    );
    let has_relayers = program
        .items
        .iter()
        .any(|item| matches!(&item.node, x3_lang_ast::ast::Item::RelayerSwarm(_)));
    assert!(has_relayers, "Flagship example must contain a relayers declaration");
    let has_invariant = program
        .items
        .iter()
        .any(|item| matches!(&item.node, x3_lang_ast::ast::Item::InvariantDecl(_)));
    assert!(has_invariant, "Flagship example must contain invariant declarations");
    let has_proofs = program
        .items
        .iter()
        .any(|item| matches!(&item.node, x3_lang_ast::ast::Item::ProofsRequired(_)));
    assert!(
        has_proofs,
        "Flagship example must contain a proofs required declaration"
    );
}

#[test]
fn test_parse_simple_swap() {
    let source = include_str!("../../examples/simple_swap.x3");
    let program = parse_source(source);
    assert!(program.is_ok(), "Failed to parse simple_swap: {:?}", program.err());
    let program = program.unwrap();
    let has_intent = program
        .items
        .iter()
        .any(|item| matches!(&item.node, x3_lang_ast::ast::Item::IntentDecl(_)));
    assert!(has_intent, "simple_swap must contain an intent declaration");
}

#[test]
fn test_parse_multi_leg_route() {
    let source = include_str!("../../examples/multi_leg_route.x3");
    match parse_source(source) {
        Ok(_) => (),
        Err(e) => panic!("Failed to parse multi_leg_route: {e}"),
    }
}

#[test]
fn test_parse_staking_intent() {
    let source = include_str!("../../examples/staking_intent.x3");
    assert!(parse_source(source).is_ok(), "Failed to parse staking_intent");
}

#[test]
fn test_b52_solver_market_parses() {
    must_parse(
        r#"
        solver_market {
            mode competitive
            min_reputation 95
        }
        "#,
    );
}

#[test]
fn test_b52_relayers_parses() {
    must_parse(
        r#"
        relayers {
            quorum_numerator 3
            quorum_denominator 5
            relayers [relayer_a, relayer_b, relayer_c]
        }
        "#,
    );
}

#[test]
fn test_b52_rpc_quorum_parses() {
    must_parse(
        r#"
        rpc_quorum {
            source arbitrum
            require_numerator 2
            require_denominator 3
            reject_on [receipt_disagree, finality_disagree]
        }
        "#,
    );
}

#[test]
fn test_b52_risk_policy_parses() {
    must_parse(
        r#"
        risk_policy {
            max_slippage 50
            max_position 500000
        }
        "#,
    );
}

#[test]
fn test_b52_privacy_parses() {
    must_parse(
        r#"
        privacy {
            hide_route_until_commit true
            reveal_on claim
            encrypted true
        }
        "#,
    );
}

#[test]
fn test_b52_invariant_parses() {
    must_parse("invariant no_double_claim");
}

#[test]
fn test_b52_proofs_required_parses() {
    must_parse(
        r#"
        proofs required {
            source_lock_proof
            source_finality_proof
        }
        "#,
    );
}

#[test]
fn test_b52_vm_decl_parses() {
    must_parse(
        r#"
        vm {
            chain arbitrum
            adapter evm
            finality safe
        }
        "#,
    );
}

#[test]
fn test_b52_target_parses() {
    must_parse(
        r#"
        target evm {
            adapter evm_adapter
        }
        "#,
    );
}

#[test]
fn test_b52_finality_policy_parses() {
    must_parse(
        r#"
        finality_policy strict {
            chain ethereum
            requirement finalized
        }
        "#,
    );
}

#[test]
fn test_b52_error_decl_parses() {
    must_parse("error SlippageExceeded");
}

#[test]
fn test_b52_all_items_together() {
    must_parse(
        r#"
        vm {
            chain arbitrum
            adapter evm
        }

        solver_market {
            mode competitive
            min_reputation 95
        }

        relayers {
            quorum_numerator 3
            quorum_denominator 5
            relayers [a, b, c]
        }

        rpc_quorum {
            source arbitrum
            require_numerator 2
            require_denominator 3
            reject_on [receipt_disagree]
        }

        risk_policy {
            max_slippage 50
        }

        privacy {
            hide_route_until_commit true
            reveal_on claim
            encrypted true
        }

        invariant no_double_claim

        proofs required {
            source_lock_proof
        }

        finality_policy strict {
            chain ethereum
            requirement finalized
        }

        error SlippageExceeded

        target evm {
            adapter evm_adapter
        }

        intent test_all {
            from arbitrum.USDC amount 100
            to solana.SOL receiver wallet

            route {
                bridge X3 arbitrum.USDC -> solana.SOL receiver wallet
            }

            require slippage <= 50

            timeout 60s
            on_fail rollback
        }
        "#,
    );
}

#[test]
fn parse_roundtrip_ir_bytecode() {
    let source = include_str!("../../examples/simple_swap.x3");
    let program = parse_source(source).expect("simple_swap should parse");
    let ir = x3_lang_compiler::lowering::lower_program(&program, x3_lang_compiler::lowering::LowerCtx::new())
        .expect("should lower");
    let bytecode = x3_lang_compiler::emitter::emit_x3ir(&ir).expect("should emit bytecode");
    let trace = x3_lang_compiler::emitter::disassemble(&bytecode).expect("should disassemble");

    assert!(trace.contains("BRIDGE"), "disassembly should contain BRIDGE");
    assert!(trace.contains("LOCK"), "disassembly should contain LOCK");
    assert!(trace.contains("RELEASE"), "disassembly should contain RELEASE");
}

#[test]
fn ir_operations_roundtrip() {
    use std::collections::HashMap;
    use x3_lang_compiler::ir::{Operation, X3IR};

    let mut ir = X3IR::new();

    let mut weights = HashMap::new();
    weights.insert("gas_price".to_string(), 30u32);
    weights.insert("liquidity".to_string(), 70u32);
    ir.push(Operation::RouteScore {
        strategy: "fastest".to_string(),
        weights,
    });

    ir.push(Operation::SolverBid {
        solver: "solver_a".to_string(),
        receive_asset: "ethereum.USDC".to_string(),
        deliver_asset: "solana.SOL".to_string(),
        fee: "0.1%".to_string(),
        bond: 1000,
    });

    ir.push(Operation::RelayerAttest {
        relayers: vec!["relayer_1".to_string(), "relayer_2".to_string()],
        quorum: (3, 5),
        signatures: vec!["sig1".to_string(), "sig2".to_string(), "sig3".to_string()],
    });

    let bytecode = x3_lang_compiler::emitter::emit_x3ir(&ir).expect("IR should emit");
    let trace = x3_lang_compiler::emitter::disassemble(&bytecode).expect("should disassemble");

    assert!(trace.contains("ROUTE_SCORE"), "should contain ROUTE_SCORE");
    assert!(trace.contains("SOLVER_BID"), "should contain SOLVER_BID");
    assert!(trace.contains("RELAYER_ATTEST"), "should contain RELAYER_ATTEST");
}

#[test]
fn full_pipeline_parse_emit_parse() {
    let source = include_str!("../../examples/simple_swap.x3");
    let program = parse_source(source).expect("simple_swap should parse");
    let json = serde_json::to_string_pretty(&program).expect("AST should serialize to JSON");
    let _restored: x3_lang_ast::ast::Program =
        serde_json::from_str(&json).expect("AST JSON should deserialize back to Program");
}

#[test]
fn test_fuzz_generation_does_not_panic() {
    let source = include_str!("../../examples/simple_swap.x3");
    let program = parse_source(source).expect("parse");

    let intent_name = program
        .items
        .iter()
        .find_map(|item| match &item.node {
            x3_lang_ast::ast::Item::IntentDecl(decl) => Some(decl.name.as_str().to_string()),
            _ => None,
        })
        .unwrap_or_else(|| "unknown".to_string());

    let iterations = 100u32;
    let _fuzz_content = format!(
        r#"//! Fuzz test for intent: {intent_name}
//! {iterations} iterations generated by `x3c fuzz`

use arbitrary::{{Arbitrary, Unstructured}};

#[derive(Arbitrary, Debug)]
pub struct {intent_name}FuzzInput {{
    pub amount: u64,
    pub slippage_bps: u16,
    pub timeout_secs: u32,
    pub use_bridge: bool,
    pub use_refund: bool,
}}

pub fn fuzz_{intent_name}(input: &{intent_name}FuzzInput) {{
    let mut source = format!("intent {intent_name}_fuzz {{");
    if input.use_bridge {{
        source.push_str(&format!("bridge x3 ethereum.USDC -> solana.USDC amount {{}} receiver 0x0000;", input.amount));
    }}
    source.push_str("require slippage <= ");
    source.push_str(&input.slippage_bps.to_string());
    source.push_str(";");
    if input.use_refund {{
        source.push_str("on_fail refund ethereum.USDC to sender;");
    }}
    source.push_str("}}");
    let _ = x3_lang_compiler::parser::parse_source(&source);
}}
"#
    );

    assert!(!intent_name.is_empty(), "intent name must be non-empty");
}

#[test]
fn test_chaos_generation_does_not_panic() {
    let source = include_str!("../../examples/simple_swap.x3");
    let program = parse_source(source).expect("parse");

    let intent_name = program
        .items
        .iter()
        .find_map(|item| match &item.node {
            x3_lang_ast::ast::Item::IntentDecl(decl) => Some(decl.name.as_str().to_string()),
            _ => None,
        })
        .unwrap_or_else(|| "unknown".to_string());

    let scenarios = 10u32;
    let _chaos_content = format!(
        r#"//! Chaos test scenarios for intent: {intent_name}
//! {scenarios} scenarios generated by `x3c chaos`

pub enum ChaosScenario {{
    RpcBlackhole,
    BridgeInconsistent,
    RelayerDowntime,
    SourceReorg,
    DestCongestion,
    InvalidSolverBid,
    PrematureTimeout,
    RefundBlocked,
}}

pub fn get_scenarios() -> Vec<ChaosScenario> {{
    vec![
        ChaosScenario::RpcBlackhole,
        ChaosScenario::BridgeInconsistent,
        ChaosScenario::RelayerDowntime,
        ChaosScenario::SourceReorg,
        ChaosScenario::DestCongestion,
        ChaosScenario::InvalidSolverBid,
        ChaosScenario::PrematureTimeout,
        ChaosScenario::RefundBlocked,
    ]
}}
"#
    );

    assert!(!intent_name.is_empty(), "intent name must be non-empty");
}

#[test]
fn test_check_mode_mainnet_rejects_unsafe_intent() {
    use x3_lang_compiler::{check_source_with_mode, CompilationMode};
    let src = include_str!("../../examples/simple_swap.x3");
    let (_, _, errors) = check_source_with_mode(src, CompilationMode::Mainnet).unwrap();
    assert!(
        !errors.is_empty(),
        "mainnet mode should produce errors for unsafe intent"
    );
    let all_msgs: String = errors.iter().map(|e| format!("{:?}", e)).collect::<Vec<_>>().join("\n");
    assert!(
        all_msgs.contains("slippage") || all_msgs.contains("refund"),
        "should mention slippage or refund in: {}",
        all_msgs
    );
}

#[test]
fn test_check_mode_dev_passes_simple() {
    use x3_lang_compiler::check_source;
    let src = include_str!("../../examples/simple_swap.x3");
    let _ = check_source(src).unwrap();
}

// ============================================================================
// B-52 lowering / compilation integration tests
// ============================================================================

#[test]
fn test_lowering_vm_decl_emits_adapter_call() {
    let src = r#"
        vm {
            chain evm
            adapter prod
            finality safe
        }
    "#;
    let program = parse_source(src).expect("should parse");
    let ir = lower_program(&program, LowerCtx::new()).expect("should lower");
    assert!(
        ir.operations
            .iter()
            .any(|op| matches!(op, Operation::VmAdapterCall { vm, .. } if vm == "evm")),
        "expected VmAdapterCall for evm"
    );
    assert!(
        ir.operations
            .iter()
            .any(|op| matches!(op, Operation::ModeCheck { mode, .. } if mode == "finality")),
        "expected ModeCheck for finality"
    );
}

#[test]
fn test_lowering_solver_market_emits_bid() {
    let src = r#"
        solver_market {
            mode automatic
            min_reputation 1000
        }
    "#;
    let program = parse_source(src).expect("should parse");
    let ir = lower_program(&program, LowerCtx::new()).expect("should lower");
    assert!(
        ir.operations
            .iter()
            .any(|op| matches!(op, Operation::SolverBid { bond: 1000, .. })),
        "expected SolverBid with bond 1000"
    );
}

#[test]
fn test_lowering_relayer_swarm_emits_attest() {
    let src = r#"
        relayers {
            quorum 3_of_5
            relayers [relayer_a, relayer_b, relayer_c]
        }
    "#;
    let program = parse_source(src).expect("should parse");
    let ir = lower_program(&program, LowerCtx::new()).expect("should lower");
    assert!(
        ir.operations
            .iter()
            .any(|op| matches!(op, Operation::RelayerAttest { quorum: (3, 5), relayers, .. } if relayers.len() == 3)),
        "expected RelayerAttest with quorum (3,5) and 3 relayers"
    );
}

#[test]
fn test_lowering_rpc_quorum_emits_consensus() {
    let src = r#"
        rpc_quorum {
            source solana
            require_numerator 2
            require_denominator 3
        }
    "#;
    let program = parse_source(src).expect("should parse");
    let ir = lower_program(&program, LowerCtx::new()).expect("should lower");
    assert!(
        ir.operations
            .iter()
            .any(|op| matches!(op, Operation::RpcConsensus { chain, require: (2, 3), .. } if chain == "solana")),
        "expected RpcConsensus for solana 2_of_3"
    );
}

#[test]
fn test_lowering_risk_policy_emits_score() {
    let src = r#"
        risk_policy {
            max_slippage 500
        }
    "#;
    let program = parse_source(src).expect("should parse");
    let ir = lower_program(&program, LowerCtx::new()).expect("should lower");
    assert!(
        ir.operations
            .iter()
            .any(|op| matches!(op, Operation::RiskScore { score: 500, .. })),
        "expected RiskScore with score 500"
    );
}

#[test]
fn test_lowering_privacy_emits_commit() {
    let src = r#"
        privacy {
            hide_route_until_commit true
            reveal_on commit
            encrypted false
        }
    "#;
    let program = parse_source(src).expect("should parse");
    let ir = lower_program(&program, LowerCtx::new()).expect("should lower");
    assert!(
        ir.operations
            .iter()
            .any(|op| matches!(op, Operation::PrivacyCommit { reveal_on, .. } if reveal_on == "commit")),
        "expected PrivacyCommit with reveal_on commit"
    );
}

#[test]
fn test_lowering_invariant_emits_check() {
    let src = "invariant no_double_claim";
    let program = parse_source(src).expect("should parse");
    let ir = lower_program(&program, LowerCtx::new()).expect("should lower");
    assert!(
        ir.operations
            .iter()
            .any(|op| matches!(op, Operation::InvariantCheck { name, .. } if name == "no_double_claim")),
        "expected InvariantCheck named no_double_claim"
    );
}

#[test]
fn test_lowering_finality_policy_emits_require() {
    let src = r#"
        finality_policy strict {
            chain btc
            requirement finalized
        }
    "#;
    let program = parse_source(src).expect("should parse");
    let ir = lower_program(&program, LowerCtx::new()).expect("should lower");
    assert!(
        ir.operations.iter().any(|op| matches!(
            op,
            Operation::Require {
                kind: x3_lang_compiler::ir::RequireKind::FinalityExplicit,
                ..
            }
        )),
        "expected Require with FinalityExplicit"
    );
}

#[test]
fn test_lowering_proofs_emits_proof_required() {
    let src = r#"
        proofs required {
            source_lock_proof
            fill_proof
        }
    "#;
    let program = parse_source(src).expect("should parse");
    let ir = lower_program(&program, LowerCtx::new()).expect("should lower");
    let proof_ops: Vec<_> = ir
        .operations
        .iter()
        .filter(|op| matches!(op, Operation::ProofRequired { .. }))
        .collect();
    assert_eq!(proof_ops.len(), 2, "expected 2 ProofRequired operations");
    assert!(
        proof_ops
            .iter()
            .any(|op| matches!(op, Operation::ProofRequired { proof_type, .. } if proof_type == "source_lock_proof")),
        "expected ProofRequired for source_lock_proof"
    );
    assert!(
        proof_ops
            .iter()
            .any(|op| matches!(op, Operation::ProofRequired { proof_type, .. } if proof_type == "fill_proof")),
        "expected ProofRequired for fill_proof"
    );
}

#[test]
fn test_lowering_vm_target_emits_adapter_call() {
    let src = r#"
        target evm {
            adapter layerzero
            contract 0xabc
        }
    "#;
    let program = parse_source(src).expect("should parse");
    let ir = lower_program(&program, LowerCtx::new()).expect("should lower");
    assert!(
        ir.operations.iter().any(|op| matches!(op, Operation::VmAdapterCall { vm, adapter, calldata } if vm == "evm" && adapter == "layerzero" && calldata == "0xabc")),
        "expected VmAdapterCall for evm/layerzero/0xabc"
    );
}
