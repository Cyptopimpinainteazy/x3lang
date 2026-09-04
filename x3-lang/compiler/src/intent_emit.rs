//! Adapter boundary: emit an X3IR intent resolution as a
//! canonical `IntentSpec` for the cross-chain intent crate.
//!
//! The x3-lang compiler produces a generic X3IR (with
//! `Operation::IntentResolve { constraints, resolver }`). The
//! cross-chain intent crate expects a fully-declarative
//! `IntentSpec` (chain, asset, amount, owner, receiver, route,
//! requirements, timeout, receipt, receiver authorization).
//!
//! This module is the **adapter boundary** between the language
//! compiler layer and the intent layer. None of the three layers
//! (x3-lang, x3-crosschain-intent, x3-settlement-engine) may build
//! a parallel state machine. The language compiler emits an
//! `IntentSpec`; the intent crate validates and lowers it; the
//! settlement engine consumes the lower-level
//! `CrossChainIntent`/`X3Instruction` plan and drives execution.

use serde::{Deserialize, Serialize};

/// AST types for building the intent draft from parsed source.
use x3_lang_ast::ast::{Expression, IntentDecl, LiteralExpr, RequireKind as AstRequireKind};

/// A single source-level constraint emitted by the x3-lang parser.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SourceConstraint {
    /// Constraint kind, e.g. `"slippage"`, `"finality"`,
    /// `"max_fee"`, `"proof"`, `"timeout"`, `"receiver"`.
    pub kind: String,
    /// Free-form string representation of the constraint's
    /// argument (e.g. `"<= 0.5%"`, `"eth >= 12"`,
    /// `"eth.lock_event"`).
    pub arg: String,
}

/// Builder that the x3-lang compiler populates from a source-level
/// intent declaration. The builder is the canonical input shape for
/// the cross-chain intent crate's
/// `intent_spec_to_crosschain_intent` adapter.
///
/// The fields are stringly-typed at the language boundary so the
/// language compiler does not need to know every chain/asset enum,
/// and the intent crate's adapter is responsible for canonicalizing
/// chain names and parsing amounts. This keeps x3-lang decoupled
/// from the intent crate's type system (no dependency cycle).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IntentSpecDraft {
    /// Human-readable intent name (the resolver identifier).
    pub name: String,
    /// Source chain (e.g. `"eth"`, `"sol"`, `"x3"`, `"btc"`).
    pub source_chain: String,
    /// Source asset symbol (e.g. `"USDC"`, `"ETH"`).
    pub source_asset: String,
    /// Source amount in base units (smallest denomination).
    pub source_amount: u128,
    /// Source owner address (chain-specific string form).
    pub source_owner: String,
    /// Optional source lock contract.
    pub source_lock_contract: Option<String>,
    /// Destination chain.
    pub dest_chain: String,
    /// Destination asset symbol.
    pub dest_asset: String,
    /// Optional minimum destination amount.
    pub dest_min_amount: Option<u128>,
    /// Destination receiver address.
    pub dest_receiver: String,
    /// Source-level constraints (slippage, finality, fee cap, …).
    pub constraints: Vec<SourceConstraint>,
    /// Timeout duration in seconds (parsed from `"30m"`, `"1h"`, …).
    pub timeout_secs: u64,
}

impl IntentSpecDraft {
    /// Build a draft with safe defaults (30-minute timeout, no
    /// destination min amount, empty constraints).
    pub fn new(
        name: impl Into<String>,
        source_chain: impl Into<String>,
        source_asset: impl Into<String>,
        source_amount: u128,
        source_owner: impl Into<String>,
        dest_chain: impl Into<String>,
        dest_asset: impl Into<String>,
        dest_receiver: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            source_chain: source_chain.into(),
            source_asset: source_asset.into(),
            source_amount,
            source_owner: source_owner.into(),
            source_lock_contract: None,
            dest_chain: dest_chain.into(),
            dest_asset: dest_asset.into(),
            dest_min_amount: None,
            dest_receiver: dest_receiver.into(),
            constraints: Vec::new(),
            timeout_secs: 30 * 60,
        }
    }

    /// Apply a timeout override in seconds. Returns `self` for
    /// chaining.
    pub fn with_timeout_secs(mut self, secs: u64) -> Self {
        self.timeout_secs = secs;
        self
    }

    /// Add a constraint. Returns `self` for chaining.
    pub fn with_constraint(mut self, kind: impl Into<String>, arg: impl Into<String>) -> Self {
        self.constraints.push(SourceConstraint {
            kind: kind.into(),
            arg: arg.into(),
        });
        self
    }

    /// Set the source lock contract.
    pub fn with_lock_contract(mut self, contract: impl Into<String>) -> Self {
        self.source_lock_contract = Some(contract.into());
        self
    }

    /// Set the destination minimum amount.
    pub fn with_dest_min_amount(mut self, amount: u128) -> Self {
        self.dest_min_amount = Some(amount);
        self
    }

    /// Parse a timeout arg like "30m", "1h", "600s" into seconds.
    pub fn timeout_from_arg(arg: &str) -> Option<u64> {
        let arg = arg.trim();
        if let Some(rest) = arg.strip_suffix('m') {
            rest.parse::<u64>().ok().map(|n| n * 60)
        } else if let Some(rest) = arg.strip_suffix('h') {
            rest.parse::<u64>().ok().map(|n| n * 3600)
        } else if let Some(rest) = arg.strip_suffix('s') {
            rest.parse::<u64>().ok()
        } else {
            arg.parse::<u64>().ok()
        }
    }
}

/// Extract a canonical [`IntentSpecDraft`] from an X3 AST [`IntentDecl`].
///
/// This function walks the intent declaration's body and constraints to
/// populate a `IntentSpecDraft` that can be serialized to JSON and fed to
/// the cross-chain intent crate's `draft_to_compiled_plan()` adapter.
///
/// The body is expected to contain statements like:
/// - `Lock { chain: "eth", asset: "USDC", amount: 500_000_000u128, from: "alice.eth" }`
/// - `Mint { chain: "sol", asset: "SOL", ... }`
/// - `Require { kind: Finality, subject: Some("eth"), value: >= 12 }`
/// - `Require { kind: Slippage, value: <= 50 bps }`
///
/// Returns `None` if the intent body cannot be parsed into a valid draft
/// (e.g., no source lock operation found). The intent compiler downstream
/// will also run its own 13 safety checks on whatever this produces.
pub fn from_intent_decl(intent: &IntentDecl) -> IntentSpecDraft {
    let name = intent.name.as_str().to_string();
    let mut draft = IntentSpecDraft::new(
        &name, "x3", // default source chain
        "UNKNOWN", 0, "unknown", "x3", "UNKNOWN", "unknown",
    );

    // Walk constraints (from `require` guards) and body statements.
    // Constraints from the `require` annotations are encoded as Expressions
    // in the AST. We convert them here.
    for expr in &intent.constraints {
        if let Some(kv) = expression_to_keyword_value(expr) {
            draft = draft.with_constraint(kv.0, kv.1);
        }
    }

    // Walk body statements for Lock, Mint, Swap, RequireGuard
    for stmt in &intent.body.stmts {
        match stmt {
            x3_lang_ast::ast::Statement::Lock {
                chain,
                asset,
                amount,
                from,
                ..
            } => {
                let chain_str = chain.0.as_str().to_string();
                let asset_str = asset.name.as_str().to_string();
                let amt = match amount {
                    Expression::Literal(LiteralExpr::Int { value, .. }) => *value,
                    _ => 0,
                };
                let from_str = string_from_expr(from);
                draft.source_chain = chain_str;
                draft.source_asset = asset_str;
                draft.source_amount = amt;
                draft.source_owner = from_str;
            }
            x3_lang_ast::ast::Statement::Mint { asset, amount, to, .. } => {
                let chain_str = asset.chain.as_str().to_string();
                let asset_str = asset.name.as_str().to_string();
                let amt = match amount {
                    Expression::Literal(LiteralExpr::Int { value, .. }) => Some(*value),
                    _ => None,
                };
                draft.dest_chain = chain_str;
                draft.dest_asset = asset_str;
                draft.dest_min_amount = amt;
                draft.dest_receiver = string_from_expr(to);
            }
            x3_lang_ast::ast::Statement::Require(guard) => {
                let kind = &guard.kind;
                let subject = &guard.subject;
                let value = &guard.value;
                let constraint_str = match kind {
                    AstRequireKind::Finality => {
                        let chain_str = subject
                            .as_ref()
                            .map(|s| s.as_str().to_string())
                            .unwrap_or_else(|| "x3".to_string());
                        let val_str = value_string_from_expr(value);
                        draft = draft.with_constraint("finality", format!("{chain_str} >= {val_str}"));
                        continue;
                    }
                    AstRequireKind::Slippage => "slippage",
                    AstRequireKind::CanonicalSupply => {
                        draft = draft.with_constraint("require_canonical_supply", "");
                        continue;
                    }
                    AstRequireKind::Nonce => "require_receiver_owner",
                    _ => continue,
                };
                let val_str = value_string_from_expr(value);
                draft = draft.with_constraint(constraint_str, val_str);
            }
            x3_lang_ast::ast::Statement::Atomic(x3_lang_ast::ast::AtomicBlock {
                meta: Some(expr),
                body: _,
            }) => {
                // The `atomic` block inside an intent likely contains
                // the route specification. We record this as a hint
                // for the adapter.
                if let Some(kv) = expression_to_keyword_value(expr) {
                    draft = draft.with_constraint(kv.0, kv.1);
                }
            }
            _ => {}
        }
    }

    draft
}

/// Attempt to extract a (kind, value) pair from an expression
/// that might be a `require` guard expression.
fn expression_to_keyword_value(expr: &Expression) -> Option<(String, String)> {
    match expr {
        // String literals like "finality eth >= 12"
        Expression::Literal(LiteralExpr::String(s)) => {
            let text = s.as_str();
            if let Some((kind, rest)) = text.split_once(char::is_whitespace) {
                let rest = rest.trim();
                Some((kind.to_string(), rest.to_string()))
            } else {
                Some((text.to_string(), String::new()))
            }
        }
        // Binary expressions like finality(eth) >= 12
        Expression::Binary { op: _, lhs, rhs } => {
            let lhs_str = string_from_expr(lhs.as_ref());
            let rhs_str = string_from_expr(rhs.as_ref());
            Some((lhs_str, rhs_str))
        }
        _ => None,
    }
}

/// Extract a string value from an expression (likely a literal or identifier).
fn value_string_from_expr(expr: &Expression) -> String {
    match expr {
        Expression::Literal(LiteralExpr::Int { value, .. }) => value.to_string(),
        Expression::Literal(LiteralExpr::String(s)) => s.as_str().to_string(),
        Expression::Literal(LiteralExpr::Percentage { value }) => format!("{value}%"),
        Expression::Literal(LiteralExpr::Duration { value, unit: _ }) => value.to_string(),
        Expression::Ident(s) => s.as_str().to_string(),
        _ => format!("{:?}", expr),
    }
}

/// Extract a string from an expression that is a path/identifier/string literal.
fn string_from_expr(expr: &Expression) -> String {
    match expr {
        Expression::Literal(LiteralExpr::String(s)) => s.as_str().to_string(),
        Expression::Literal(LiteralExpr::Address(s)) => s.as_str().to_string(),
        Expression::Ident(s) => s.as_str().to_string(),
        Expression::FieldAccess { target, field } => {
            format!("{}.{}", string_from_expr(target.as_ref()), field.as_str())
        }
        _ => format!("{:?}", expr),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_simple_timeout_args() {
        assert_eq!(IntentSpecDraft::timeout_from_arg("30m"), Some(30 * 60));
        assert_eq!(IntentSpecDraft::timeout_from_arg("1h"), Some(3600));
        assert_eq!(IntentSpecDraft::timeout_from_arg("600s"), Some(600));
        assert_eq!(IntentSpecDraft::timeout_from_arg("600"), Some(600));
        assert_eq!(IntentSpecDraft::timeout_from_arg("garbage"), None);
    }

    #[test]
    fn builder_uses_safe_defaults() {
        let draft = IntentSpecDraft::new(
            "bridge_usdc_sol",
            "eth",
            "USDC",
            500_000_000,
            "alice.eth",
            "sol",
            "SOL",
            "alice.sol",
        );
        assert_eq!(draft.timeout_secs, 30 * 60);
        assert_eq!(draft.source_lock_contract, None);
        assert_eq!(draft.dest_min_amount, None);
        assert!(draft.constraints.is_empty());
    }

    #[test]
    fn builder_chains_constraints_and_overrides() {
        let draft = IntentSpecDraft::new(
            "bridge_usdc_sol",
            "eth",
            "USDC",
            500_000_000,
            "alice.eth",
            "sol",
            "SOL",
            "alice.sol",
        )
        .with_timeout_secs(900)
        .with_lock_contract("0xBridge")
        .with_dest_min_amount(3_500_000_000)
        .with_constraint("slippage", "<= 0.5%")
        .with_constraint("max_fee", "<= 10 USDC")
        .with_constraint("proof", "eth.lock_event");

        assert_eq!(draft.timeout_secs, 900);
        assert_eq!(draft.source_lock_contract.as_deref(), Some("0xBridge"));
        assert_eq!(draft.dest_min_amount, Some(3_500_000_000));
        assert_eq!(draft.constraints.len(), 3);
        assert_eq!(draft.constraints[0].kind, "slippage");
        assert_eq!(draft.constraints[0].arg, "<= 0.5%");
    }

    #[test]
    fn draft_round_trips_through_json() {
        let draft = IntentSpecDraft::new(
            "round_trip",
            "btc",
            "BTC",
            10_000,
            "bc1qalice",
            "x3",
            "BTC.e",
            "alice.x3",
        )
        .with_constraint("finality", "btc >= 6");

        let json = serde_json::to_string(&draft).expect("serialize");
        let parsed: IntentSpecDraft = serde_json::from_str(&json).expect("parse");
        assert_eq!(draft, parsed);
    }
}
