//! Parser and formatter tests for Trading Core v1 declarations.
//!
//! Covers the complete design-spec example, structural negative cases
//! (missing `via`, missing `min_out`, duplicate debt names, malformed
//! basis points), and formatter round-trip equivalence via serde values.

use x3_lang_ast::ast::{Expression, Item, LiteralExpr, Program};
use x3_lang_ast::{AmountExpr, AssetDecl, AtomicTradeDecl, DebtId, TradeRiskPolicy, TradeStmt};
use x3_lang_common::Symbol;
use x3_lang_compiler::formatter::X3Formatter;
use x3_lang_compiler::parser::parse_source;

pub const TRADING_CORE_V1_SOURCE: &str = include_str!("fixtures/trading_core_v1.x3");
pub const TRADING_INVALID_MISSING_MIN_OUT_SOURCE: &str = include_str!("fixtures/trading_invalid_missing_min_out.x3");

fn find_items(program: &Program) -> (Vec<&AssetDecl>, Option<&TradeRiskPolicy>, Option<&AtomicTradeDecl>) {
    let mut assets = Vec::new();
    let mut policy = None;
    let mut trade = None;
    for item in &program.items {
        match &item.node {
            Item::AssetDecl(decl) => assets.push(decl),
            Item::TradeRiskPolicy(p) => policy = Some(p),
            Item::AtomicTrade(t) => trade = Some(t),
            _ => {}
        }
    }
    (assets, policy, trade)
}

#[test]
fn parses_complete_trading_core_v1_example() {
    let program =
        parse_source(TRADING_CORE_V1_SOURCE).unwrap_or_else(|e| panic!("canonical trading fixture must parse: {e}"));

    let (assets, policy, trade) = find_items(&program);
    assert_eq!(assets.len(), 3, "expected USDC, WETH, and ETH AssetDecl items");
    assert_eq!(program.items.len(), 5, "asset, asset, asset, policy, trade");

    let usdc = assets[0];
    assert_eq!(usdc.name.as_str(), "USDC");
    assert_eq!(usdc.asset.vm_family.as_str(), "evm");
    assert_eq!(usdc.asset.chain.as_str(), "ethereum");
    assert_eq!(usdc.asset.canonical_id.as_str(), "0xA0b8");
    assert_eq!(usdc.asset.symbol.as_str(), "USDC");
    assert_eq!(usdc.asset.decimals, 6);

    let weth = assets[1];
    assert_eq!(weth.name.as_str(), "WETH");
    assert_eq!(weth.asset.chain.as_str(), "ethereum");
    assert_eq!(weth.asset.canonical_id.as_str(), "0xC02a");
    assert_eq!(weth.asset.symbol.as_str(), "WETH");
    assert_eq!(weth.asset.decimals, 18);

    let eth = assets[2];
    assert_eq!(eth.name.as_str(), "ETH");
    assert_eq!(eth.asset.chain.as_str(), "ethereum");
    assert_eq!(eth.asset.decimals, 18);

    let policy = policy.expect("canonical fixture must contain one TradeRiskPolicy");
    assert_eq!(policy.name.as_str(), "MainnetArb");
    assert_eq!(policy.max_slippage_bps, 30);
    assert_eq!(policy.max_gas.asset.as_str(), "ETH");
    match &policy.max_gas.value {
        Expression::Literal(LiteralExpr::Float { raw, .. }) => assert_eq!(raw.as_str(), "0.02"),
        other => panic!("max_gas must preserve decimal literal, got {other:?}"),
    }
    assert_eq!(policy.max_flash_fee_bps, 10);
    match &policy.deadline {
        Expression::Literal(LiteralExpr::Int { value, .. }) => assert_eq!(*value, 2),
        other => panic!("deadline must be the block-count literal, got {other:?}"),
    }
    assert!(policy.require_private_submission);
    assert!(
        policy.min_profit.is_none(),
        "policy has no min_profit in canonical example"
    );

    let trade = trade.expect("canonical fixture must contain one AtomicTradeDecl");
    assert_eq!(trade.name.as_str(), "CrossDexArb");
    assert_eq!(trade.risk_policy.as_str(), "MainnetArb");
    assert_eq!(trade.body.len(), 7);

    match &trade.body[0] {
        TradeStmt::Borrow { amount, provider, debt } => {
            assert_eq!(amount.asset.as_str(), "USDC");
            assert!(matches!(
                &amount.value,
                Expression::Literal(LiteralExpr::Int { value: 1_000_000, .. })
            ));
            assert_eq!(provider.as_str(), "aave_v3");
            assert_eq!(debt.0.as_str(), "debt");
        }
        other => panic!("first statement must be Borrow, got {other:?}"),
    }

    match &trade.body[1] {
        TradeStmt::Swap {
            binding,
            input,
            from_asset,
            to_asset,
            venue,
            min_output,
        } => {
            assert_eq!(binding.as_str(), "weth");
            assert!(matches!(&input.value, Expression::FieldAccess { field, .. } if field.as_str() == "amount"));
            assert_eq!(input.asset.as_str(), "USDC");
            assert_eq!(from_asset.as_str(), "USDC");
            assert_eq!(to_asset.as_str(), "WETH");
            assert_eq!(venue.as_str(), "uniswap_v3");
            assert_eq!(min_output.asset.as_str(), "WETH");
            assert!(matches!(
                &min_output.value,
                Expression::Literal(LiteralExpr::Int { value: 410, .. })
            ));
        }
        other => panic!("second statement must be Swap, got {other:?}"),
    }

    match &trade.body[2] {
        TradeStmt::Swap {
            binding,
            input,
            from_asset,
            to_asset,
            venue,
            min_output,
        } => {
            assert_eq!(binding.as_str(), "returned");
            assert!(matches!(&input.value, Expression::Ident(name) if name.as_str() == "weth"));
            assert_eq!(input.asset.as_str(), "WETH");
            assert_eq!(from_asset.as_str(), "WETH");
            assert_eq!(to_asset.as_str(), "USDC");
            assert_eq!(venue.as_str(), "sushiswap");
            assert_eq!(min_output.asset.as_str(), "USDC");
            assert!(matches!(
                &min_output.value,
                Expression::Literal(LiteralExpr::Int { value: 1_002_000, .. })
            ));
        }
        other => panic!("third statement must be Swap, got {other:?}"),
    }

    assert!(matches!(&trade.body[3], TradeStmt::Repay { debt } if debt.0.as_str() == "debt"));
    match &trade.body[4] {
        TradeStmt::RequireMinNetProfit { amount } => {
            assert_eq!(amount.asset.as_str(), "USDC");
            assert!(matches!(
                &amount.value,
                Expression::Literal(LiteralExpr::Int { value: 1_000, .. })
            ));
        }
        other => panic!("fifth statement must be RequireMinNetProfit, got {other:?}"),
    }
    assert!(matches!(trade.body[5], TradeStmt::RequireAllDebtsRepaid));
    assert!(matches!(trade.body[6], TradeStmt::EmitReceipt));
}

#[test]
fn asset_quote_symbol_equals_decl_name() {
    let program = parse_source(TRADING_CORE_V1_SOURCE).expect("canonical fixture must parse");
    let (assets, _, _) = find_items(&program);
    for decl in assets {
        assert_eq!(decl.name.as_str(), decl.asset.symbol.as_str());
    }
}

#[test]
fn rejects_swap_without_via() {
    let source = r#"
asset USDC = evm.ethereum.0xA0b8 { decimals: 6 }
asset WETH = evm.ethereum.0xC02a { decimals: 18 }
atomic trade NoVia using MainnetArb {
    let out = swap 100 USDC -> WETH min_out 90 WETH
}
"#;
    let err = parse_source(source).expect_err("swap without `via` must be a parser diagnostic");
    assert!(
        format!("{err}").to_lowercase().contains("via"),
        "diagnostic should name the missing `via`, got: {err}"
    );
}

#[test]
fn rejects_swap_without_min_out() {
    let err = parse_source(TRADING_INVALID_MISSING_MIN_OUT_SOURCE)
        .expect_err("swap without `min_out` must be a parser diagnostic");
    assert!(
        format!("{err}").to_lowercase().contains("min_out"),
        "diagnostic should name the missing `min_out`, got: {err}"
    );
}

#[test]
fn rejects_duplicate_debt_names() {
    let source = r#"
asset USDC = evm.ethereum.0xA0b8 { decimals: 6 }
atomic trade DupDebt using MainnetArb {
    borrow 1 USDC from aave_v3 as debt
    borrow 2 USDC from aave_v3 as debt
    require all_debts_repaid
}
"#;
    let err = parse_source(source).expect_err("duplicate debt names must be rejected structurally");
    assert!(
        format!("{err}").to_lowercase().contains("debt"),
        "diagnostic should name the duplicated debt, got: {err}"
    );
}

#[test]
fn rejects_malformed_basis_points() {
    for source in [
        "risk policy BadBps { max_slippage: nope bps }",
        "risk policy BadBps { max_slippage: 30.5 bps }",
        "risk policy BadBps { max_flash_fee: 10.0 bps }",
    ] {
        assert!(
            parse_source(source).is_err(),
            "malformed basis points must be rejected: {source}"
        );
    }
}

#[test]
fn formatted_source_reparses_to_equivalent_ast() {
    let first = parse_source(TRADING_CORE_V1_SOURCE).expect("canonical fixture must parse");
    let formatted = X3Formatter::new().format_program(&first);
    let second = parse_source(&formatted).unwrap_or_else(|e| panic!("formatted output must reparse: {formatted}\n{e}"));
    assert_eq!(
        serde_json::to_value(&first).unwrap(),
        serde_json::to_value(&second).unwrap(),
        "formatter round-trip changed the trading AST"
    );
}

#[test]
fn symbol_and_debt_helpers_used_by_parser_output() {
    let _ = Symbol::from("helper");
    let _ = DebtId(Symbol::from("helper"));
    let _ = AmountExpr::literal(1, Symbol::from("USDC"));
}
