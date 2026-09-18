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

#[test]
fn swap_across_declared_chains_is_rejected() {
    // A plain `swap ... via <venue>` is one venue call on one chain — this
    // is not a bridge. USDC_ARB and USDC_BASE are declared on different
    // chains; mixing them in one swap must be rejected, not silently
    // compiled as an ordinary same-chain DEX call.
    let source = r#"
asset USDC_ARB = evm.arbitrum.0xA0b8 { decimals: 6 }
asset USDC_BASE = evm.base.0xB0c9 { decimals: 6 }

risk policy P {
    max_slippage: 30 bps
    max_gas: 100000 USDC_ARB
    max_flash_fee: 10 bps
    deadline: 10 blocks
    require_private_submission: false
}

atomic trade CrossChainMixup using P {
    let out = swap 100 USDC_ARB -> USDC_BASE via uniswap_v3 min_out 90 USDC_BASE
    require net_profit >= 1 USDC_ARB
}
"#;
    let errors = pipeline_errors(source, CompilationMode::Dev);
    assert!(
        has_message(&errors, "mixes chains"),
        "swap across two declared chains must be rejected: {errors:?}"
    );
}

#[test]
fn borrow_on_a_different_chain_than_the_swap_is_rejected() {
    // Chain mismatch isn't only a from/to-in-one-swap problem — an earlier
    // statement can set the trade's chain and a later, unrelated statement
    // can still drift onto a different one.
    let source = r#"
asset USDC_ARB = evm.arbitrum.0xA0b8 { decimals: 6 }
asset WETH_BASE = evm.base.0xC02a { decimals: 18 }

risk policy P {
    max_slippage: 30 bps
    max_gas: 100000 USDC_ARB
    max_flash_fee: 10 bps
    deadline: 10 blocks
    require_private_submission: false
}

atomic trade DriftedChain using P {
    borrow 1000 USDC_ARB from aave_v3 as debt
    let out = swap debt.amount USDC_ARB -> WETH_BASE via uniswap_v3 min_out 1 WETH_BASE
    require net_profit >= 1 USDC_ARB
}
"#;
    let errors = pipeline_errors(source, CompilationMode::Dev);
    assert!(
        has_message(&errors, "mixes chains"),
        "a later statement on a different chain than the trade's first asset must be rejected: {errors:?}"
    );
}

#[test]
fn same_chain_trade_with_several_assets_is_accepted() {
    // Multiple assets on the *same* chain (the ordinary, common case) must
    // not trip the cross-chain check.
    let source = format!(
        r#"{ASSET_HEADER}{POLICY_HEADER}
atomic trade SameChain using P {{
    borrow 1 USDC from aave_v3 as debt
    let out = swap debt.amount USDC -> ETH via uniswap_v3 min_out 1 ETH
    repay debt
    require net_profit >= 1 USDC
    require all_debts_repaid
    emit receipt
}}
"#
    );
    let errors = pipeline_errors(&source, CompilationMode::Dev);
    assert!(
        !has_message(&errors, "mixes chains"),
        "same-chain assets must not be flagged as a chain mismatch: {errors:?}"
    );
}

const BRIDGE_ASSET_HEADER: &str = r#"
asset USDC_ARB = evm.arbitrum.0xA0b8 { decimals: 6 }
asset USDC_BASE = evm.base.0xB0c9 { decimals: 6 }

risk policy P {
    max_slippage: 30 bps
    max_gas: 100000 USDC_ARB
    max_flash_fee: 10 bps
    deadline: 10 blocks
    require_private_submission: false
}
"#;

#[test]
fn bridge_across_declared_chains_is_the_one_accepted_exception() {
    // The exact scenario the "mixes chains" errors above exist to reject
    // for a plain swap must be the one thing a bridge is allowed to do.
    let source = format!(
        r#"{BRIDGE_ASSET_HEADER}
atomic trade CrossChainSettle using P {{
    bridge 100 USDC_ARB -> USDC_BASE via wormhole to "0x1234567890abcdef1234567890abcdef12345678"
    require net_profit >= 1 USDC_BASE
    require all_debts_repaid
    emit receipt
}}
"#
    );
    let errors = pipeline_errors(&source, CompilationMode::Dev);
    assert!(
        !has_message(&errors, "mixes chains"),
        "a bridge crossing declared chains must not be flagged as a chain mismatch: {errors:?}"
    );
}

#[test]
fn bridge_to_the_same_chain_is_rejected() {
    let source = r#"
asset USDC = evm.arbitrum.0xA0b8 { decimals: 6 }
asset WETH = evm.arbitrum.0xC02a { decimals: 18 }

risk policy P {
    max_slippage: 30 bps
    max_gas: 100000 USDC
    max_flash_fee: 10 bps
    deadline: 10 blocks
    require_private_submission: false
}

atomic trade FakeBridge using P {
    bridge 100 USDC -> WETH via wormhole to "0x1234567890abcdef1234567890abcdef12345678"
    require net_profit >= 1 WETH
    require all_debts_repaid
    emit receipt
}
"#;
    let errors = pipeline_errors(source, CompilationMode::Dev);
    assert!(
        has_message(&errors, "must move between two different chains"),
        "a bridge whose source and destination are on the same chain must be rejected: {errors:?}"
    );
}

#[test]
fn statement_after_bridge_is_rejected() {
    let source = format!(
        r#"{BRIDGE_ASSET_HEADER}
atomic trade TradeAfterBridge using P {{
    bridge 100 USDC_ARB -> USDC_BASE via wormhole to "0x1234567890abcdef1234567890abcdef12345678"
    let out = swap 1 USDC_ARB -> USDC_ARB via uniswap_v3 min_out 1 USDC_ARB
    require net_profit >= 1 USDC_BASE
    require all_debts_repaid
    emit receipt
}}
"#
    );
    let errors = pipeline_errors(&source, CompilationMode::Dev);
    assert!(
        has_message(&errors, "after bridging via"),
        "a swap after the trade already bridged to another chain must be rejected: {errors:?}"
    );
}

#[test]
fn bridge_source_must_match_the_trade_chain() {
    // The bridge's own from_asset is still checked against the trade's
    // established chain like any other asset reference — only its
    // to_asset is exempt.
    let source = r#"
asset USDC_ARB = evm.arbitrum.0xA0b8 { decimals: 6 }
asset WETH_ARB = evm.arbitrum.0xC02a { decimals: 18 }
asset USDC_BASE = evm.base.0xB0c9 { decimals: 6 }
asset USDC_OP = evm.optimism.0xD0e1 { decimals: 6 }

risk policy P {
    max_slippage: 30 bps
    max_gas: 100000 USDC_ARB
    max_flash_fee: 10 bps
    deadline: 10 blocks
    require_private_submission: false
}

atomic trade DriftedBridgeSource using P {
    let out = swap 100 USDC_ARB -> WETH_ARB via uniswap_v3 min_out 1 WETH_ARB
    bridge 1 USDC_BASE -> USDC_OP via wormhole to "0x1234567890abcdef1234567890abcdef12345678"
    require net_profit >= 1 USDC_OP
    require all_debts_repaid
    emit receipt
}
"#;
    let errors = pipeline_errors(source, CompilationMode::Dev);
    assert!(
        has_message(&errors, "mixes chains"),
        "a bridge whose from_asset drifts onto a chain other than the trade's own must still be caught: {errors:?}"
    );
}

/// A trade whose declared effects are all produced and whose declared
/// guarantees are all discharged by its body.
const DISCHARGED_TRADE: &str = r#"
atomic trade Discharged using P
    effects [borrow, swap, repay]
    guarantees [debt_closed, min_profit]
{
    borrow 1_000_000 USDC from aave_v3 as debt
    let weth = swap debt.amount USDC -> ETH
        via uniswap_v3
        min_out 1 ETH
    repay debt
    require net_profit >= 1_000 USDC
    require all_debts_repaid
    emit receipt
}
"#;

#[test]
fn declared_effects_and_guarantees_are_discharged_by_the_body() {
    let source = format!("{ASSET_HEADER}{POLICY_HEADER}{DISCHARGED_TRADE}");
    let errors = pipeline_errors(&source, CompilationMode::Dev);
    // Guard against this test passing vacuously: if the clause syntax failed to
    // parse, the only error would be a ParseError and the check below would
    // trivially hold.
    assert!(
        !errors.iter().any(|e| matches!(e, X3Error::ParseError { .. })),
        "the effects/guarantees clauses must parse: {errors:?}"
    );
    assert!(
        !errors.iter().any(|e| {
            let text = format!("{e}");
            text.contains("declares effect") || text.contains("declares guarantee")
        }),
        "a body that produces and discharges everything it declares must compile: {errors:?}"
    );
}

#[test]
fn an_effect_with_no_matching_statement_is_rejected() {
    // The body never bridges, so declaring the bridge effect is a claim the
    // compiler will not let stand.
    let source = format!(
        "{ASSET_HEADER}{POLICY_HEADER}{}",
        DISCHARGED_TRADE.replace("effects [borrow, swap, repay]", "effects [borrow, bridge, repay]")
    );
    let errors = pipeline_errors(&source, CompilationMode::Dev);
    assert!(
        has_message(&errors, "declares effect 'bridge'"),
        "an undeclared-but-claimed effect must be reported: {errors:?}"
    );
}

#[test]
fn a_guarantee_with_no_discharge_is_rejected() {
    // Nothing asserts the solvent invariant, so declaring that guarantee is a
    // promise the body does not keep.
    let source = format!(
        "{ASSET_HEADER}{POLICY_HEADER}{}",
        DISCHARGED_TRADE.replace(
            "guarantees [debt_closed, min_profit]",
            "guarantees [debt_closed, min_profit, solvent]"
        )
    );
    let errors = pipeline_errors(&source, CompilationMode::Dev);
    assert!(
        has_message(&errors, "declares guarantee 'solvent'"),
        "an undischarged guarantee must be reported: {errors:?}"
    );
}

#[test]
fn a_trade_without_effect_or_guarantee_clauses_is_unaffected() {
    // Both clauses are optional; omitting them must not introduce diagnostics.
    let source = format!(
        "{ASSET_HEADER}{POLICY_HEADER}{}",
        DISCHARGED_TRADE
            .replace("    effects [borrow, swap, repay]\n", "")
            .replace("    guarantees [debt_closed, min_profit]\n", "")
    );
    let errors = pipeline_errors(&source, CompilationMode::Dev);
    assert!(
        !errors.iter().any(|e| {
            let text = format!("{e}");
            text.contains("declares effect") || text.contains("declares guarantee")
        }),
        "omitting the optional clauses must be fine: {errors:?}"
    );
}

/// An intent that requires a solver bond, so the requirement can be compared
/// against whatever the program declares.
fn intent_requiring_solver_bond(amount: &str) -> String {
    format!(
        r#"intent bonded_swap {{
    from ethereum.USDC amount 1 receiver 0x1
    to solana.USDC receiver 0x2
    route {{
        swap uniswap ethereum.USDC -> ethereum.ETH amount 1 min_output 1
    }}
    require slippage <= 50
    require solver_bond >= {amount}
    timeout 30s refund ethereum.USDC to sender
    on_fail rollback
}}
"#
    )
}

fn solver_market_declaring(bond: &str) -> String {
    format!("solver_market {{\n    mode competitive\n    min_reputation 95\n    bond {bond} USDC\n}}\n\n")
}

#[test]
fn a_solver_bond_guard_without_a_declaration_is_rejected() {
    // The guard asserts "the solver posted at least N", which is a claim about
    // the program's configuration. With no `bond` anywhere it asserted nothing —
    // in the compiler and in the VM alike.
    let source = format!(
        "{ASSET_HEADER}{POLICY_HEADER}{}",
        intent_requiring_solver_bond("10_000")
    );
    let errors = pipeline_errors(&source, CompilationMode::Dev);
    assert!(
        has_message(&errors, "no `solver_market"),
        "an unbacked solver bond guard must be reported: {errors:?}"
    );
}

#[test]
fn a_solver_bond_guard_above_the_declared_bond_is_rejected() {
    let source = format!(
        "{ASSET_HEADER}{POLICY_HEADER}{}{}",
        solver_market_declaring("1_000"),
        intent_requiring_solver_bond("10_000")
    );
    let errors = pipeline_errors(&source, CompilationMode::Dev);
    assert!(
        has_message(&errors, "declared bond is 1000"),
        "a requirement above the declared bond must be reported: {errors:?}"
    );
}

#[test]
fn a_solver_bond_guard_within_the_declared_bond_is_accepted() {
    // Non-vacuous: the check must not reject a backed requirement.
    let source = format!(
        "{ASSET_HEADER}{POLICY_HEADER}{}{}",
        solver_market_declaring("10_000"),
        intent_requiring_solver_bond("10_000")
    );
    let errors = pipeline_errors(&source, CompilationMode::Dev);
    assert!(
        !errors.iter().any(|error| format!("{error}").contains("solver bond")),
        "a backed solver bond requirement must compile: {errors:?}"
    );
}

#[test]
fn ast_level_checks_run_on_the_build_path_not_only_the_check_path() {
    // `compile_source` is what `x3c build` calls, and it used to skip every
    // AST-level check — the same "a check that is not on the path that matters"
    // shape as the rest of this compiler. The unbacked bond above must be
    // refused there too, not only by `check_source`.
    let source = format!(
        "{ASSET_HEADER}{POLICY_HEADER}{}",
        intent_requiring_solver_bond("10_000")
    );
    let result = x3_lang_compiler::compile_source(&source);
    assert!(
        result.is_err(),
        "the build path must refuse an unbacked solver bond guard, got {:?}",
        result.map(|bytecode| bytecode.len())
    );
}
