//! Lower Trading Core v1 AST declarations into deterministic IR.

use std::{collections::BTreeSet, fmt};

use x3_lang_ast::ast::{Expression, LiteralExpr};
use x3_lang_ast::{AtomicTradeDecl, RoundingMode, TradeRiskPolicy, TradeStmt};
use x3_lang_common::{Symbol, X3Error};

use crate::ir::{
    AssetKey, CompiledTradingPolicy, CostKind, Operation, StateBindingMode, SubmissionProfile, TradingOperation,
    ValueRef,
};
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

    let policy_chain = trade
        .body
        .iter()
        .find_map(|stmt| match stmt {
            TradeStmt::Borrow { amount, .. } => symbols
                .assets
                .get(&amount.asset)
                .map(|asset| asset.chain.as_str().to_string()),
            TradeStmt::Swap { from_asset, .. } => symbols
                .assets
                .get(from_asset)
                .map(|asset| asset.chain.as_str().to_string()),
            _ => None,
        })
        .ok_or_else(|| LowerError {
            message: format!("atomic trade '{}' has no chain-qualified asset", trade.name.as_str()),
        })?;

    let max_gas = literal_amount(&policy.max_gas, symbols, "policy max_gas")?;
    let max_gas_asset = asset_key(&policy.max_gas.asset, symbols)?;
    let (minimum_net_profit, minimum_net_profit_asset) = match &policy.min_profit {
        Some(amount) => (
            Some(literal_amount(amount, symbols, "policy min_profit")?),
            Some(asset_key(&amount.asset, symbols)?),
        ),
        None => (None, None),
    };
    let (max_cumulative_loss, max_cumulative_loss_asset) = match &policy.max_cumulative_loss {
        Some(amount) => (
            Some(literal_amount(amount, symbols, "policy max_cumulative_loss")?),
            Some(asset_key(&amount.asset, symbols)?),
        ),
        None => (None, None),
    };
    // Through the language's one duration rule, so a deadline written in time
    // (`2h`) is the same number of blocks as the same duration written anywhere
    // else, and a fraction of a block is refused rather than truncated
    // (TICKET-048).
    let deadline_blocks = u64::from(
        crate::lowering::expression_to_blocks(&policy.deadline).map_err(|error| LowerError {
            message: format!("policy deadline: {error}"),
        })?,
    );

    let compiled_policy = CompiledTradingPolicy {
        policy_id: policy.name.as_str().to_string(),
        policy_version: 1,
        chain: policy_chain,
        max_slippage_bps: policy.max_slippage_bps,
        max_gas,
        max_gas_asset,
        max_flash_fee_bps: policy.max_flash_fee_bps,
        deadline_blocks,
        require_private_submission: policy.require_private_submission,
        minimum_net_profit,
        minimum_net_profit_asset,
        quote_freshness_blocks: policy.quote_freshness,
        submission_profile: if policy.require_private_submission {
            SubmissionProfile::Private
        } else {
            SubmissionProfile::Public
        },
        state_binding: StateBindingMode::Exact,
        // The default allowlist must cover every category a v1 trade body
        // can actually generate, or enabling enforcement would make legal
        // trades unpayable. A trade body can bridge (cross-domain fee) and
        // a bridge is settled against a finality/inclusion proof (proof
        // fee), so both belong here alongside gas and the venue-side
        // slippage/impact categories. Solver infrastructure fees stay
        // excluded: nothing in a v1 trade body charges one, so a host
        // reporting that category is rejected.
        allowed_cost_kinds: BTreeSet::from([
            CostKind::Gas,
            CostKind::LiquidityFee,
            CostKind::FlashLiquidityFee,
            CostKind::ProofFee,
            CostKind::CrossDomainFee,
            CostKind::Slippage,
            CostKind::PriceImpact,
            CostKind::MevLeakage,
        ]),
        allow_mint: false,
        allow_burn: false,
        max_oracle_deviation_bps: policy.max_oracle_deviation_bps,
        max_cumulative_loss,
        max_cumulative_loss_asset,
        max_price_impact_bps: policy.max_price_impact_bps,
        max_mev_leakage_bps: policy.max_mev_leakage_bps,
    };

    let mut operations = vec![Operation::Trading(TradingOperation::BeginAtomicTrade {
        trade_id: trade.name.as_str().to_string(),
        policy: compiled_policy,
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
            TradeStmt::Bridge {
                input,
                from_asset,
                to_asset,
                via,
                receiver,
            } => {
                operations.push(Operation::Trading(TradingOperation::Bridge {
                    via: via.as_str().to_string(),
                    from: asset_key(from_asset, symbols)?,
                    to: asset_key(to_asset, symbols)?,
                    input: binding_ref(&input.value, symbols, &input.asset)?,
                    receiver: literal_string(receiver, "bridge receiver")?,
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
            TradeStmt::AssertInvariant { kind } => {
                let ir_kind = match kind {
                    x3_lang_ast::InvariantKind::Solvent => crate::ir::InvariantKind::Solvent,
                };
                operations.push(Operation::Trading(TradingOperation::AssertInvariant { kind: ir_kind }));
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

fn literal_string(expr: &Expression, context: &str) -> Result<String, LowerError> {
    match expr {
        Expression::Literal(LiteralExpr::String(value)) => Ok(value.as_str().to_string()),
        _ => Err(LowerError {
            message: format!("{context} must be a string literal"),
        }),
    }
}
