//! Lower Trading Core v1 AST declarations into deterministic IR.

use std::fmt;

use x3_lang_ast::ast::{Expression, LiteralExpr};
use x3_lang_ast::{AtomicTradeDecl, RoundingMode, TradeRiskPolicy, TradeStmt};
use x3_lang_common::{Symbol, X3Error};

use crate::ir::{AssetKey, Operation, TradingOperation, ValueRef};
use crate::trading_semantic::{decimal_to_base_units, TradingSymbols};

/// Errors produced while lowering a verified atomic trade.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LowerError {
    pub message: String,
}

impl fmt::Display for LowerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for LowerError {}

impl From<LowerError> for X3Error {
    fn from(error: LowerError) -> Self {
        X3Error::SemanticError {
            message: error.message,
            span: x3_lang_common::Span::DUMMY,
        }
    }
}

/// Lower one verified atomic trade into explicit accounting IR.
pub fn lower_atomic_trade(trade: &AtomicTradeDecl, symbols: &TradingSymbols) -> Result<Vec<Operation>, LowerError> {
    let policy = symbols.policies.get(&trade.risk_policy).ok_or_else(|| LowerError {
        message: format!(
            "atomic trade '{}' references unknown risk policy '{}'",
            trade.name.as_str(),
            trade.risk_policy.as_str()
        ),
    })?;

    let mut operations = vec![Operation::Trading(TradingOperation::BeginAtomicTrade {
        trade_id: trade.name.as_str().to_string(),
        policy_id: policy.name.as_str().to_string(),
    })];
    let mut has_borrow = false;
    let mut has_receipt = false;

    for stmt in &trade.body {
        match stmt {
            TradeStmt::Borrow { amount, provider, debt } => {
                has_borrow = true;
                operations.push(Operation::Trading(TradingOperation::OpenDebt {
                    debt_id: debt.0.as_str().to_string(),
                    provider: provider.as_str().to_string(),
                    asset: asset_key(&amount.asset, symbols)?,
                    principal: literal_amount(amount, symbols, "borrow amount")?,
                }));
            }
            TradeStmt::Swap {
                binding,
                input,
                from_asset,
                to_asset,
                venue,
                min_output,
            } => {
                operations.push(Operation::Trading(TradingOperation::ExecuteSwap {
                    binding: binding.as_str().to_string(),
                    venue: venue.as_str().to_string(),
                    from: asset_key(from_asset, symbols)?,
                    to: asset_key(to_asset, symbols)?,
                    input: binding_ref(&input.value, symbols, &input.asset)?,
                    min_output: literal_amount(min_output, symbols, "swap min_out")?,
                }));
            }
            TradeStmt::Repay { debt } => {
                operations.push(Operation::Trading(TradingOperation::CloseDebt {
                    debt_id: debt.0.as_str().to_string(),
                }));
            }
            TradeStmt::RequireMinNetProfit { amount } => {
                operations.push(Operation::Trading(TradingOperation::AssertMinNetProfit {
                    settlement_asset: asset_key(&amount.asset, symbols)?,
                    minimum: literal_amount(amount, symbols, "net_profit")?,
                }));
            }
            TradeStmt::RequireAllDebtsRepaid => {
                operations.push(Operation::Trading(TradingOperation::AssertAllDebtsClosed));
            }
            TradeStmt::EmitReceipt => {
                has_receipt = true;
                operations.push(Operation::Trading(TradingOperation::EmitTradeReceipt));
            }
        }
    }

    if has_borrow && !has_receipt {
        operations.push(Operation::Trading(TradingOperation::EmitTradeReceipt));
    }
    operations.push(Operation::Trading(TradingOperation::CommitAtomicTrade));

    Ok(operations)
}

/// Convert the declared policy itself into stable IR metadata when needed by
/// later passes. The policy is already embedded in BeginAtomicTrade by id.
pub fn policy_id(policy: &TradeRiskPolicy) -> String {
    policy.name.as_str().to_string()
}

fn asset_key(symbol: &Symbol, symbols: &TradingSymbols) -> Result<AssetKey, LowerError> {
    let asset = symbols.assets.get(symbol).ok_or_else(|| LowerError {
        message: format!("atomic trade references undeclared asset '{}'", symbol.as_str()),
    })?;
    Ok(AssetKey {
        vm_family: asset.vm_family.as_str().to_string(),
        chain: asset.chain.as_str().to_string(),
        canonical_id: asset.canonical_id.as_str().to_string(),
        symbol: asset.symbol.as_str().to_string(),
        decimals: asset.decimals,
    })
}

fn literal_amount(
    amount: &x3_lang_ast::AmountExpr,
    symbols: &TradingSymbols,
    context: &str,
) -> Result<u128, LowerError> {
    let asset = symbols.assets.get(&amount.asset).ok_or_else(|| LowerError {
        message: format!("{context} references undeclared asset '{}'", amount.asset.as_str()),
    })?;
    let literal = literal_text(&amount.value).ok_or_else(|| LowerError {
        message: format!("{context} must be a literal amount before lowering"),
    })?;
    decimal_to_base_units(&literal, asset.decimals, RoundingMode::Exact).map_err(|err| LowerError {
        message: format!("{context}: {err}"),
    })
}

fn binding_ref(value: &Expression, symbols: &TradingSymbols, asset_symbol: &Symbol) -> Result<ValueRef, LowerError> {
    match value {
        Expression::Ident(name) => Ok(ValueRef::Binding(name.as_str().to_string())),
        Expression::FieldAccess { target, field } => match target.as_ref() {
            Expression::Ident(owner) => Ok(ValueRef::Binding(format!("{}.{}", owner.as_str(), field.as_str()))),
            _ => Err(LowerError {
                message: "swap input field access must reference a named debt binding".to_string(),
            }),
        },
        Expression::Literal(_) => {
            let amount = x3_lang_ast::AmountExpr {
                value: value.clone(),
                asset: asset_symbol.clone(),
            };
            Ok(ValueRef::Literal(literal_amount(&amount, symbols, "swap input")?))
        }
        _ => Err(LowerError {
            message: "swap input must be a literal, binding, or debt amount".to_string(),
        }),
    }
}

fn literal_text(expr: &Expression) -> Option<String> {
    match expr {
        Expression::Literal(LiteralExpr::Int { value, .. }) => Some(value.to_string()),
        Expression::Literal(LiteralExpr::Float { raw, .. }) => Some(raw.as_str().to_string()),
        _ => None,
    }
}
