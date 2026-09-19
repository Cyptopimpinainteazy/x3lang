//! Trading Core v1 control-flow and economic invariant verification.
//!
//! v1 trade bodies are linear, so this pass proves debt closure over the
//! single successful path. The state machine is intentionally conservative:
//! if debt can ever reach a successful exit unclosed, verification fails.

use std::collections::BTreeSet;

use x3_lang_ast::ast::{Expression, Item, LiteralExpr, Program};
use x3_lang_ast::{AtomicTradeDecl, DebtId, TradeStmt};
use x3_lang_common::{Bps, Span, X3Error};

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

    // Declared effects and guarantees must be discharged by the body. That is
    // the whole point of declaring them: an `effects` list the compiler never
    // checks reads like a promise while meaning nothing, which is the same
    // defect as the policy ceilings that were aliased from unrelated fields.
    // This is what lets X3Lang refuse an economically broken trade before it
    // reaches a chain.
    for effect in &trade.effects {
        if !trade.body.iter().any(|stmt| effect.is_produced_by(stmt)) {
            errors.push(unresolved_effect_error(
                format!(
                    "atomic trade '{}' declares effect '{}' but no statement in the body produces it; \
                     add a '{}' statement or drop it from the effects list",
                    trade.name.as_str(),
                    effect.as_str(),
                    effect.as_str()
                ),
                span,
            ));
        }
    }
    for guarantee in &trade.guarantees {
        if !trade.body.iter().any(|stmt| guarantee.is_discharged_by(stmt)) {
            errors.push(unresolved_effect_error(
                format!(
                    "atomic trade '{}' declares guarantee '{}' but nothing in the body discharges it; {}",
                    trade.name.as_str(),
                    guarantee.as_str(),
                    guarantee_requirement(*guarantee)
                ),
                span,
            ));
        }
    }

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
    let mut seen_invariants = BTreeSet::new();
    // A plain `swap`/`borrow` is a single-venue, single-chain call — nothing
    // here models an actual bridge. Without this, referencing an asset
    // declared on a different chain (a typo, or a copy-paste from another
    // trade) silently compiles as if it were an ordinary same-chain call.
    let mut trade_chain: Option<(String, x3_lang_common::Symbol)> = None;
    // Once a trade bridges its proceeds away, nothing after it can still be
    // operating on the source chain — there is no "resume trading" after a
    // cross-chain move within one atomic trade.
    let mut bridged: Option<x3_lang_common::Symbol> = None;

    for stmt in &trade.body {
        for symbol in trade_stmt_asset_refs(stmt) {
            check_same_chain(
                symbol,
                symbols,
                &mut trade_chain,
                trade.name.as_str(),
                span,
                &mut errors,
            );
        }
        if let Some(bridge_via) = &bridged {
            if matches!(
                stmt,
                TradeStmt::Borrow { .. } | TradeStmt::Swap { .. } | TradeStmt::Repay { .. }
            ) {
                errors.push(semantic_error(
                    format!(
                        "atomic trade '{}' has a source-chain statement after bridging via '{}' — nothing can operate on the source chain once its proceeds have moved to another chain",
                        trade.name.as_str(),
                        bridge_via.as_str()
                    ),
                    span,
                ));
            }
        }
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
            TradeStmt::Bridge {
                from_asset,
                to_asset,
                via,
                ..
            } => {
                if bridged.is_some() {
                    errors.push(semantic_error(
                        format!("atomic trade '{}' bridges more than once", trade.name.as_str()),
                        span,
                    ));
                }
                bridged = Some(via.clone());
                if let (Some(from), Some(to)) = (symbols.assets.get(from_asset), symbols.assets.get(to_asset)) {
                    if from.chain.as_str() == to.chain.as_str() {
                        errors.push(semantic_error(
                            format!(
                                "atomic trade '{}' bridges '{}' to '{}', both on chain '{}' — a bridge must move between two different chains",
                                trade.name.as_str(),
                                from_asset.as_str(),
                                to_asset.as_str(),
                                from.chain.as_str()
                            ),
                            span,
                        ));
                    } else {
                        // Everything after a bridge is now expected to be on
                        // the destination chain — e.g. `require net_profit`
                        // naming the destination asset is the normal,
                        // expected pattern, not a fresh chain mismatch
                        // against whatever chain the trade started on.
                        trade_chain = Some((to.chain.as_str().to_string(), to_asset.clone()));
                    }
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
            TradeStmt::AssertInvariant { kind } => {
                if !seen_invariants.insert(kind.as_str()) {
                    errors.push(semantic_error(
                        format!(
                            "atomic trade '{}' declares invariant '{}' more than once",
                            trade.name.as_str(),
                            kind.as_str()
                        ),
                        span,
                    ));
                }
            }
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
            format!("atomic trade '{}' has no minimum net-profit guard", trade.name.as_str()),
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
    if !Bps::from_raw(u32::from(policy.max_slippage_bps)).is_within_whole() {
        errors.push(semantic_error(
            format!(
                "risk policy '{}' has max_slippage {} bps above the 10000 bps ceiling",
                policy.name.as_str(),
                policy.max_slippage_bps
            ),
            span,
        ));
    }
    if let Some(deviation_bps) = policy.max_oracle_deviation_bps {
        if !Bps::from_raw(u32::from(deviation_bps)).is_within_whole() {
            errors.push(semantic_error(
                format!(
                    "risk policy '{}' has max_oracle_deviation {deviation_bps} bps above the 10000 bps ceiling",
                    policy.name.as_str()
                ),
                span,
            ));
        }
    }
    if !Bps::from_raw(u32::from(policy.max_flash_fee_bps)).is_within_whole() {
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

/// Every asset symbol a single trade statement references. A plain `swap`
/// is one venue call, so every asset it touches must be on the same chain
/// as the rest of the trade — this is what lets callers find every asset
/// worth chain-checking without duplicating the match in the caller.
fn trade_stmt_asset_refs(stmt: &TradeStmt) -> Vec<&x3_lang_common::Symbol> {
    match stmt {
        TradeStmt::Borrow { amount, .. } => vec![&amount.asset],
        TradeStmt::Swap {
            from_asset,
            to_asset,
            min_output,
            ..
        } => vec![from_asset, to_asset, &min_output.asset],
        // Only from_asset is checked against the trade's single consistent
        // chain — to_asset is the deliberate exception: a bridge exists
        // specifically to move value onto a different chain, so checking
        // it here would reject the one legitimate cross-chain reference in
        // the whole language. The Bridge match arm in the caller enforces
        // its own, different invariant instead: to_asset's chain must
        // actually differ from from_asset's.
        TradeStmt::Bridge { from_asset, .. } => vec![from_asset],
        TradeStmt::RequireMinNetProfit { amount } => vec![&amount.asset],
        TradeStmt::Repay { .. }
        | TradeStmt::RequireAllDebtsRepaid
        | TradeStmt::AssertInvariant { .. }
        | TradeStmt::EmitReceipt => vec![],
    }
}

/// Record the trade's chain on first sighting an asset that's actually
/// declared (unresolved-asset errors are reported elsewhere), then reject
/// any later asset reference that lands on a different chain.
fn check_same_chain(
    symbol: &x3_lang_common::Symbol,
    symbols: &TradingSymbols,
    trade_chain: &mut Option<(String, x3_lang_common::Symbol)>,
    trade_name: &str,
    span: Span,
    errors: &mut Vec<X3Error>,
) {
    let Some(asset) = symbols.assets.get(symbol) else {
        return;
    };
    let chain = asset.chain.as_str().to_string();
    match trade_chain {
        None => *trade_chain = Some((chain, symbol.clone())),
        Some((expected_chain, first_symbol)) => {
            if *expected_chain != chain {
                errors.push(semantic_error(
                    format!(
                        "atomic trade '{trade_name}' mixes chains: '{}' is on '{expected_chain}' but '{}' is on '{chain}' — a plain swap/borrow is single-chain; cross-chain movement needs a bridge, not this asset reference",
                        first_symbol.as_str(),
                        symbol.as_str()
                    ),
                    span,
                ));
            }
        }
    }
}

fn deadline_is_zero(expr: &Expression) -> bool {
    match expr {
        Expression::Literal(LiteralExpr::Int { value: 0, .. }) => true,
        Expression::Literal(LiteralExpr::Int { value, .. }) => *value == 0,
        _ => false,
    }
}

/// What the author has to add to discharge a guarantee, for the diagnostic.
fn guarantee_requirement(guarantee: x3_lang_ast::TradeGuarantee) -> &'static str {
    match guarantee {
        x3_lang_ast::TradeGuarantee::DebtClosed => "add `require all_debts_repaid`",
        x3_lang_ast::TradeGuarantee::MinProfit => "add `require net_profit >= <amount>`",
        x3_lang_ast::TradeGuarantee::Solvent => "add `invariant solvent`",
    }
}

fn semantic_error(message: impl Into<String>, span: Span) -> X3Error {
    X3Error::SemanticError {
        message: message.into(),
        span,
    }
}

/// A declaration nothing in the body discharges, with the code the catalogue gives
/// that class: `X3E4021` unresolved economic effect. An economic diagnostic has to
/// be keyable, or a build system is matching wording (PHASE 52, TICKET-021).
fn unresolved_effect_error(message: impl Into<String>, span: Span) -> X3Error {
    crate::diagnostic::CompilerDiagnostic::error(
        crate::diagnostic::DiagnosticCode::UnresolvedEconomicEffect,
        message,
        span,
    )
    .into_error()
}
