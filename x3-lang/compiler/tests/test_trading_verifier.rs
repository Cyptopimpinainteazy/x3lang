//! Trading Core v1 debt closure, guard, and risk-policy verification tests.

use x3_lang_common::X3Error;
use x3_lang_compiler::semantic::CompilationMode;
use x3_lang_compiler::trading_verify::DebtFlowState;
use x3_lang_compiler::{check_source_with_mode, verify_atomic_trade, verify_trading_program, TradingSymbols};

const ASSET_HEADER: &str = r#"
asset USDC = evm.ethereum.0xA0b8 { decimals: 6 }
asset ETH = evm.ethereum.0x0000000000000000000000000000000000000000 { decimals: 18 }
"#;

const POLICY_HEADER: &str = r#"
risk policy P {
    max_slippage: 30 bps
    max_gas: 0.02 ETH
    max_flash_fee: 10 bps
    deadline: 2 blocks
    require_private_submission: false
}
"#;

fn pipeline_errors(source: &str, mode: CompilationMode) -> Vec<X3Error> {
    match check_source_with_mode(source, mode) {
        Ok((_, _, errors)) => errors,
        Err(error) => vec![error],
    }
}

fn has_message(errors: &[X3Error], needle: &str) -> bool {
    errors.iter().any(|error| format!("{error}").contains(needle))
}

#[test]
fn debt_flow_state_starts_empty() {
    let state = DebtFlowState::default();
    assert!(state.open.is_empty());
    assert!(state.closed.is_empty());
}

#[test]
fn missing_repayment_is_rejected() {
    let source = format!(
        r#"{ASSET_HEADER}{POLICY_HEADER}
atomic trade MissingRepayment using P {{
    borrow 1 USDC from aave_v3 as debt
    require net_profit >= 1 USDC
    require all_debts_repaid
    emit receipt
}}
"#
    );
    let errors = pipeline_errors(&source, CompilationMode::Dev);
    assert!(
        has_message(&errors, "unrepaid debt"),
        "missing repayment must fail verification: {errors:?}"
    );
}

#[test]
fn duplicate_repayment_is_rejected() {
    let source = format!(
        r#"{ASSET_HEADER}{POLICY_HEADER}
atomic trade DoubleRepayment using P {{
    borrow 1 USDC from aave_v3 as debt
    repay debt
    repay debt
    require net_profit >= 1 USDC
    require all_debts_repaid
    emit receipt
}}
"#
    );
    let errors = pipeline_errors(&source, CompilationMode::Dev);
    assert!(
        has_message(&errors, "more than once"),
        "duplicate repayment must fail verification: {errors:?}"
    );
}

#[test]
fn missing_net_profit_guard_is_rejected() {
    let source = format!(
        r#"{ASSET_HEADER}{POLICY_HEADER}
atomic trade NoProfit using P {{
    borrow 1 USDC from aave_v3 as debt
    repay debt
    require all_debts_repaid
    emit receipt
}}
"#
    );
    let errors = pipeline_errors(&source, CompilationMode::Dev);
    assert!(
        has_message(&errors, "minimum net-profit"),
        "missing net-profit guard must fail verification: {errors:?}"
    );
}

#[test]
fn missing_all_debts_guard_is_rejected() {
    let source = format!(
        r#"{ASSET_HEADER}{POLICY_HEADER}
atomic trade NoAllDebts using P {{
    borrow 1 USDC from aave_v3 as debt
    repay debt
    require net_profit >= 1 USDC
    emit receipt
}}
"#
    );
    let errors = pipeline_errors(&source, CompilationMode::Dev);
    assert!(
        has_message(&errors, "all_debts_repaid"),
        "missing all-debts guard must fail verification: {errors:?}"
    );
}

#[test]
fn missing_receipt_for_borrowed_capital_is_rejected() {
    let source = format!(
        r#"{ASSET_HEADER}{POLICY_HEADER}
atomic trade NoReceipt using P {{
    borrow 1 USDC from aave_v3 as debt
    repay debt
    require net_profit >= 1 USDC
    require all_debts_repaid
}}
"#
    );
    let errors = pipeline_errors(&source, CompilationMode::Dev);
    assert!(
        has_message(&errors, "never emits a receipt"),
        "missing receipt must fail verification: {errors:?}"
    );
}

#[test]
fn mainnet_rejects_unattested_private_submission_policy() {
    let source = format!(
        r#"{ASSET_HEADER}
risk policy PrivateP {{
    max_slippage: 30 bps
    max_gas: 0.02 ETH
    max_flash_fee: 10 bps
    deadline: 2 blocks
    require_private_submission: true
}}
atomic trade PrivateTrade using PrivateP {{
    require net_profit >= 1 USDC
    require all_debts_repaid
    emit receipt
}}
"#
    );
    let errors = pipeline_errors(&source, CompilationMode::Mainnet);
    assert!(
        has_message(&errors, "private-submission"),
        "mainnet must reject unattested private submission: {errors:?}"
    );
}

#[test]
fn unknown_debt_repayment_is_rejected() {
    let source = format!(
        r#"{ASSET_HEADER}{POLICY_HEADER}
atomic trade UnknownDebt using P {{
    repay ghost
    require all_debts_repaid
    emit receipt
}}
"#
    );
    let errors = pipeline_errors(&source, CompilationMode::Dev);
    assert!(
        errors
            .iter()
            .any(|e| format!("{e}").contains("undeclared debt") || format!("{e}").contains("unknown")),
        "unknown debt repayment must be rejected: {errors:?}"
    );
}

#[test]
fn verifier_api_accepts_well_formed_borrowed_trade_when_called_directly() {
    let source = include_str!("fixtures/trading_core_v1.x3");
    let program = x3_lang_compiler::parser::parse_source(source).expect("fixture must parse");
    let symbols: TradingSymbols =
        x3_lang_compiler::analyze_trading(&program, CompilationMode::Dev).expect("fixture must type-check");
    let trade = program
        .items
        .iter()
        .find_map(|item| match &item.node {
            x3_lang_ast::Item::AtomicTrade(trade) => Some(trade),
            _ => None,
        })
        .expect("fixture must contain an atomic trade");
    assert!(verify_atomic_trade(trade, &symbols, CompilationMode::Dev).is_empty());
    assert!(verify_trading_program(&program, &symbols, CompilationMode::Dev).is_empty());
}
