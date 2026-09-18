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
fn parses_invariant_solvent_statement() {
    let source = r#"
asset USDC = evm.ethereum.0xA0b8 { decimals: 6 }
atomic trade WithInvariant using MainnetArb {
    invariant solvent
    require all_debts_repaid
    emit receipt
}
"#;
    let program = parse_source(source).expect("invariant statement must parse");
    let (_, _, trade) = find_items(&program);
    let trade = trade.expect("program must contain the atomic trade");
    assert!(
        matches!(
            trade.body.first(),
            Some(TradeStmt::AssertInvariant {
                kind: x3_lang_ast::InvariantKind::Solvent
            })
        ),
        "first statement must lower to AssertInvariant(Solvent), got: {:?}",
        trade.body.first()
    );
}

#[test]
fn rejects_unknown_invariant_name() {
    let source = r#"
asset USDC = evm.ethereum.0xA0b8 { decimals: 6 }
atomic trade BadInvariant using MainnetArb {
    invariant made_up_property
}
"#;
    let err = parse_source(source).expect_err("unknown invariant name must be a parser diagnostic");
    assert!(
        format!("{err}").to_lowercase().contains("invariant"),
        "diagnostic should name the invalid invariant, got: {err}"
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
        "risk policy BadBps { max_oracle_deviation: nope bps }",
    ] {
        assert!(
            parse_source(source).is_err(),
            "malformed basis points must be rejected: {source}"
        );
    }
}

#[test]
fn max_oracle_deviation_is_optional_and_parses_when_present() {
    let fixture_program = parse_source(TRADING_CORE_V1_SOURCE).expect("fixture must parse");
    let (_, policy_without, _) = find_items(&fixture_program);
    assert_eq!(
        policy_without
            .expect("fixture must declare a policy")
            .max_oracle_deviation_bps,
        None,
        "the canonical fixture never declares max_oracle_deviation — must default to None, not error"
    );

    let source = r#"
asset USDC = evm.ethereum.0xA0b8 { decimals: 6 }
asset ETH = evm.ethereum.0x0000000000000000000000000000000000000000 { decimals: 18 }
risk policy WithOracle {
    max_slippage: 30 bps
    max_gas: 0.02 ETH
    max_flash_fee: 10 bps
    deadline: 2 blocks
    require_private_submission: false
    max_oracle_deviation: 50 bps
}
"#;
    let program = parse_source(source).expect("policy declaring max_oracle_deviation must parse");
    let (_, policy, _) = find_items(&program);
    assert_eq!(
        policy.expect("policy must be present").max_oracle_deviation_bps,
        Some(50)
    );
}

#[test]
fn duplicate_max_oracle_deviation_is_rejected() {
    let source = r#"
asset USDC = evm.ethereum.0xA0b8 { decimals: 6 }
asset ETH = evm.ethereum.0x0000000000000000000000000000000000000000 { decimals: 18 }
risk policy P {
    max_slippage: 30 bps
    max_gas: 0.02 ETH
    max_flash_fee: 10 bps
    deadline: 2 blocks
    require_private_submission: false
    max_oracle_deviation: 50 bps
    max_oracle_deviation: 60 bps
}
"#;
    let err = parse_source(source).expect_err("duplicate max_oracle_deviation must be a parser diagnostic");
    assert!(
        format!("{err}").to_lowercase().contains("max_oracle_deviation"),
        "diagnostic should name the duplicated field, got: {err}"
    );
}

/// The four policy fields below used to be fabricated by the lowering pass from
/// unrelated source fields, so no program could set them. Only
/// `quote_freshness` survived — it is now source-declarable and enforced. These
/// tests pin the syntax that replaced the fabrication.
#[test]
fn quote_freshness_is_optional_and_parses_when_present() {
    let fixture_program = parse_source(TRADING_CORE_V1_SOURCE).expect("fixture must parse");
    let (_, policy_without, _) = find_items(&fixture_program);
    assert_eq!(
        policy_without.expect("fixture must declare a policy").quote_freshness,
        None,
        "the canonical fixture never declares quote_freshness — must default to None, not error"
    );

    let source = r#"
asset USDC = evm.ethereum.0xA0b8 { decimals: 6 }
asset ETH = evm.ethereum.0x0000000000000000000000000000000000000000 { decimals: 18 }
risk policy WithFreshness {
    max_slippage: 30 bps
    max_gas: 0.02 ETH
    max_flash_fee: 10 bps
    deadline: 2 blocks
    require_private_submission: false
    quote_freshness: 12
}
"#;
    let program = parse_source(source).expect("policy declaring quote_freshness must parse");
    let (_, policy, _) = find_items(&program);
    assert_eq!(policy.expect("policy must be present").quote_freshness, Some(12));
}

#[test]
fn duplicate_quote_freshness_is_rejected() {
    let source = r#"
asset USDC = evm.ethereum.0xA0b8 { decimals: 6 }
asset ETH = evm.ethereum.0x0000000000000000000000000000000000000000 { decimals: 18 }
risk policy P {
    max_slippage: 30 bps
    max_gas: 0.02 ETH
    max_flash_fee: 10 bps
    deadline: 2 blocks
    require_private_submission: false
    quote_freshness: 12
    quote_freshness: 24
}
"#;
    let err = parse_source(source).expect_err("duplicate quote_freshness must be a parser diagnostic");
    assert!(
        format!("{err}").to_lowercase().contains("quote_freshness"),
        "diagnostic should name the duplicated field, got: {err}"
    );
}

#[test]
fn a_zero_or_fractional_quote_freshness_ceiling_is_rejected() {
    // Zero would mean "no quote may ever be used", which is never what a policy
    // author means; the way to express "no bound" is to omit the field.
    for ceiling in ["0", "12.5", "nope"] {
        let source = format!(
            r#"
asset USDC = evm.ethereum.0xA0b8 {{ decimals: 6 }}
asset ETH = evm.ethereum.0x0000000000000000000000000000000000000000 {{ decimals: 18 }}
risk policy P {{
    max_slippage: 30 bps
    max_gas: 0.02 ETH
    max_flash_fee: 10 bps
    deadline: 2 blocks
    require_private_submission: false
    quote_freshness: {ceiling}
}}
"#
        );
        assert!(
            parse_source(&source).is_err(),
            "quote_freshness: {ceiling} must be rejected"
        );
    }
}

#[test]
fn max_cumulative_loss_is_optional_and_parses_when_present() {
    let fixture_program = parse_source(TRADING_CORE_V1_SOURCE).expect("fixture must parse");
    let (_, policy_without, _) = find_items(&fixture_program);
    assert!(
        policy_without
            .expect("fixture must declare a policy")
            .max_cumulative_loss
            .is_none(),
        "the canonical fixture never declares max_cumulative_loss — must default to None, not error"
    );

    let source = r#"
asset USDC = evm.ethereum.0xA0b8 { decimals: 6 }
asset ETH = evm.ethereum.0x0000000000000000000000000000000000000000 { decimals: 18 }
risk policy WithBreaker {
    max_slippage: 30 bps
    max_gas: 0.02 ETH
    max_flash_fee: 10 bps
    deadline: 2 blocks
    require_private_submission: false
    max_cumulative_loss: 500000000 USDC
}
"#;
    let program = parse_source(source).expect("policy declaring max_cumulative_loss must parse");
    let (_, policy, _) = find_items(&program);
    let amount = policy
        .expect("policy must be present")
        .max_cumulative_loss
        .as_ref()
        .expect("max_cumulative_loss must be Some");
    assert_eq!(amount.asset.as_str(), "USDC");
}

#[test]
fn duplicate_max_cumulative_loss_is_rejected() {
    let source = r#"
asset USDC = evm.ethereum.0xA0b8 { decimals: 6 }
asset ETH = evm.ethereum.0x0000000000000000000000000000000000000000 { decimals: 18 }
risk policy P {
    max_slippage: 30 bps
    max_gas: 0.02 ETH
    max_flash_fee: 10 bps
    deadline: 2 blocks
    require_private_submission: false
    max_cumulative_loss: 500000000 USDC
    max_cumulative_loss: 1000000000 USDC
}
"#;
    let err = parse_source(source).expect_err("duplicate max_cumulative_loss must be a parser diagnostic");
    assert!(
        format!("{err}").to_lowercase().contains("max_cumulative_loss"),
        "diagnostic should name the duplicated field, got: {err}"
    );
}

#[test]
fn formatted_source_reparses_to_equivalent_ast() {
    let first = parse_source(TRADING_CORE_V1_SOURCE).expect("canonical fixture must parse");
    let formatted = X3Formatter::new().format_program(&first);
    let second = parse_source(&formatted).unwrap_or_else(|e| panic!("formatted output must reparse: {formatted}\n{e}"));

    let first_nodes: Vec<_> = first.items.iter().map(|item| &item.node).collect();
    let second_nodes: Vec<_> = second.items.iter().map(|item| &item.node).collect();
    assert_eq!(
        serde_json::to_value(first_nodes).unwrap(),
        serde_json::to_value(second_nodes).unwrap(),
        "formatter round-trip changed the semantic trading AST"
    );

    for item in &second.items {
        assert!(!item.span.is_dummy(), "formatted parse must preserve source spans");
        assert!(
            item.span.to_range().end <= formatted.len(),
            "formatted parse span must stay inside formatted source"
        );
    }
}

#[test]
fn symbol_and_debt_helpers_used_by_parser_output() {
    let _ = Symbol::from("helper");
    let _ = DebtId(Symbol::from("helper"));
    let _ = AmountExpr::literal(1, Symbol::from("USDC"));
}
#[test]
fn formatter_round_trips_max_oracle_deviation_and_max_cumulative_loss() {
    // Neither field is set on the canonical fixture, so the general
    // round-trip test above never exercises their formatter output.
    // format_trade_risk_policy silently dropped max_oracle_deviation_bps
    // entirely until this test was written to catch it: a policy that
    // declared an oracle-deviation ceiling would lose that ceiling on any
    // format round-trip, with no error and no warning.
    let source = r#"
asset USDC = evm.ethereum.0xA0b8 { decimals: 6 }
asset ETH = evm.ethereum.0x0000000000000000000000000000000000000000 { decimals: 18 }
risk policy WithBreakers {
    max_slippage: 30 bps
    max_gas: 0.02 ETH
    max_flash_fee: 10 bps
    deadline: 2 blocks
    require_private_submission: false
    max_oracle_deviation: 50 bps
    max_cumulative_loss: 500000000 USDC
}
"#;
    let first = parse_source(source).expect("must parse");
    let formatted = X3Formatter::new().format_program(&first);
    assert!(
        formatted.contains("max_oracle_deviation"),
        "formatter dropped max_oracle_deviation:\n{formatted}"
    );
    assert!(
        formatted.contains("max_cumulative_loss"),
        "formatter dropped max_cumulative_loss:\n{formatted}"
    );

    let second = parse_source(&formatted).unwrap_or_else(|e| panic!("formatted output must reparse: {formatted}\n{e}"));
    let (_, first_policy, _) = find_items(&first);
    let (_, second_policy, _) = find_items(&second);
    assert_eq!(
        serde_json::to_value(first_policy).unwrap(),
        serde_json::to_value(second_policy).unwrap(),
        "formatter round-trip changed the risk policy AST"
    );
}

const BRIDGE_SOURCE: &str = r#"
asset USDC = evm.ethereum.0xA0b8 { decimals: 6 }
asset ETH = evm.ethereum.0x0000000000000000000000000000000000000000 { decimals: 18 }
asset USDC_BASE = evm.base.0xB1a0 { decimals: 6 }
risk policy P {
    max_slippage: 30 bps
    max_gas: 0.02 ETH
    max_flash_fee: 10 bps
    deadline: 2 blocks
    require_private_submission: false
}
atomic trade CrossChainSettle using P {
    bridge 1_000_000 USDC -> USDC_BASE via wormhole to "0x1234567890abcdef1234567890abcdef12345678"
    require net_profit >= 1 USDC_BASE
    require all_debts_repaid
    emit receipt
}
"#;

#[test]
fn parses_bridge_statement() {
    let program = parse_source(BRIDGE_SOURCE).unwrap_or_else(|e| panic!("bridge source must parse: {e}"));
    let (_, _, trade) = find_items(&program);
    let trade = trade.expect("trade must be present");
    let bridge = trade
        .body
        .iter()
        .find_map(|stmt| match stmt {
            TradeStmt::Bridge {
                input,
                from_asset,
                to_asset,
                via,
                receiver,
            } => Some((input, from_asset, to_asset, via, receiver)),
            _ => None,
        })
        .expect("trade must contain a bridge statement");
    let (input, from_asset, to_asset, via, receiver) = bridge;
    assert_eq!(from_asset.as_str(), "USDC");
    assert_eq!(input.asset.as_str(), "USDC");
    assert_eq!(to_asset.as_str(), "USDC_BASE");
    assert_eq!(via.as_str(), "wormhole");
    assert!(matches!(receiver, Expression::Literal(LiteralExpr::String(_))));
}

#[test]
fn bridge_formatter_round_trips_to_equivalent_ast() {
    let first = parse_source(BRIDGE_SOURCE).expect("bridge source must parse");
    let formatted = X3Formatter::new().format_program(&first);
    assert!(
        formatted.contains("bridge"),
        "formatter dropped the bridge statement:\n{formatted}"
    );
    assert!(
        formatted.contains("USDC_BASE"),
        "formatter dropped the bridge destination asset:\n{formatted}"
    );

    let second = parse_source(&formatted).unwrap_or_else(|e| panic!("formatted output must reparse: {formatted}\n{e}"));
    let (_, _, first_trade) = find_items(&first);
    let (_, _, second_trade) = find_items(&second);
    assert_eq!(
        serde_json::to_value(first_trade).unwrap(),
        serde_json::to_value(second_trade).unwrap(),
        "formatter round-trip changed the bridge statement AST"
    );
}

#[test]
fn bridge_without_receiver_is_rejected() {
    let source = BRIDGE_SOURCE.replace(r#"to "0x1234567890abcdef1234567890abcdef12345678""#, "");
    let err = parse_source(&source).expect_err("bridge without 'to <receiver>' must be a parser diagnostic");
    assert!(
        format!("{err}").to_lowercase().contains("receiver"),
        "diagnostic should name the missing receiver, got: {err}"
    );
}

#[test]
fn bridge_without_via_is_rejected() {
    let source = BRIDGE_SOURCE.replace("via wormhole ", "");
    let err = parse_source(&source).expect_err("bridge without 'via <bridge>' must be a parser diagnostic");
    assert!(
        format!("{err}").to_lowercase().contains("via"),
        "diagnostic should name the missing 'via' clause, got: {err}"
    );
}
