use std::collections::HashMap;

use x3_lang_compiler::diagnostic::DiagnosticCode;
use x3_lang_compiler::ir::{
    AssetKey, CompiledTradingPolicy, FailureAction, InvariantKind, Operation, ProgramMetadata, TradingOperation,
    ValueRef, X3IR,
};
use x3_lang_compiler::verify::verify_ir;

fn codes(ir: &X3IR) -> Vec<DiagnosticCode> {
    verify_ir(ir)
        .err()
        .unwrap_or_default()
        .into_iter()
        .map(|diagnostic| diagnostic.code)
        .collect()
}

fn ir_with(operations: Vec<Operation>) -> X3IR {
    X3IR {
        operations,
        metadata: ProgramMetadata {
            nonce: Some("nonce-1".to_owned()),
            chain_id: Some(1),
            timeout_blocks: Some(10),
        },
    }
}

#[test]
fn accepts_minimal_well_formed_atomic_sequence() {
    let ir = ir_with(vec![
        Operation::AtomicBegin,
        Operation::Lock {
            chain: "ethereum".to_owned(),
            asset: "USDC".to_owned(),
            amount: 100,
            from: "sender".to_owned(),
        },
        Operation::OnFail {
            action: FailureAction::Rollback,
        },
        Operation::AtomicEnd,
    ]);

    assert!(verify_ir(&ir).is_ok());
}

#[test]
fn rejects_atomic_end_without_begin() {
    let ir = ir_with(vec![Operation::AtomicEnd]);
    assert_eq!(codes(&ir), vec![DiagnosticCode::UnsafeIr]);
}

#[test]
fn rejects_unclosed_atomic_begin() {
    let ir = ir_with(vec![Operation::AtomicBegin, Operation::Nop]);
    assert_eq!(codes(&ir), vec![DiagnosticCode::UnsafeIr]);
}

#[test]
fn rejects_nested_atomic_scope() {
    let ir = ir_with(vec![
        Operation::AtomicBegin,
        Operation::AtomicBegin,
        Operation::AtomicEnd,
        Operation::AtomicEnd,
    ]);
    assert_eq!(codes(&ir), vec![DiagnosticCode::UnsafeIr]);
}

#[test]
fn rejects_zero_iteration_loop() {
    let ir = ir_with(vec![Operation::Loop {
        max_iterations: 0,
        body: vec![Operation::Nop],
    }]);
    assert_eq!(codes(&ir), vec![DiagnosticCode::UnsafeIr]);
}

#[test]
fn rejects_empty_host_call_identifier() {
    let ir = ir_with(vec![Operation::Call {
        function: String::new(),
        args: Vec::new(),
    }]);
    assert_eq!(codes(&ir), vec![DiagnosticCode::UnsafeIr]);
}

#[test]
fn rejects_invalid_multisig_threshold() {
    let ir = ir_with(vec![Operation::MultisigCheck { required: 3, total: 2 }]);
    assert_eq!(codes(&ir), vec![DiagnosticCode::UnsafeIr]);
}

#[test]
fn recursively_rejects_unsafe_nested_control_flow() {
    let ir = ir_with(vec![Operation::If {
        condition: x3_lang_compiler::ir::Condition::True,
        then_ops: vec![Operation::ScheduledDispatch {
            period_blocks: 0,
            entry: vec![Operation::Nop],
        }],
        else_ops: Some(vec![Operation::Emit {
            name: "ok".to_owned(),
            data: HashMap::new(),
        }]),
    }]);

    assert_eq!(codes(&ir), vec![DiagnosticCode::UnsafeIr]);
}

fn asset(symbol: &str) -> AssetKey {
    AssetKey {
        vm_family: "evm".to_owned(),
        chain: "ethereum".to_owned(),
        canonical_id: format!("0x{symbol}"),
        symbol: symbol.to_owned(),
        decimals: 6,
    }
}

fn trading_ir(ops: Vec<TradingOperation>) -> X3IR {
    ir_with(ops.into_iter().map(Operation::Trading).collect())
}

fn compiled_policy(id: &str) -> CompiledTradingPolicy {
    CompiledTradingPolicy {
        policy_id: id.to_owned(),
        policy_version: 1,
        chain: "ethereum".to_owned(),
        max_slippage_bps: 30,
        max_gas: 1_000_000,
        max_flash_fee_bps: 10,
        deadline_blocks: 10,
        require_private_submission: false,
        minimum_net_profit: None,
    }
}

fn valid_trading_ops() -> Vec<TradingOperation> {
    vec![
        TradingOperation::BeginAtomicTrade {
            trade_id: "T".to_owned(),
            policy: compiled_policy("P"),
        },
        TradingOperation::OpenDebt {
            debt_id: "debt".to_owned(),
            provider: "aave_v3".to_owned(),
            asset: asset("USDC"),
            principal: 1_000_000,
        },
        TradingOperation::ExecuteSwap {
            binding: "weth".to_owned(),
            venue: "uniswap_v3".to_owned(),
            from: asset("USDC"),
            to: asset("WETH"),
            input: ValueRef::Binding("debt.amount".to_owned()),
            min_output: 1,
        },
        TradingOperation::ExecuteSwap {
            binding: "returned".to_owned(),
            venue: "sushiswap".to_owned(),
            from: asset("WETH"),
            to: asset("USDC"),
            input: ValueRef::Binding("weth".to_owned()),
            min_output: 1,
        },
        TradingOperation::CloseDebt {
            debt_id: "debt".to_owned(),
        },
        TradingOperation::AssertMinNetProfit {
            settlement_asset: asset("USDC"),
            minimum: 1,
        },
        TradingOperation::AssertAllDebtsClosed,
        TradingOperation::EmitTradeReceipt,
        TradingOperation::CommitAtomicTrade,
    ]
}

#[test]
fn accepts_statefully_valid_trading_sequence() {
    assert!(verify_ir(&trading_ir(valid_trading_ops())).is_ok());
}

#[test]
fn rejects_swap_before_begin() {
    let mut ops = valid_trading_ops();
    let swap = ops.remove(2);
    ops.insert(0, swap);
    assert!(verify_ir(&trading_ir(ops)).is_err());
}

#[test]
fn rejects_duplicate_trading_begin() {
    let mut ops = valid_trading_ops();
    ops.insert(
        1,
        TradingOperation::BeginAtomicTrade {
            trade_id: "T2".to_owned(),
            policy: compiled_policy("P"),
        },
    );
    assert!(verify_ir(&trading_ir(ops)).is_err());
}

#[test]
fn rejects_binding_use_before_creation() {
    let mut ops = valid_trading_ops();
    if let TradingOperation::ExecuteSwap { input, .. } = &mut ops[2] {
        *input = ValueRef::Binding("missing".to_owned());
    }
    assert!(verify_ir(&trading_ir(ops)).is_err());
}

#[test]
fn rejects_binding_reuse_with_conflicting_asset() {
    let mut ops = valid_trading_ops();
    if let TradingOperation::ExecuteSwap { binding, .. } = &mut ops[3] {
        *binding = "weth".to_owned();
    }
    assert!(verify_ir(&trading_ir(ops)).is_err());
}

#[test]
fn rejects_repay_before_borrow() {
    let mut ops = valid_trading_ops();
    let close = ops.remove(4);
    ops.insert(1, close);
    assert!(verify_ir(&trading_ir(ops)).is_err());
}

#[test]
fn rejects_profit_guard_after_receipt() {
    let mut ops = valid_trading_ops();
    let profit = ops.remove(5);
    ops.insert(7, profit);
    assert!(verify_ir(&trading_ir(ops)).is_err());
}

#[test]
fn rejects_receipt_after_commit() {
    let mut ops = valid_trading_ops();
    let receipt = ops.remove(7);
    ops.push(receipt);
    assert!(verify_ir(&trading_ir(ops)).is_err());
}

#[test]
fn rejects_operation_after_commit() {
    let mut ops = valid_trading_ops();
    ops.push(TradingOperation::AbortAtomicTrade);
    assert!(verify_ir(&trading_ir(ops)).is_err());
}

#[test]
fn rejects_missing_all_debts_guard() {
    let mut ops = valid_trading_ops();
    ops.retain(|op| !matches!(op, TradingOperation::AssertAllDebtsClosed));
    assert!(verify_ir(&trading_ir(ops)).is_err());
}

#[test]
fn rejects_multiple_commits() {
    let mut ops = valid_trading_ops();
    ops.push(TradingOperation::CommitAtomicTrade);
    assert!(verify_ir(&trading_ir(ops)).is_err());
}

#[test]
fn accepts_invariant_guard_before_receipt() {
    let mut ops = valid_trading_ops();
    let receipt_index = ops
        .iter()
        .position(|op| matches!(op, TradingOperation::EmitTradeReceipt))
        .expect("fixture must emit a receipt");
    ops.insert(
        receipt_index,
        TradingOperation::AssertInvariant {
            kind: InvariantKind::Solvent,
        },
    );
    assert!(verify_ir(&trading_ir(ops)).is_ok());
}

#[test]
fn rejects_invariant_guard_after_receipt() {
    let mut ops = valid_trading_ops();
    let receipt_index = ops
        .iter()
        .position(|op| matches!(op, TradingOperation::EmitTradeReceipt))
        .expect("fixture must emit a receipt");
    ops.insert(
        receipt_index + 1,
        TradingOperation::AssertInvariant {
            kind: InvariantKind::Solvent,
        },
    );
    assert_eq!(codes(&trading_ir(ops)), vec![DiagnosticCode::UnsafeIr]);
}

#[test]
fn rejects_duplicate_invariant_guard() {
    let mut ops = valid_trading_ops();
    let receipt_index = ops
        .iter()
        .position(|op| matches!(op, TradingOperation::EmitTradeReceipt))
        .expect("fixture must emit a receipt");
    ops.insert(
        receipt_index,
        TradingOperation::AssertInvariant {
            kind: InvariantKind::Solvent,
        },
    );
    ops.insert(
        receipt_index,
        TradingOperation::AssertInvariant {
            kind: InvariantKind::Solvent,
        },
    );
    assert_eq!(codes(&trading_ir(ops)), vec![DiagnosticCode::UnsafeIr]);
}

#[test]
fn rejects_invariant_guard_before_trade_begins() {
    let ops = vec![TradingOperation::AssertInvariant {
        kind: InvariantKind::Solvent,
    }];
    // A lone invariant with no BeginAtomicTrade/CommitAtomicTrade trips more
    // than one structural rule (unstarted trade, no terminal commit/abort);
    // this test only cares that it's rejected, not the exact diagnostic count.
    assert!(verify_ir(&trading_ir(ops)).is_err());
}
