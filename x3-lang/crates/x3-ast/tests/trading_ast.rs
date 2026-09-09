#![cfg(test)]

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use x3_lang_ast::trading::{
    AmountExpr, AssetDecl, AssetId, AtomicTradeDecl, DebtId, RoundingMode, TradeRiskPolicy, TradeStmt,
};
use x3_lang_ast::{ChainRef, Expression, Item, LiteralExpr, Program};
use x3_lang_common::{BytePos, IntBase, Span, Spanned, Symbol};

fn int_literal(value: u128) -> Expression {
    Expression::Literal(LiteralExpr::Int {
        value,
        base: IntBase::Decimal,
        suffix: None,
    })
}

#[test]
fn asset_id_constructs_with_exact_fields() {
    let asset = AssetId {
        vm_family: Symbol::from("evm"),
        chain: ChainRef::new(Symbol::from("ethereum")),
        canonical_id: Symbol::from("0xA0b8"),
        symbol: Symbol::from("USDC"),
        decimals: 6,
    };

    assert_eq!(asset.decimals, 6);
    assert_eq!(asset.vm_family.as_str(), "evm");
    assert_eq!(asset.chain.as_str(), "ethereum");
    assert_eq!(asset.canonical_id.as_str(), "0xA0b8");
    assert_eq!(asset.symbol.as_str(), "USDC");
}

#[test]
fn atomic_trade_body_matches_plan_sample() {
    let trade = AtomicTradeDecl {
        name: Symbol::from("CrossDexArb"),
        risk_policy: Symbol::from("MainnetArb"),
        body: vec![
            TradeStmt::Borrow {
                amount: AmountExpr::literal(1_000_000_000_000, Symbol::from("USDC")),
                provider: Symbol::from("aave_v3"),
                debt: DebtId(Symbol::from("debt")),
            },
            TradeStmt::Repay {
                debt: DebtId(Symbol::from("debt")),
            },
            TradeStmt::RequireAllDebtsRepaid,
            TradeStmt::EmitReceipt,
        ],
    };

    assert_eq!(trade.name.as_str(), "CrossDexArb");
    assert_eq!(trade.risk_policy.as_str(), "MainnetArb");
    assert_eq!(trade.body.len(), 4);

    match &trade.body[0] {
        TradeStmt::Borrow { amount, provider, debt } => {
            assert_eq!(provider.as_str(), "aave_v3");
            assert_eq!(debt.0.as_str(), "debt");
            assert_eq!(amount.asset.as_str(), "USDC");
            match &amount.value {
                Expression::Literal(LiteralExpr::Int {
                    value,
                    base: IntBase::Decimal,
                    suffix: None,
                }) => assert_eq!(*value, 1_000_000_000_000),
                other => panic!("borrow amount must stay a lossless integer literal, got {other:?}"),
            }
        }
        other => panic!("expected Borrow as first statement, got {other:?}"),
    }

    assert!(matches!(&trade.body[1], TradeStmt::Repay { debt } if debt.0.as_str() == "debt"));
    assert!(matches!(trade.body[2], TradeStmt::RequireAllDebtsRepaid));
    assert!(matches!(trade.body[3], TradeStmt::EmitReceipt));
}

#[test]
fn amount_expr_literal_is_lossless_decimal_expression() {
    let amount = AmountExpr::literal(2_000_000_000_000_000_000u128, Symbol::from("WETH"));

    assert_eq!(amount.asset.as_str(), "WETH");
    match amount.value {
        Expression::Literal(LiteralExpr::Int {
            value,
            base: IntBase::Decimal,
            suffix: None,
        }) => assert_eq!(value, 2_000_000_000_000_000_000),
        other => panic!("AmountExpr::literal must preserve the integer literal, got {other:?}"),
    }
}

#[test]
fn trade_risk_policy_holds_all_enforceable_fields() {
    let policy = TradeRiskPolicy {
        name: Symbol::from("MainnetArb"),
        max_slippage_bps: 30,
        max_gas: AmountExpr::literal(20_000_000_000_000_000u128, Symbol::from("ETH")),
        max_flash_fee_bps: 10,
        deadline: int_literal(2),
        require_private_submission: true,
        min_profit: Some(AmountExpr::literal(1_000_000_000_000, Symbol::from("USDC"))),
    };

    assert_eq!(policy.name.as_str(), "MainnetArb");
    assert_eq!(policy.max_slippage_bps, 30);
    assert_eq!(policy.max_flash_fee_bps, 10);
    assert!(policy.require_private_submission);
    assert!(policy.min_profit.is_some());
}

#[test]
fn debt_id_and_rounding_mode_are_plain_value_types() {
    let debt = DebtId(Symbol::from("debt"));
    assert_eq!(debt.0.as_str(), "debt");

    for mode in [RoundingMode::Down, RoundingMode::Up, RoundingMode::Exact] {
        assert_eq!(mode, mode);
    }
}

#[test]
fn asset_and_atomic_trade_are_top_level_items() {
    let span = Span::new(BytePos(0), BytePos(0), 0);
    let asset = AssetId {
        vm_family: Symbol::from("evm"),
        chain: ChainRef::new(Symbol::from("ethereum")),
        canonical_id: Symbol::from("0xA0b8"),
        symbol: Symbol::from("USDC"),
        decimals: 6,
    };
    let policy = TradeRiskPolicy {
        name: Symbol::from("MainnetArb"),
        max_slippage_bps: 30,
        max_gas: AmountExpr::literal(20_000_000_000_000_000, Symbol::from("ETH")),
        max_flash_fee_bps: 10,
        deadline: int_literal(2),
        require_private_submission: true,
        min_profit: Some(AmountExpr::literal(1_000_000_000_000, Symbol::from("USDC"))),
    };
    let program = Program::new(vec![
        Spanned::new(
            Item::AssetDecl(AssetDecl {
                name: Symbol::from("USDC"),
                asset,
            }),
            span,
        ),
        Spanned::new(Item::TradeRiskPolicy(policy), span),
        Spanned::new(
            Item::AtomicTrade(AtomicTradeDecl {
                name: Symbol::from("CrossDexArb"),
                risk_policy: Symbol::from("MainnetArb"),
                body: vec![TradeStmt::RequireAllDebtsRepaid],
            }),
            span,
        ),
    ]);

    assert_eq!(program.items.len(), 3);
    assert!(matches!(
        &program.items[0].node,
        Item::AssetDecl(decl) if decl.name.as_str() == "USDC"
    ));
    assert!(matches!(
        &program.items[1].node,
        Item::TradeRiskPolicy(policy) if policy.name.as_str() == "MainnetArb"
    ));
    assert!(matches!(
        &program.items[2].node,
        Item::AtomicTrade(decl) if decl.name.as_str() == "CrossDexArb"
    ));

    struct RecordingVisitor {
        trade_risk_policies: usize,
    }
    impl x3_lang_ast::visitor::AstVisitor for RecordingVisitor {
        fn visit_trade_risk_policy(&mut self, _p: &x3_lang_ast::TradeRiskPolicy) {
            self.trade_risk_policies += 1;
        }
    }
    let mut visitor = RecordingVisitor { trade_risk_policies: 0 };
    program.walk(&mut visitor);
    assert_eq!(visitor.trade_risk_policies, 1);
}

#[test]
fn trading_module_is_exposed_from_crate_root() {
    let asset = x3_lang_ast::AssetId {
        vm_family: Symbol::from("svm"),
        chain: ChainRef::new(Symbol::from("solana")),
        canonical_id: Symbol::from("So11111111111111111111111111111111111111112"),
        symbol: Symbol::from("WSOL"),
        decimals: 9,
    };
    assert_eq!(asset.decimals, 9);
}

#[test]
fn all_public_trading_types_implement_serde() {
    fn assert_serde<T>()
    where
        T: Serialize + DeserializeOwned,
    {
    }

    assert_serde::<AssetId>();
    assert_serde::<DebtId>();
    assert_serde::<RoundingMode>();
    assert_serde::<AmountExpr>();
    assert_serde::<AssetDecl>();
    assert_serde::<TradeRiskPolicy>();
    assert_serde::<AtomicTradeDecl>();
    assert_serde::<TradeStmt>();
}
