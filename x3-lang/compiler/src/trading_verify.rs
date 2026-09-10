//! Trading Core v1 control-flow and economic invariant verification.
//!
//! v1 trade bodies are linear, so this pass proves debt closure over the
//! single successful path. The state machine is intentionally conservative:
//! if debt can ever reach a successful exit unclosed, verification fails.

use std::collections::BTreeSet;

use x3_lang_ast::ast::{Expression, Item, LiteralExpr, Program};
use x3_lang_ast::{AtomicTradeDecl, DebtId, TradeStmt};
use x3_lang_common::{Span, X3Error};

use crate::semantic::CompilationMode;
use crate::trading_semantic::TradingSymbols;

/// Tracks the linear lifecycle of every debt inside one atomic trade.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DebtFlowState {
    pub open: BTreeSet<DebtId>,
    pub closed: BTreeSet<DebtId>,
}

/// Verify every trading declaration in a semantically analyzed program.
pub fn verify_trading_program(program: &Program, symbols: &TradingSymbols, mode: CompilationMode) -> Vec<X3Error> {
    let mut errors = Vec::new();
    for item in &program.items {
        if let Item::AtomicTrade(trade) = &item.node {
            errors.extend(verify_atomic_trade_at_span(trade, symbols, mode, item.span));
        }
    }
    errors
}

/// Prove that a single atomic trade closes every debt on its successful path
/// and satisfies the selected risk policy and guard requirements.
pub fn verify_atomic_trade(trade: &AtomicTradeDecl, symbols: &TradingSymbols, mode: CompilationMode) -> Vec<X3Error> {
    verify_atomic_trade_at_span(trade, symbols, mode, Span::DUMMY)
}

fn verify_atomic_trade_at_span(
    trade: &AtomicTradeDecl,
    symbols: &TradingSymbols,
    mode: CompilationMode,
    span: Span,
) -> Vec<X3Error> {
    let mut errors = Vec::new();
    let policy = match symbols.policies.get(&trade.risk_policy) {
        Some(policy) => policy,
        None => {
            errors.push(semantic_error(
                format!(
                    "atomic trade '{}' references unknown risk policy '{}'",
                    trade.name.as_str(),
                    trade.risk_policy.as_str()
                ),
                span,
            ));
            return errors;
        }
    };

    enforce_policy_bounds(policy, mode, span, &mut errors);

    let mut state = DebtFlowState::default();
    let mut has_borrow = false;
    let mut has_net_profit_guard = false;
    let mut has_all_debts_guard = false;
    let mut has_receipt = false;

    for stmt in &trade.body {
        match stmt {
            TradeStmt::Borrow { debt, .. } => {
                has_borrow = true;
                if !state.open.insert(debt.clone()) {
                    errors.push(semantic_error(
                        format!(
                            "atomic trade '{}' borrows debt '{}' more than once",
                            trade.name.as_str(),
                            debt.0.as_str()
                        ),
                        span,
                    ));
                }
            }
            TradeStmt::Repay { debt } => {
                if state.closed.contains(debt) {
                    errors.push(semantic_error(
                        format!(
                            "atomic trade '{}' repays debt '{}' more than once",
                            trade.name.as_str(),
                            debt.0.as_str()
                        ),
                        span,
                    ));
                } else if state.open.remove(debt) {
                    state.closed.insert(debt.clone());
                } else {
                    errors.push(semantic_error(
                        format!(
                            "atomic trade '{}' repays unknown or already-closed debt '{}'",
                            trade.name.as_str(),
                            debt.0.as_str()
                        ),
                        span,
                    ));
                }
            }
            TradeStmt::RequireMinNetProfit { .. } => has_net_profit_guard = true,
            TradeStmt::RequireAllDebtsRepaid => has_all_debts_guard = true,
            TradeStmt::EmitReceipt => has_receipt = true,
            TradeStmt::Swap { .. } => {}
        }
    }

    if !state.open.is_empty() {
        let names: Vec<&str> = state.open.iter().map(|debt| debt.0.as_str()).collect();
        errors.push(semantic_error(
            format!(
                "atomic trade '{}' can succeed with unrepaid debt: {}",
                trade.name.as_str(),
                names.join(", ")
            ),
            span,
        ));
    }
    if has_borrow && !has_all_debts_guard {
        errors.push(semantic_error(
            format!(
                "atomic trade '{}' borrows capital but never asserts all_debts_repaid",
                trade.name.as_str()
            ),
            span,
        ));
    }
    if has_borrow && !has_receipt {
        errors.push(semantic_error(
            format!(
                "atomic trade '{}' borrows capital but never emits a receipt",
                trade.name.as_str()
            ),
            span,
        ));
    }
    if !has_net_profit_guard && policy.min_profit.is_none() {
        errors.push(semantic_error(
            format!(
                "atomic trade '{}' has no minimum net-profit guard",
                trade.name.as_str()
            ),
            span,
        ));
    }

    errors
}

fn enforce_policy_bounds(
    policy: &x3_lang_ast::TradeRiskPolicy,
    mode: CompilationMode,
    span: Span,
    errors: &mut Vec<X3Error>,
) {
    if policy.max_slippage_bps > 10_000 {
        errors.push(semantic_error(
            format!(
                "risk policy '{}' has max_slippage {} bps above the 10000 bps ceiling",
                policy.name.as_str(),
                policy.max_slippage_bps
            ),
            span,
        ));
    }
    if policy.max_flash_fee_bps > 10_000 {
        errors.push(semantic_error(
            format!(
                "risk policy '{}' has max_flash_fee {} bps above the 10000 bps ceiling",
                policy.name.as_str(),
                policy.max_flash_fee_bps
            ),
            span,
        ));
    }
    if deadline_is_zero(&policy.deadline) {
        errors.push(semantic_error(
            format!("risk policy '{}' has a zero deadline", policy.name.as_str()),
            span,
        ));
    }
    if mode == CompilationMode::Mainnet && policy.require_private_submission {
        errors.push(semantic_error(
            format!(
                "risk policy '{}' requires private submission but no production private-submission capability is attested",
                policy.name.as_str()
            ),
            span,
        ));
    }
}

fn deadline_is_zero(expr: &Expression) -> bool {
    match expr {
        Expression::Literal(LiteralExpr::Int { value: 0, .. }) => true,
        Expression::Literal(LiteralExpr::Int { value, .. }) => *value == 0,
        _ => false,
    }
}

fn semantic_error(message: impl Into<String>, span: Span) -> X3Error {
    X3Error::SemanticError {
        message: message.into(),
        span,
    }
}
