//! Trading Core v1 AST nodes.
//!
//! These declarations model typed assets, risk policies, and atomic trades.
//! Amounts deliberately carry lossless [`Expression`] values: source literals
//! are converted to base units during semantic analysis only when the
//! conversion is exact. Assets with different VM families, chains, canonical
//! identifiers, or decimals are distinct types.

use serde::{Deserialize, Serialize};
use x3_lang_common::{IntBase, Symbol};

use crate::ast::{ChainRef, Expression, LiteralExpr};

/// Full, chain-qualified identity of a tradeable asset.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AssetId {
    /// VM family that hosts the asset (for example `"evm"` or `"svm"`).
    pub vm_family: Symbol,
    /// Chain the canonical identifier is valid on.
    pub chain: ChainRef,
    /// Canonical identifier recognized by the target VM/chain.
    pub canonical_id: Symbol,
    /// Human-facing asset symbol (for example `"USDC"`). Never used for
    /// equivalence; [`AssetId`] fields define asset identity.
    pub symbol: Symbol,
    /// Decimal places used when converting to and from base units.
    pub decimals: u8,
}

/// Linear debt obligation created by a `borrow ... as <debt>` statement.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DebtId(pub Symbol);

impl PartialOrd for DebtId {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for DebtId {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.as_str().cmp(other.0.as_str())
    }
}

/// Explicit rounding direction for amount arithmetic that can lose precision.
///
/// Defined in `x3-lang-common::fixed` and re-exported here, because the arithmetic that
/// uses it lives below the AST and one vocabulary is the point: a value that rounds "down"
/// in an asset conversion and "down" in a basis-point computation have to mean the same
/// thing (PHASE 43).
pub use x3_lang_common::fixed::RoundingMode;

/// A typed amount expressed against a symbolic asset.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AmountExpr {
    /// Lossless source expression; literal decimal conversion happens later.
    pub value: Expression,
    /// Symbolic reference to the declared asset carrying the amount.
    pub asset: Symbol,
}

impl AmountExpr {
    /// Build an amount from an unsigned decimal literal in base units.
    ///
    /// The integer is preserved verbatim inside [`Expression::Literal`] so no
    /// precision is lost before semantic decimal conversion.
    pub fn literal(value: u128, asset: Symbol) -> Self {
        AmountExpr {
            value: Expression::Literal(LiteralExpr::Int {
                value,
                base: IntBase::Decimal,
                suffix: None,
            }),
            asset,
        }
    }
}

/// `asset NAME = vm.chain.canonical_id { ... }` declaration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetDecl {
    pub name: Symbol,
    pub asset: AssetId,
}

/// Named, reusable trading risk policy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeRiskPolicy {
    pub name: Symbol,
    /// Maximum acceptable slippage measured in basis points.
    pub max_slippage_bps: u16,
    /// Maximum gas/execution cost the trade may commit.
    pub max_gas: AmountExpr,
    /// Maximum flash-liquidity fee measured in basis points.
    pub max_flash_fee_bps: u16,
    /// Deadline bound in the trade's declared clock domain.
    pub deadline: Expression,
    /// Whether a private-submission capability is mandatory.
    pub require_private_submission: bool,
    /// Optional policy-wide minimum net profit.
    pub min_profit: Option<AmountExpr>,
    /// Optional oracle-firewall ceiling: maximum allowed disagreement (in
    /// basis points) between the venue's primary quote and any other
    /// independent price source the host reports. `None` means no
    /// cross-source check is required — the historical default, so
    /// existing policies that don't declare this stay unaffected.
    pub max_oracle_deviation_bps: Option<u16>,
    /// Optional circuit breaker on realized losses accumulated across every
    /// trade a single VM instance has committed under this policy's asset,
    /// not just this one trade. `None` means no cross-trade ceiling is
    /// enforced — the historical default.
    pub max_cumulative_loss: Option<AmountExpr>,
    /// Optional quote-freshness ceiling: the maximum age, in blocks, of the
    /// venue quote a swap may be priced from. `None` means the policy requires
    /// no freshness bound — the same opt-in shape as
    /// `max_oracle_deviation_bps`, so policies that do not declare it are
    /// unaffected.
    pub quote_freshness: Option<u64>,
}

/// `atomic trade NAME using POLICY { ... }` declaration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AtomicTradeDecl {
    pub name: Symbol,
    pub risk_policy: Symbol,
    /// Economic effects this body claims to produce, from
    /// `effects [borrow, swap, repay]`. Each one must be realized by a matching
    /// statement or the trade is rejected — an effect label the compiler cannot
    /// check would read like a guarantee while meaning nothing.
    #[serde(default)]
    pub effects: Vec<TradeEffect>,
    /// Guarantees this body promises to discharge before it may commit, from
    /// `guarantees [debt_closed, min_profit]`. Each one must be discharged by a
    /// matching guard in the body, for the same reason.
    #[serde(default)]
    pub guarantees: Vec<TradeGuarantee>,
    pub body: Vec<TradeStmt>,
}

/// An economic effect a trade body claims to produce.
///
/// Deliberately closed, like `InvariantKind`: an unrecognized name is a parse
/// error, not a silently-accepted label.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TradeEffect {
    /// Take on debt that must later be repaid.
    Borrow,
    /// Convert one asset into another through a venue.
    Swap,
    /// Move value to another chain.
    Bridge,
    /// Close a previously opened debt.
    Repay,
}

impl TradeEffect {
    /// Every effect the language knows, in a fixed order for diagnostics.
    pub const ALL: [TradeEffect; 4] = [
        TradeEffect::Borrow,
        TradeEffect::Swap,
        TradeEffect::Bridge,
        TradeEffect::Repay,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            TradeEffect::Borrow => "borrow",
            TradeEffect::Swap => "swap",
            TradeEffect::Bridge => "bridge",
            TradeEffect::Repay => "repay",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "borrow" => Some(TradeEffect::Borrow),
            "swap" => Some(TradeEffect::Swap),
            "bridge" => Some(TradeEffect::Bridge),
            "repay" => Some(TradeEffect::Repay),
            _ => None,
        }
    }

    /// Whether `stmt` produces this effect.
    pub fn is_produced_by(self, stmt: &TradeStmt) -> bool {
        match self {
            TradeEffect::Borrow => matches!(stmt, TradeStmt::Borrow { .. }),
            TradeEffect::Swap => matches!(stmt, TradeStmt::Swap { .. }),
            TradeEffect::Bridge => matches!(stmt, TradeStmt::Bridge { .. }),
            TradeEffect::Repay => matches!(stmt, TradeStmt::Repay { .. }),
        }
    }
}

/// A guarantee a trade body promises before it is allowed to commit.
///
/// Closed like `TradeEffect`, and restricted to guarantees the compiler can
/// actually discharge from the body: a name whose enforcement would be a no-op
/// is not admitted, for the same reason the unenforceable policy ceilings were
/// removed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TradeGuarantee {
    /// Every borrowed debt is closed exactly once.
    DebtClosed,
    /// A minimum net profit floor is asserted.
    MinProfit,
    /// The solvent invariant is asserted.
    Solvent,
}

impl TradeGuarantee {
    /// Every guarantee the language knows, in a fixed order for diagnostics.
    pub const ALL: [TradeGuarantee; 3] = [
        TradeGuarantee::DebtClosed,
        TradeGuarantee::MinProfit,
        TradeGuarantee::Solvent,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            TradeGuarantee::DebtClosed => "debt_closed",
            TradeGuarantee::MinProfit => "min_profit",
            TradeGuarantee::Solvent => "solvent",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "debt_closed" => Some(TradeGuarantee::DebtClosed),
            "min_profit" => Some(TradeGuarantee::MinProfit),
            "solvent" => Some(TradeGuarantee::Solvent),
            _ => None,
        }
    }

    /// Whether `stmt` discharges this guarantee in a trade body.
    pub fn is_discharged_by(self, stmt: &TradeStmt) -> bool {
        match self {
            TradeGuarantee::DebtClosed => matches!(stmt, TradeStmt::RequireAllDebtsRepaid),
            TradeGuarantee::MinProfit => matches!(stmt, TradeStmt::RequireMinNetProfit { .. }),
            TradeGuarantee::Solvent => matches!(
                stmt,
                TradeStmt::AssertInvariant {
                    kind: InvariantKind::Solvent
                }
            ),
        }
    }
}

/// A statement inside an atomic trade body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TradeStmt {
    Borrow {
        amount: AmountExpr,
        provider: Symbol,
        debt: DebtId,
    },
    Swap {
        binding: Symbol,
        input: AmountExpr,
        from_asset: Symbol,
        to_asset: Symbol,
        venue: Symbol,
        min_output: AmountExpr,
    },
    Repay {
        debt: DebtId,
    },
    /// Move a settled amount to another chain through a bridge — the one
    /// place a trade is allowed to cross chains at all. Unlike `Swap`,
    /// there is no `min_output`: a bridge transfer is proven by a
    /// cryptographic inclusion/finality proof at settlement time, not
    /// subject to venue-side slippage the way a DEX quote is.
    Bridge {
        input: AmountExpr,
        from_asset: Symbol,
        to_asset: Symbol,
        via: Symbol,
        /// Destination-chain receiver address. Must be a string literal —
        /// deliberately not a dynamic binding, since a bridge destination
        /// should be an explicit, reviewable part of the trade's source,
        /// not a value that could be swapped out by upstream state.
        receiver: Expression,
    },
    RequireMinNetProfit {
        amount: AmountExpr,
    },
    RequireAllDebtsRepaid,
    AssertInvariant {
        kind: InvariantKind,
    },
    EmitReceipt,
}

/// A named, formally-checked economic invariant. Deliberately closed: an
/// unrecognized invariant name is a parse error, not a silently-accepted
/// no-op — see `parser.rs`'s `parse_invariant_kind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum InvariantKind {
    /// Every asset touched by the trade nets to a non-negative delta at
    /// commit time — the trade never ends up owing more of any asset than
    /// it received. Broader than the settlement-asset profit floor: it
    /// covers every asset the trade touched, not just the one named in
    /// `require net_profit`.
    Solvent,
}

impl InvariantKind {
    pub fn as_str(self) -> &'static str {
        match self {
            InvariantKind::Solvent => "solvent",
        }
    }
}
