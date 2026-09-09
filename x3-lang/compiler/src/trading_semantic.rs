//! Trading Core v1 semantic analysis.
//!
//! Resolves declared assets and risk policies, enforces asset identity and
//! decimal rules, converts human-unit literals to checked base units, and
//! validates the typed statements of every atomic trade before later
//! lowering or verification can proceed.

use std::collections::HashMap;
use std::error::Error;
use std::fmt;

use indexmap::IndexMap;
use x3_lang_ast::ast::{Expression, Item, LiteralExpr, Program};
use x3_lang_ast::{AmountExpr, AssetId, AtomicTradeDecl, DebtId, RoundingMode, TradeRiskPolicy, TradeStmt};
use x3_lang_common::{Span, Symbol, X3Error};

use crate::semantic::CompilationMode;

/// Registered trading symbols from the top-level declarations of a program.
#[derive(Debug, Clone, Default)]
pub struct TradingSymbols {
    pub assets: IndexMap<Symbol, AssetId>,
    pub policies: IndexMap<Symbol, TradeRiskPolicy>,
}

/// A compile-time typed amount in the base units of its asset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypedAmount {
    pub base_units: u128,
    pub asset: AssetId,
}

/// Errors produced by decimal-to-base-unit conversion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TradingTypeError {
    InvalidLiteral(String),
    PrecisionLoss,
    Overflow,
    InvalidDecimals(u8),
}

impl fmt::Display for TradingTypeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLiteral(raw) => write!(f, "invalid decimal literal '{raw}'"),
            Self::PrecisionLoss => write!(f, "decimal literal loses precision for the asset decimals"),
            Self::Overflow => write!(f, "decimal literal overflows u128 base units"),
            Self::InvalidDecimals(decimals) => {
                write!(f, "asset decimals {decimals} exceed the supported maximum of 38")
            }
        }
    }
}

impl Error for TradingTypeError {}

const MAX_DECIMALS: u8 = 38;

/// Analyze the trading declarations in a parsed program.
///
/// The mode is accepted so mainnet-specific checks can be layered on later
/// without changing the public entry point.
pub fn analyze_trading(program: &Program, _mode: CompilationMode) -> Result<TradingSymbols, Vec<X3Error>> {
    let mut symbols = TradingSymbols::default();
    let mut errors = Vec::new();

    for item in &program.items {
        match &item.node {
            Item::AssetDecl(decl) => {
                if decl.asset.decimals > MAX_DECIMALS {
                    errors.push(semantic_error(format!(
                        "asset '{}' declares {} decimals; maximum is {MAX_DECIMALS}",
                        decl.name.as_str(),
                        decl.asset.decimals
                    )));
                }
                if decl.asset.vm_family.as_str().is_empty()
                    || decl.asset.chain.as_str().is_empty()
                    || decl.asset.canonical_id.as_str().is_empty()
                {
                    errors.push(semantic_error(format!(
                        "asset '{}' must declare a non-empty vm family, chain, and canonical identifier",
                        decl.name.as_str()
                    )));
                }
                if symbols.assets.insert(decl.name.clone(), decl.asset.clone()).is_some() {
                    errors.push(semantic_error(format!(
                        "duplicate asset declaration '{}'",
                        decl.name.as_str()
                    )));
                }
            }
            Item::TradeRiskPolicy(policy) => {
                validate_policy_asset(&policy, &symbols.assets, &mut errors);
                if symbols.policies.insert(policy.name.clone(), policy.clone()).is_some() {
                    errors.push(semantic_error(format!(
                        "duplicate trading risk policy '{}'",
                        policy.name.as_str()
                    )));
                }
            }
            _ => {}
        }
    }

    for item in &program.items {
        if let Item::AtomicTrade(trade) = &item.node {
            validate_atomic_trade(trade, &symbols, &mut errors);
        }
    }

    if errors.is_empty() {
        Ok(symbols)
    } else {
        Err(errors)
    }
}

fn validate_policy_asset(policy: &TradeRiskPolicy, assets: &IndexMap<Symbol, AssetId>, errors: &mut Vec<X3Error>) {
    for (field, amount) in [
        ("max_gas", Some(&policy.max_gas)),
        ("min_profit", policy.min_profit.as_ref()),
    ] {
        if let Some(amount) = amount {
            validate_amount(amount, assets, field, errors);
        }
    }
    if policy.max_slippage_bps > 10_000 {
        errors.push(semantic_error(format!(
            "risk policy '{}' has max_slippage above 10000 bps",
            policy.name.as_str()
        )));
    }
    if policy.max_flash_fee_bps > 10_000 {
        errors.push(semantic_error(format!(
            "risk policy '{}' has max_flash_fee above 10000 bps",
            policy.name.as_str()
        )));
    }
}

fn validate_atomic_trade(trade: &AtomicTradeDecl, symbols: &TradingSymbols, errors: &mut Vec<X3Error>) {
    if !symbols.policies.contains_key(&trade.risk_policy) {
        errors.push(semantic_error(format!(
            "atomic trade '{}' references unknown risk policy '{}'",
            trade.name.as_str(),
            trade.risk_policy.as_str()
        )));
    }

    // debt name -> asset symbol; binding name -> asset symbol
    let mut debts: HashMap<Symbol, Symbol> = HashMap::new();
    let mut declared_debts: HashMap<Symbol, Symbol> = HashMap::new();
    let mut bindings: HashMap<Symbol, Symbol> = HashMap::new();

    for stmt in &trade.body {
        match stmt {
            TradeStmt::Borrow { amount, debt, .. } => {
                validate_amount(amount, &symbols.assets, "borrow amount", errors);
                if declared_debts.insert(debt.0.clone(), amount.asset.clone()).is_some() {
                    errors.push(semantic_error(format!(
                        "atomic trade '{}' declares debt '{}' more than once",
                        trade.name.as_str(),
                        debt.0.as_str()
                    )));
                }
            }
            TradeStmt::Swap {
                binding,
                input,
                from_asset,
                to_asset,
                min_output,
                ..
            } => {
                validate_amount(input, &symbols.assets, "swap input", errors);
                validate_amount(min_output, &symbols.assets, "swap min_out", errors);
                if input.asset != *from_asset {
                    errors.push(semantic_error(format!(
                        "atomic trade '{}' swap input asset '{}' does not match declared from_asset '{}'",
                        trade.name.as_str(),
                        input.asset.as_str(),
                        from_asset.as_str()
                    )));
                }
                if min_output.asset != *to_asset {
                    errors.push(semantic_error(format!(
                        "atomic trade '{}' swap min_out is denominated in '{}' but the destination is '{}'",
                        trade.name.as_str(),
                        min_output.asset.as_str(),
                        to_asset.as_str()
                    )));
                }
                if !symbols.assets.contains_key(from_asset) {
                    errors.push(semantic_error(format!(
                        "atomic trade '{}' uses undeclared asset '{}'",
                        trade.name.as_str(),
                        from_asset.as_str()
                    )));
                }
                if !symbols.assets.contains_key(to_asset) {
                    errors.push(semantic_error(format!(
                        "atomic trade '{}' uses undeclared asset '{}'",
                        trade.name.as_str(),
                        to_asset.as_str()
                    )));
                }
                if bindings.insert(binding.clone(), to_asset.clone()).is_some() {
                    errors.push(semantic_error(format!(
                        "atomic trade '{}' binds '{}' more than once",
                        trade.name.as_str(),
                        binding.as_str()
                    )));
                }
                if let Some(actual) = referenced_value_asset(&input.value, &bindings, &debts) {
                    if actual != *from_asset {
                        errors.push(semantic_error(format!(
                            "atomic trade '{}' swap source binding is typed '{}', not '{}'",
                            trade.name.as_str(),
                            actual.as_str(),
                            from_asset.as_str()
                        )));
                    }
                }
            }
            TradeStmt::Repay { debt } => {
                if declared_debts.contains_key(&debt.0) {
                    debts.insert(debt.0.clone(), declared_debts[&debt.0].clone());
                } else {
                    errors.push(semantic_error(format!(
                        "atomic trade '{}' repays undeclared debt '{}'",
                        trade.name.as_str(),
                        debt.0.as_str()
                    )));
                }
            }
            TradeStmt::RequireMinNetProfit { amount } => {
                validate_amount(amount, &symbols.assets, "net_profit", errors);
            }
            TradeStmt::RequireAllDebtsRepaid | TradeStmt::EmitReceipt => {}
        }
    }
}

fn referenced_value_asset(
    expr: &Expression,
    bindings: &HashMap<Symbol, Symbol>,
    debts: &HashMap<Symbol, Symbol>,
) -> Option<Symbol> {
    match expr {
        Expression::Ident(name) => bindings.get(name).cloned(),
        Expression::FieldAccess { target, field } if field.as_str() == "amount" => {
            if let Expression::Ident(owner) = target.as_ref() {
                debts.get(owner).cloned()
            } else {
                None
            }
        }
        _ => None,
    }
}

fn validate_amount(amount: &AmountExpr, assets: &IndexMap<Symbol, AssetId>, context: &str, errors: &mut Vec<X3Error>) {
    let Some(asset) = assets.get(&amount.asset) else {
        errors.push(semantic_error(format!(
            "{context} references undeclared asset '{}'",
            amount.asset.as_str()
        )));
        return;
    };
    if let Some(literal) = amount_literal_text(&amount.value) {
        if let Err(err) = decimal_to_base_units(&literal, asset.decimals, RoundingMode::Exact) {
            errors.push(semantic_error(format!("{context}: {err}")));
        }
    }
}

fn amount_literal_text(expr: &Expression) -> Option<String> {
    match expr {
        Expression::Literal(LiteralExpr::Int { value, .. }) => Some(value.to_string()),
        Expression::Literal(LiteralExpr::Float { raw, .. }) => Some(raw.as_str().to_string()),
        _ => None,
    }
}

/// Convert a human-unit decimal literal to base units.
///
/// `Exact` rejects any fractional digit beyond the asset decimals, `Down`
/// truncates them, and `Up` rounds a non-zero discarded fraction upward.
pub fn decimal_to_base_units(literal: &str, decimals: u8, rounding: RoundingMode) -> Result<u128, TradingTypeError> {
    if decimals > MAX_DECIMALS {
        return Err(TradingTypeError::InvalidDecimals(decimals));
    }
    let normalized: String = literal.chars().filter(|ch| *ch != '_').collect();
    let (whole, fraction) = match normalized.split_once('.') {
        Some((whole, fraction)) => (whole, Some(fraction)),
        None => (normalized.as_str(), None),
    };
    if whole.is_empty() || !whole.chars().all(|ch| ch.is_ascii_digit()) {
        return Err(TradingTypeError::InvalidLiteral(literal.to_string()));
    }
    let fraction = fraction.unwrap_or("");
    if !fraction.chars().all(|ch| ch.is_ascii_digit()) {
        return Err(TradingTypeError::InvalidLiteral(literal.to_string()));
    }

    let decimal_digits = decimals as usize;
    let (kept, discarded) = if fraction.len() > decimal_digits {
        fraction.split_at(decimal_digits)
    } else {
        (fraction, "")
    };
    let discarded_nonzero = discarded.chars().any(|ch| ch != '0');
    if rounding == RoundingMode::Exact && discarded_nonzero {
        return Err(TradingTypeError::PrecisionLoss);
    }

    let factor = 10u128.checked_pow(decimals as u32).ok_or(TradingTypeError::Overflow)?;
    let whole_value: u128 = whole.parse().map_err(|_| TradingTypeError::Overflow)?;
    let whole_scaled = whole_value.checked_mul(factor).ok_or(TradingTypeError::Overflow)?;

    let mut padded = kept.to_string();
    while padded.len() < decimal_digits {
        padded.push('0');
    }
    let fraction_value: u128 = if padded.is_empty() {
        0
    } else {
        padded.parse().map_err(|_| TradingTypeError::Overflow)?
    };
    if fraction_value >= factor {
        return Err(TradingTypeError::InvalidLiteral(literal.to_string()));
    }

    let mut base = whole_scaled
        .checked_add(fraction_value)
        .ok_or(TradingTypeError::Overflow)?;
    if rounding == RoundingMode::Up && discarded_nonzero {
        base = base.checked_add(1).ok_or(TradingTypeError::Overflow)?;
    }
    Ok(base)
}

fn semantic_error(message: impl Into<String>) -> X3Error {
    X3Error::SemanticError {
        message: message.into(),
        span: Span::DUMMY,
    }
}

/// Helper used by focused type tests and later verifier stages.
pub fn amount_base_units(amount: &AmountExpr, assets: &TradingSymbols) -> Result<u128, TradingTypeError> {
    let asset = assets
        .assets
        .get(&amount.asset)
        .ok_or_else(|| TradingTypeError::InvalidLiteral(amount.asset.as_str().to_string()))?;
    let literal = amount_literal_text(&amount.value)
        .ok_or_else(|| TradingTypeError::InvalidLiteral("binding amount".to_string()))?;
    decimal_to_base_units(&literal, asset.decimals, RoundingMode::Exact)
}
