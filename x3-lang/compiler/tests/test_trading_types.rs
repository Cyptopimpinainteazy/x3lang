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
fn checked_constant_amounts_lower_to_exact_base_units() {
    let source = VALID_TRADING_SOURCE
        .replace("borrow 1_000_000 USDC", "borrow (999999.5 + 0.5) USDC")
        .replace("max_gas: 0.02 ETH", "max_gas: (0.03 - 0.01) ETH");
    let expected = x3_lang_compiler::compile_to_ir(&parse_source(VALID_TRADING_SOURCE).unwrap()).unwrap();
    let actual = x3_lang_compiler::compile_to_ir(&parse_source(&source).unwrap())
        .expect("exact same-asset arithmetic must lower");
    let trading = |operations: Vec<x3_lang_compiler::ir::Operation>| {
        operations
            .into_iter()
            .filter_map(|op| match op {
                x3_lang_compiler::ir::Operation::Trading(trade) => Some(trade),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    assert!(!trading(expected.operations.clone()).is_empty());
    assert_eq!(trading(actual.operations), trading(expected.operations));
}

#[test]
fn checked_constant_amounts_reject_underflow() {
    let source = VALID_TRADING_SOURCE.replace("max_gas: 0.02 ETH", "max_gas: (0.01 - 0.03) ETH");
    let errors = analyze(&source).expect_err("negative unsigned asset amount must fail analysis");
    assert!(errors.iter().any(|e| e.to_string().contains("underflow")));
}

#[test]
fn checked_constant_amounts_reject_overflow() {
    let source = VALID_TRADING_SOURCE.replace(
        "borrow 1_000_000 USDC",
        "borrow (340282366920938463463374607431768.211455 + 0.000001) USDC",
    );
    let errors = analyze(&source).expect_err("overflowing amount must fail analysis");
    assert!(errors.iter().any(|e| e.to_string().contains("overflow")));
}

#[test]
fn checked_typed_amount_arithmetic_preserves_asset_and_rejects_mismatches() {
    use x3_lang_compiler::{trading_semantic::TradingTypeError, TypedAmount};
    let symbols = analyze(VALID_TRADING_SOURCE).unwrap();
    let usdc = symbols
        .assets
        .get(&x3_lang_common::Symbol::new("USDC"))
        .unwrap()
        .clone();
    let weth = symbols
        .assets
        .get(&x3_lang_common::Symbol::new("WETH"))
        .unwrap()
        .clone();
    let amount = TypedAmount {
        base_units: 100,
        asset: usdc.clone(),
    };
    let same = TypedAmount {
        base_units: 50,
        asset: usdc.clone(),
    };
    let different = TypedAmount {
        base_units: 50,
        asset: weth,
    };
    let mut other_chain = same.clone();
    other_chain.asset.chain = x3_lang_ast::ast::ChainRef(x3_lang_common::Symbol::new("base"));
    assert_eq!(
        amount.checked_add(&same).unwrap(),
        TypedAmount {
            base_units: 150,
            asset: usdc.clone()
        }
    );
    assert_eq!(amount.checked_sub(&same).unwrap(), same);
    assert_eq!(amount.checked_add(&different), Err(TradingTypeError::AssetMismatch));
    assert_eq!(amount.checked_sub(&other_chain), Err(TradingTypeError::AssetMismatch));
    assert_eq!(same.checked_sub(&amount), Err(TradingTypeError::Underflow));
    assert_eq!(
        TypedAmount {
            base_units: u128::MAX,
            asset: usdc
        }
        .checked_add(&same),
        Err(TradingTypeError::Overflow)
    );
}

#[test]
fn constant_swap_input_uses_exact_asset_arithmetic() {
    let source = VALID_TRADING_SOURCE.replace("swap debt.amount USDC", "swap (999999.75 + 0.25) USDC");
    let ir = x3_lang_compiler::compile_to_ir(&parse_source(&source).unwrap()).unwrap();
    assert!(ir.operations.iter().any(|op| matches!(
        op,
        x3_lang_compiler::ir::Operation::Trading(
            x3_lang_compiler::ir::TradingOperation::ExecuteSwap {
                input: x3_lang_compiler::ir::ValueRef::Literal(1_000_000_000_000), from, ..
            }
        ) if from.symbol == "USDC"
    )));
}

#[test]
fn dynamic_arithmetic_cannot_disguise_a_debt_amount_as_another_asset() {
    let source = VALID_TRADING_SOURCE.replace("swap debt.amount USDC", "swap (debt.amount + 1) WETH");
    let errors = analyze(&source).expect_err("dynamic arithmetic is not a typed constant");
    assert!(errors.iter().any(|e| e.to_string().contains("constant numeric amount")));
}

#[test]
fn checked_amount_arithmetic_refuses_precision_loss_and_unsupported_operators() {
    for expression in ["(0.0000001 + 1)", "(1 / 2)"] {
        let source = VALID_TRADING_SOURCE.replace("borrow 1_000_000 USDC", &format!("borrow {expression} USDC"));
        assert!(analyze(&source).is_err());
    }
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
