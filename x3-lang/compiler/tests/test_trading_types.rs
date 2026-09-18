//! Trading Core v1 asset registry and economic type tests.

use x3_lang_ast::trading::RoundingMode;
use x3_lang_common::{Symbol, X3Error};
use x3_lang_compiler::parser::parse_source;
use x3_lang_compiler::semantic::CompilationMode;
use x3_lang_compiler::trading_semantic::decimal_to_base_units;
use x3_lang_compiler::{analyze_trading, TradingSymbols};

const VALID_TRADING_SOURCE: &str = include_str!("fixtures/trading_core_v1.x3");

fn analyze(source: &str) -> Result<TradingSymbols, Vec<X3Error>> {
    let program = parse_source(source).expect("test source must parse");
    analyze_trading(&program, CompilationMode::Dev)
}

#[test]
fn canonical_example_registers_assets_and_policy() {
    let symbols = analyze(VALID_TRADING_SOURCE).expect("canonical trading example must type-check");
    assert_eq!(symbols.assets.len(), 3);
    assert!(symbols.assets.contains_key(&Symbol::new("USDC")));
    assert!(symbols.assets.contains_key(&Symbol::new("WETH")));
    assert_eq!(symbols.policies.len(), 1);
    assert!(symbols.policies.contains_key(&Symbol::new("MainnetArb")));
}

#[test]
fn duplicate_asset_declarations_are_rejected() {
    let source = r#"
asset USDC = evm.ethereum.0xA0b8 { decimals: 6 }
asset USDC = evm.ethereum.0xA0b8 { decimals: 6 }
"#;
    let errors = analyze(source).expect_err("duplicate asset names must be rejected");
    assert!(
        errors.iter().any(|e| format!("{e}").contains("duplicate asset")),
        "expected a duplicate-asset diagnostic: {errors:?}"
    );
}

#[test]
fn decimals_above_38_are_rejected() {
    let source = r#"
asset USDC = evm.ethereum.0xA0b8 { decimals: 39 }
"#;
    let errors = analyze(source).expect_err("decimals above 38 must be rejected");
    assert!(
        errors.iter().any(|e| format!("{e}").contains("39 decimals")),
        "expected a decimals diagnostic: {errors:?}"
    );
}

#[test]
fn swap_source_binding_type_mismatch_is_rejected() {
    let source = r#"
asset USDC = evm.ethereum.0xA0b8 { decimals: 6 }
asset WETH = evm.ethereum.0xC02a { decimals: 18 }
risk policy P {
    max_slippage: 30 bps
    max_gas: 0.02 ETH
    max_flash_fee: 10 bps
    deadline: 2 blocks
    require_private_submission: false
}
atomic trade BadBinding using P {
    let weth = swap 1 USDC -> WETH via uniswap_v3 min_out 1 WETH
    let returned = swap weth USDC -> WETH via sushiswap min_out 1 WETH
    require all_debts_repaid
}
"#;
    let errors = analyze(source).expect_err("binding type mismatch must be rejected");
    assert!(
        errors.iter().any(|e| format!("{e}").contains("binding is typed")),
        "expected a binding-type diagnostic: {errors:?}"
    );
}

#[test]
fn min_out_in_input_asset_is_rejected() {
    let source = r#"
asset USDC = evm.ethereum.0xA0b8 { decimals: 6 }
asset WETH = evm.ethereum.0xC02a { decimals: 18 }
risk policy P {
    max_slippage: 30 bps
    max_gas: 0.02 ETH
    max_flash_fee: 10 bps
    deadline: 2 blocks
    require_private_submission: false
}
atomic trade BadMinOut using P {
    let weth = swap 1 USDC -> WETH via uniswap_v3 min_out 1 USDC
    require all_debts_repaid
}
"#;
    let errors = analyze(source).expect_err("min_out in the input asset must be rejected");
    assert!(
        errors.iter().any(|e| format!("{e}").contains("min_out is denominated")),
        "expected a min_out diagnostic: {errors:?}"
    );
}

#[test]
fn profit_in_undeclared_asset_is_rejected() {
    let source = r#"
asset USDC = evm.ethereum.0xA0b8 { decimals: 6 }
risk policy P {
    max_slippage: 30 bps
    max_gas: 0.02 ETH
    max_flash_fee: 10 bps
    deadline: 2 blocks
    require_private_submission: false
}
atomic trade BadProfit using P {
    require net_profit >= 1 DAI
    require all_debts_repaid
}
"#;
    let errors = analyze(source).expect_err("profit in an undeclared asset must be rejected");
    assert!(
        errors.iter().any(|e| format!("{e}").contains("undeclared asset 'DAI'")),
        "expected an undeclared-asset diagnostic: {errors:?}"
    );
}

#[test]
fn decimal_exact_rejects_precision_loss() {
    let decimals = 6u8;
    assert_eq!(
        decimal_to_base_units("1.000000", decimals, RoundingMode::Exact).unwrap(),
        1_000_000
    );
    assert!(matches!(
        decimal_to_base_units("1.0000001", decimals, RoundingMode::Exact),
        Err(x3_lang_compiler::trading_semantic::TradingTypeError::PrecisionLoss)
    ));
    assert_eq!(
        decimal_to_base_units("1.0000001", decimals, RoundingMode::Down).unwrap(),
        1_000_000
    );
    assert_eq!(
        decimal_to_base_units("1.0000001", decimals, RoundingMode::Up).unwrap(),
        1_000_001
    );
}

#[test]
fn decimal_overflow_is_rejected() {
    assert!(matches!(
        decimal_to_base_units("340282366920938463463374607431768211456", 6, RoundingMode::Down),
        Err(x3_lang_compiler::trading_semantic::TradingTypeError::Overflow)
    ));
}

#[test]
fn integer_with_separators_converts_like_decimal_integer() {
    assert_eq!(
        decimal_to_base_units("1_000_000", 6, RoundingMode::Exact).unwrap(),
        1_000_000_000_000u128
    );
    assert_eq!(
        decimal_to_base_units("0.02", 18, RoundingMode::Exact).unwrap(),
        20_000_000_000_000_000u128
    );
}
