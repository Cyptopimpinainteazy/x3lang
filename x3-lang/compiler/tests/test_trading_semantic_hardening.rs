//! Regression tests for compiler-mode propagation and trading name/type resolution.

use x3_lang_common::X3Error;
use x3_lang_compiler::parser::parse_source;
use x3_lang_compiler::{analyze_trading, compile_with_mode, CompilationMode, TradingSymbols};

const VALID_TRADING_SOURCE: &str = include_str!("fixtures/trading_core_v1.x3");

const ASSETS: &str = r#"
asset USDC = evm.ethereum.0xA0b8 { decimals: 6 }
asset WETH = evm.ethereum.0xC02a { decimals: 18 }
asset ETH = evm.ethereum.0x0000000000000000000000000000000000000000 { decimals: 18 }
"#;

const POLICY: &str = r#"
risk policy P {
    max_slippage: 30 bps
    max_gas: 0.02 ETH
    max_flash_fee: 10 bps
    deadline: 2 blocks
    require_private_submission: false
}
"#;

fn analyze(source: &str) -> Result<TradingSymbols, Vec<X3Error>> {
    let program = parse_source(source).expect("test source must parse");
    analyze_trading(&program, CompilationMode::Dev)
}

fn source_with_trade(body: &str) -> String {
    format!("{ASSETS}{POLICY}\natomic trade Test using P {{\n{body}\n}}\n")
}

fn has_message(errors: &[X3Error], needle: &str) -> bool {
    errors.iter().any(|error| error.to_string().contains(needle))
}

/// Defect: `compile_with_mode` passes Mainnet only to the legacy IR verifier
/// while trading analysis and verification are run in Dev mode during lowering.
#[test]
fn compile_with_mode_mainnet_rejects_missing_private_submission_capability() {
    let error = compile_with_mode(VALID_TRADING_SOURCE, CompilationMode::Mainnet)
        .expect_err("mainnet compilation must reject an unattested private-submission requirement");

    assert!(
        error.to_string().contains("private-submission"),
        "the production capability rejection must come from mode-aware trading verification: {error:?}"
    );
}

/// Defect: a borrowed debt is not entered into the type environment until its
/// `repay`, so `debt.amount` before repayment silently escapes asset checking.
#[test]
fn debt_amount_before_repay_must_match_the_borrowed_asset() {
    let source = source_with_trade(
        r#"    borrow 1 USDC from aave_v3 as debt
    let returned = swap debt.amount WETH -> USDC via sushiswap min_out 1 USDC
    repay debt
    require net_profit >= 1 USDC
    require all_debts_repaid
    emit receipt"#,
    );

    let errors = analyze(&source).expect_err("debt.amount cannot be annotated as a different asset");
    assert!(
        has_message(&errors, "debt") && has_message(&errors, "USDC") && has_message(&errors, "WETH"),
        "the diagnostic must identify the debt and conflicting assets: {errors:?}"
    );
}

/// Defect: an unresolved identifier produces `None` from name resolution and
/// is then accepted as though there were no type information to check.
#[test]
fn unknown_swap_binding_fails_closed() {
    let source = source_with_trade(
        r#"    let returned = swap missing USDC -> WETH via uniswap_v3 min_out 1 WETH
    require net_profit >= 1 USDC
    require all_debts_repaid
    emit receipt"#,
    );

    let errors = analyze(&source).expect_err("an unknown swap binding must be rejected");
    assert!(
        has_message(&errors, "missing"),
        "the diagnostic must identify the unresolved binding: {errors:?}"
    );
}

/// Defect: field access on a debt is accepted for any field name even though
/// Trading Core v1 exposes only `debt.amount` as an input value.
#[test]
fn invalid_debt_field_fails_closed() {
    let source = source_with_trade(
        r#"    borrow 1 USDC from aave_v3 as debt
    let weth = swap debt.principal USDC -> WETH via uniswap_v3 min_out 1 WETH
    repay debt
    require net_profit >= 1 USDC
    require all_debts_repaid
    emit receipt"#,
    );

    let errors = analyze(&source).expect_err("an unsupported debt field must be rejected");
    assert!(
        has_message(&errors, "principal"),
        "the diagnostic must identify the unsupported debt field: {errors:?}"
    );
}

/// Defect: policy amounts are resolved during the declaration-collection pass,
/// so a valid asset declared later is incorrectly reported as undeclared.
#[test]
fn policy_asset_resolution_is_declaration_order_independent() {
    let source = r#"
risk policy P {
    max_slippage: 30 bps
    max_gas: 0.02 ETH
    max_flash_fee: 10 bps
    deadline: 2 blocks
    require_private_submission: false
}
asset ETH = evm.ethereum.0x0000000000000000000000000000000000000000 { decimals: 18 }
"#;

    let symbols = analyze(source).expect("policy assets may be declared later in the source");
    assert!(symbols.policies.contains_key(&x3_lang_common::Symbol::new("P")));
}

/// Defect: trading semantic diagnostics discard parser source locations and
/// expose `Span::DUMMY`, even for errors tied to a concrete declaration.
#[test]
fn trading_semantic_diagnostic_preserves_source_span() {
    let source = "asset USDC = evm.ethereum.0xA0b8 { decimals: 6 }\n\
                  asset USDC = evm.ethereum.0xA0b8 { decimals: 6 }\n";
    let errors = analyze(source).expect_err("the duplicate declaration must be rejected");
    let duplicate = errors
        .iter()
        .find(|error| error.to_string().contains("duplicate asset"))
        .expect("a duplicate-asset diagnostic must be present");
    let span = duplicate.span().expect("semantic diagnostics must carry a source span");

    assert!(!span.is_dummy(), "the diagnostic must not use Span::DUMMY: {duplicate:?}");
    let highlighted = source
        .get(span.to_range())
        .expect("the diagnostic span must be within the original source");
    assert!(
        highlighted.contains("USDC"),
        "the diagnostic span must highlight the conflicting declaration: {highlighted:?}"
    );
}
