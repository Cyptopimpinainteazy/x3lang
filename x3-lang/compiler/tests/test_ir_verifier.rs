use std::collections::{BTreeSet, HashMap};

use x3_lang_compiler::diagnostic::DiagnosticCode;
use x3_lang_compiler::ir::{
    AssetKey, CompiledTradingPolicy, CostKind, FailureAction, InvariantKind, Operation, ProgramMetadata,
    StateBindingMode, SubmissionProfile, TradingOperation, ValueRef, X3IR,
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
fn a_loop_is_refused_because_the_vm_cannot_execute_one() {
    // This test used to assert the zero-iteration check. That check is gone with
    // the arm it lived in: a `Loop` is refused outright now, so its iteration
    // count is not a fact anything reads. The refusal is what the test asserts,
    // message included, because "some UnsafeIr" would also be satisfied by the
    // nested-op checks this replaced.
    let ir = ir_with(vec![Operation::Loop {
        max_iterations: 0,
        body: vec![Operation::Nop],
    }]);
    let diagnostics = verify_ir(&ir).expect_err("a loop has no target this VM could jump back to");
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert!(
        diagnostics[0].message.contains("`loop` cannot be executed") && diagnostics[0].message.contains("padded"),
        "the refusal must say what the VM branches on and what the stream is: {diagnostics:?}"
    );
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
    // The nested-walk coverage moves from `If` (refused outright, so nothing
    // inside it is ever walked) to a construct that is emitted: a scheduled
    // dispatch whose entry carries an unsafe operation. The defect is two levels
    // down, so a walk that stopped at the top level would miss it.
    let ir = ir_with(vec![Operation::ScheduledDispatch {
        period_blocks: 0,
        entry: vec![Operation::Simulate {
            body: vec![Operation::MultisigCheck { required: 3, total: 2 }],
            receipt_slot: "evidence".to_owned(),
        }],
    }]);

    assert_eq!(codes(&ir), vec![DiagnosticCode::UnsafeIr, DiagnosticCode::UnsafeIr]);
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
        max_gas_asset: asset("USDC"),
        max_flash_fee_bps: 10,
        deadline_blocks: 10,
        require_private_submission: false,
        minimum_net_profit: None,
        minimum_net_profit_asset: None,
        quote_freshness_blocks: Some(10),
        submission_profile: SubmissionProfile::Public,
        state_binding: StateBindingMode::Exact,
        allowed_cost_kinds: BTreeSet::from([
            CostKind::Gas,
            CostKind::LiquidityFee,
            CostKind::FlashLiquidityFee,
            CostKind::Slippage,
            CostKind::PriceImpact,
            CostKind::MevLeakage,
        ]),
        allow_mint: false,
        allow_burn: false,
        max_oracle_deviation_bps: None,
        max_cumulative_loss: None,
        max_cumulative_loss_asset: None,
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

fn valid_bridge_ops() -> Vec<TradingOperation> {
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
        TradingOperation::CloseDebt {
            debt_id: "debt".to_owned(),
        },
        TradingOperation::Bridge {
            via: "wormhole".to_owned(),
            from: asset("USDC"),
            to: asset("USDC_BASE"),
            input: ValueRef::Literal(1_000_000),
            receiver: "0x1234567890abcdef1234567890abcdef12345678".to_owned(),
        },
        TradingOperation::AssertMinNetProfit {
            settlement_asset: asset("USDC_BASE"),
            minimum: 1,
        },
        TradingOperation::AssertAllDebtsClosed,
        TradingOperation::EmitTradeReceipt,
        TradingOperation::CommitAtomicTrade,
    ]
}

#[test]
fn accepts_statefully_valid_bridge_sequence() {
    assert!(verify_ir(&trading_ir(valid_bridge_ops())).is_ok());
}

#[test]
fn rejects_open_debt_after_bridge() {
    let mut ops = valid_bridge_ops();
    let bridge_index = ops
        .iter()
        .position(|op| matches!(op, TradingOperation::Bridge { .. }))
        .expect("bridge op must be present");
    ops.insert(
        bridge_index + 1,
        TradingOperation::OpenDebt {
            debt_id: "debt2".to_owned(),
            provider: "aave_v3".to_owned(),
            asset: asset("USDC"),
            principal: 1,
        },
    );
    assert!(verify_ir(&trading_ir(ops)).is_err());
}

#[test]
fn rejects_swap_after_bridge() {
    let mut ops = valid_bridge_ops();
    let bridge_index = ops
        .iter()
        .position(|op| matches!(op, TradingOperation::Bridge { .. }))
        .expect("bridge op must be present");
    ops.insert(
        bridge_index + 1,
        TradingOperation::ExecuteSwap {
            binding: "late".to_owned(),
            venue: "uniswap_v3".to_owned(),
            from: asset("USDC"),
            to: asset("WETH"),
            input: ValueRef::Literal(1),
            min_output: 1,
        },
    );
    assert!(verify_ir(&trading_ir(ops)).is_err());
}

#[test]
fn rejects_close_debt_after_bridge() {
    let mut ops = valid_bridge_ops();
    let bridge_index = ops
        .iter()
        .position(|op| matches!(op, TradingOperation::Bridge { .. }))
        .expect("bridge op must be present");
    ops.insert(
        bridge_index + 1,
        TradingOperation::CloseDebt {
            debt_id: "debt".to_owned(),
        },
    );
    assert!(verify_ir(&trading_ir(ops)).is_err());
}

#[test]
fn rejects_duplicate_bridge() {
    let mut ops = valid_bridge_ops();
    let bridge_index = ops
        .iter()
        .position(|op| matches!(op, TradingOperation::Bridge { .. }))
        .expect("bridge op must be present");
    let bridge = ops[bridge_index].clone();
    ops.insert(bridge_index + 1, bridge);
    assert!(verify_ir(&trading_ir(ops)).is_err());
}

#[test]
fn rejects_bridge_with_empty_via() {
    let mut ops = valid_bridge_ops();
    if let Some(TradingOperation::Bridge { via, .. }) =
        ops.iter_mut().find(|op| matches!(op, TradingOperation::Bridge { .. }))
    {
        via.clear();
    }
    assert!(verify_ir(&trading_ir(ops)).is_err());
}

#[test]
fn rejects_bridge_with_empty_receiver() {
    let mut ops = valid_bridge_ops();
    if let Some(TradingOperation::Bridge { receiver, .. }) =
        ops.iter_mut().find(|op| matches!(op, TradingOperation::Bridge { .. }))
    {
        receiver.clear();
    }
    assert!(verify_ir(&trading_ir(ops)).is_err());
}

#[test]
fn a_hedge_operation_is_refused_as_unexecutable() {
    // PHASE 9's exposure is decided on the AST (`hedge::verify`), and this layer says
    // what the *VM* can do with the result: nothing, because a perp leg needs a venue
    // adapter it does not have. The refusal is the feature's honest end, and it is
    // here rather than only in the emitter so `check` and `build` agree.
    let ir = ir_with(vec![Operation::Hedge {
        asset: "ethereum.ETH".to_owned(),
        long: 1_000,
        short: 1_000,
        delta_bps: 0,
        delta_bound_bps: Some(1),
    }]);
    let diagnostics = verify_ir(&ir).expect_err("a hedge cannot be executed here");
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert!(
        diagnostics[0].message.contains("cannot be executed")
            && diagnostics[0].message.contains("venue adapter")
            && !diagnostics[0].message.contains("  "),
        "the refusal must name the missing venue, with no spacing artefacts: {diagnostics:?}"
    );
}

#[test]
fn a_liquidation_operation_is_refused_as_unexecutable() {
    // PHASE 10's accounting is decided on the AST (`liquidation::verify`); this layer
    // says what the VM can do with it: nothing, because `liquidate` and `receive` are
    // calls into a lending protocol it has no adapter for. Refusing here (and not
    // only in the emitter) keeps `check` and `build` in agreement.
    let ir = ir_with(vec![Operation::Liquidation {
        position: "borrower.position".to_owned(),
        debt_asset: "ethereum.USDC".to_owned(),
        collateral_asset: "ethereum.ETH".to_owned(),
        capital: 1_000,
        collateral: 1_200,
        min_output: 1_100,
        repaid: 1_000,
        profit_floor: Some(100),
    }]);
    let diagnostics = verify_ir(&ir).expect_err("a liquidation cannot be executed here");
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert!(
        diagnostics[0].message.contains("cannot be executed")
            && diagnostics[0].message.contains("lending protocol")
            && diagnostics[0].message.contains("borrower.position")
            && !diagnostics[0].message.contains("  "),
        "the refusal must name the position and the missing adapter, with no spacing artefacts: {diagnostics:?}"
    );
}

#[test]
fn a_rebalance_operation_is_refused_until_the_graph_can_be_generated() {
    // PHASE 11's weights are decided on the AST (`rebalance::verify`); what is missing
    // is the plan that reaches them, and the phase says that part is eventual. A record
    // with no legs would be a plan that does nothing, so this layer refuses it.
    let ir = ir_with(vec![Operation::Rebalance {
        name: "portfolio".to_owned(),
        weights: vec![("unknown.BTC".to_owned(), 40), ("unknown.ETH".to_owned(), 60)],
        criterion: "fees".to_owned(),
    }]);
    let diagnostics = verify_ir(&ir).expect_err("no graph can be generated yet");
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert!(
        diagnostics[0].message.contains("cannot be executed")
            && diagnostics[0].message.contains("generate the transaction graph")
            && diagnostics[0].message.contains("portfolio")
            && !diagnostics[0].message.contains("  "),
        "the refusal must name the rebalance and what is missing: {diagnostics:?}"
    );
}

#[test]
fn a_netting_book_is_refused_until_something_can_settle_the_residual() {
    // PHASE 22's offsets are decided on the AST (`netting::book`); what is missing is
    // anything that can move the residual. A party in a book is a name rather than an
    // account, so there is no balance to debit, and an artifact carrying a residual
    // nothing settles would be the fake this repository forbids.
    let ir = ir_with(vec![Operation::Netting {
        book: "book_a".to_owned(),
        transfers: vec![("alice".to_owned(), "bob".to_owned(), "ethereum.USDC".to_owned(), 200)],
    }]);
    let diagnostics = verify_ir(&ir).expect_err("nothing settles the residual yet");
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert!(
        diagnostics[0].message.contains("cannot be executed")
            && diagnostics[0].message.contains("book_a")
            && diagnostics[0].message.contains("1 transfer(s)")
            && diagnostics[0].message.contains("account")
            && !diagnostics[0].message.contains("  "),
        "the refusal must name the book, the residual and what is missing: {diagnostics:?}"
    );
}
