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
use x3_lang_ast::ast::{
    ComparisonOp, Expression, FailureAction, IntentDecl, LiteralExpr, RequireKind as AstRequireKind, Statement,
};

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
    /// Owner whose externally enforced mandate authorizes this adapter intent.
    ///
    /// This is not interpreted by the generic VM. A resolver that consumes the
    /// draft must either enforce it or reject the draft.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mandate_owner: Option<String>,
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
            mandate_owner: None,
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
            rest.parse::<u64>().ok().and_then(|n| n.checked_mul(60))
        } else if let Some(rest) = arg.strip_suffix('h') {
            rest.parse::<u64>().ok().and_then(|n| n.checked_mul(3600))
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
/// This is a draft, not an authorization or settlement proof. Callers must
/// validate source semantics before export; downstream adapters must enforce
/// the constraints against their actual execution context.
pub fn from_intent_decl(intent: &IntentDecl) -> IntentSpecDraft {
    let name = intent.name.as_str().to_string();
    let mut draft = IntentSpecDraft::new(
        &name, "x3", // default source chain
        "UNKNOWN", 0, "unknown", "x3", "UNKNOWN", "unknown",
    );
    draft.mandate_owner = intent.mandate.as_ref().map(|owner| owner.as_str().to_string());

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
            // The `to <chain>.<ASSET> receiver <x>` clause lowers to a `Release`, and this walker
            // only read `Mint`. So an intent whose route is a bridge — which is every cross-chain
            // intent in the corpus — kept the draft's hardcoded placeholders for its own
            // destination: measured on `examples/simple_swap.x3`, whose `to solana.SOL receiver
            // wallet` came back as `dest_chain: "x3"`, `dest_asset: "UNKNOWN"`, `dest_receiver:
            // "unknown"`. The compiler knew the destination (the artifact released to solana.SOL);
            // only the intent spec did not. TICKET-129.
            x3_lang_ast::ast::Statement::Release { chain, asset, to } => {
                draft.dest_chain = chain.0.as_str().to_string();
                draft.dest_asset = asset.name.as_str().to_string();
                draft.dest_receiver = string_from_expr(to);
            }
            x3_lang_ast::ast::Statement::Require(guard) => {
                let kind = &guard.kind;
                let subject = &guard.subject;
                let constraint_str = match kind {
                    AstRequireKind::Finality => {
                        let chain_str = subject
                            .as_ref()
                            .map(|s| s.as_str().to_string())
                            .unwrap_or_else(|| "x3".to_string());
                        let val_str = guard_value_string(guard);
                        draft = draft.with_constraint("finality", format!("{chain_str} {val_str}"));
                        continue;
                    }
                    AstRequireKind::Slippage => "slippage",
                    AstRequireKind::Fees => "fees",
                    AstRequireKind::CanonicalSupply => {
                        draft = draft.with_constraint("require_canonical_supply", guard_value_string(guard));
                        continue;
                    }
                    AstRequireKind::Nonce => {
                        let status = subject.as_ref().map(|s| s.as_str()).unwrap_or("unused");
                        draft = draft.with_constraint("nonce", format!("{status} {}", guard_value_string(guard)));
                        continue;
                    }
                    _ => {
                        let value = guard_value_string(guard);
                        let argument = match (&guard.subject, &guard.value) {
                            (Some(subject), Some(_)) => format!("{} {value}", subject.as_str()),
                            _ => value,
                        };
                        draft = draft.with_constraint(guard.kind.as_str(), argument);
                        continue;
                    }
                };
                let val_str = guard_value_string(guard);
                draft = draft.with_constraint(constraint_str, val_str);
            }
            Statement::OnTimeout { duration, action } => {
                // An explicit unsupported/overflowing duration is invalid (zero),
                // never the unrelated builder default. The CLI checks diagnostics
                // before exporting any draft.
                draft.timeout_secs = timeout_seconds(duration).unwrap_or(0);
                if let FailureAction::Refund(receiver) = action {
                    draft = draft.with_constraint("refund_path", string_from_expr(receiver));
                }
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

    collect_destination_floor(&intent.body.stmts, &mut draft);
    for constraint in &mut draft.constraints {
        if constraint.kind == "refund_path" {
            if let Some(asset) = constraint.arg.strip_suffix(":sender") {
                constraint.arg = format!("{asset}:{}", draft.source_owner);
            }
        }
    }
    draft
}

fn collect_destination_floor(statements: &[Statement], draft: &mut IntentSpecDraft) {
    fn terminal(statement: &Statement) -> Option<&Statement> {
        match statement {
            Statement::Atomic(block) => block.body.stmts.last().and_then(terminal),
            other => Some(other),
        }
    }
    // A route may visit the destination asset and leave it again. Only its
    // terminal bridge can describe the final payout; intermediate floors do not.
    let last = statements
        .iter()
        .filter_map(|statement| match statement {
            Statement::Atomic(_) => terminal(statement),
            _ => None,
        })
        .next_back();
    if let Some(Statement::Bridge {
        to,
        min_receive: Some(Expression::Literal(LiteralExpr::Int { value, .. })),
        ..
    }) = last
    {
        if to.chain.as_str().eq_ignore_ascii_case(&draft.dest_chain) && to.name.as_str() == draft.dest_asset {
            draft.dest_min_amount = Some(*value);
        }
    }
}

fn timeout_seconds(expression: &Expression) -> Option<u64> {
    use x3_lang_common::DurationUnit;
    match expression {
        Expression::Literal(LiteralExpr::Int { value, .. }) => u64::try_from(*value)
            .ok()?
            .checked_mul(crate::lowering::SECONDS_PER_BLOCK),
        Expression::Literal(LiteralExpr::Duration { value, unit }) => match unit {
            DurationUnit::Nanoseconds => Some(value.div_ceil(1_000_000_000)),
            DurationUnit::Microseconds => Some(value.div_ceil(1_000_000)),
            DurationUnit::Milliseconds => Some(value.div_ceil(1_000)),
            DurationUnit::Seconds => Some(*value),
            DurationUnit::Minutes => value.checked_mul(60),
            DurationUnit::Hours => value.checked_mul(3600),
            DurationUnit::Days => value.checked_mul(86400),
        },
        _ => None,
    }
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
/// The value a guard states, or the name it asserts when it states none.
///
/// A property guard has no right-hand side: `require canonical_supply USDC`
/// says something about USDC and nothing about a number, so the name is what the
/// emitted constraint carries.
fn guard_value_string(guard: &x3_lang_ast::ast::RequireGuard) -> String {
    if let Some(value) = &guard.value {
        let mut rendered = value_string_from_expr(value);
        if matches!(guard.kind, AstRequireKind::Fees | AstRequireKind::Slippage)
            && matches!(value, Expression::Literal(LiteralExpr::Int { .. }))
        {
            rendered.push_str(" bps");
        }
        let comparison = match guard.comparison {
            Some(ComparisonOp::GreaterOrEqual) => ">=",
            Some(ComparisonOp::Greater) => ">",
            Some(ComparisonOp::LessOrEqual) => "<=",
            Some(ComparisonOp::Less) => "<",
            Some(ComparisonOp::Equal) => "==",
            Some(ComparisonOp::NotEqual) => "!=",
            None => "",
        };
        return if comparison.is_empty() {
            rendered
        } else {
            format!("{comparison} {rendered}")
        };
    }
    guard
        .subject
        .as_ref()
        .map(|subject| subject.as_str().to_string())
        .unwrap_or_default()
}

fn value_string_from_expr(expr: &Expression) -> String {
    match expr {
        Expression::Literal(LiteralExpr::Int { value, .. }) => value.to_string(),
        Expression::Literal(LiteralExpr::String(s)) => s.as_str().to_string(),
        Expression::Literal(LiteralExpr::Percentage { value }) => value.as_str().to_string(),
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
