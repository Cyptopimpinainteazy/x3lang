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

/// Explicit rounding direction for amount arithmetic that can lose precision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RoundingMode {
    Down,
    Up,
    Exact,
}

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
}

/// `atomic trade NAME using POLICY { ... }` declaration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AtomicTradeDecl {
    pub name: Symbol,
    pub risk_policy: Symbol,
    pub body: Vec<TradeStmt>,
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
    RequireMinNetProfit {
        amount: AmountExpr,
    },
    RequireAllDebtsRepaid,
    EmitReceipt,
}
