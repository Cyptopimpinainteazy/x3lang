//! Profit as a first-class value — spec PHASE 5.
//!
//! "Profit must not be represented as an arbitrary integer." The language
//! already records every cost with a typed category (`CostKind`) and a signed
//! per-asset delta, but the two were never assembled into a value: the profit
//! floor was checked against a bare `u128` clamped at zero, so a loss read as
//! zero profit and the decomposition the spec asks for existed only as a list of
//! ledger entries.
//!
//! This type is that decomposition. Three decisions carry it:
//!
//! - **One field per cost kind, exhaustively.** The fields are named for the
//!   kinds the language defines rather than for a taxonomy invented here, and a
//!   new `CostKind` breaks the build until it has a field. A grouping that hid
//!   two kinds in one number would be the same defect one level up.
//! - **A cost kind this type does not know is refused, not omitted.** Silently
//!   dropping an unrecognised category would make `net` larger than the trade
//!   earned, which is the worst possible direction for an error in a profit
//!   figure.
//! - **`net` is signed.** A loss is a loss. The signed delta was already in the
//!   state; clamping it at zero in the accessor is what let a losing trade
//!   satisfy a floor of zero.
//!
//! Two of the spec's named quantities do not exist here, and the type says so
//! instead of carrying a zero that pretends otherwise: `hedging_costs` (no
//! hedging is modelled) and `unrealized` (which needs a price the language does
//! not have). `realized` is this value, because everything in the ledger has
//! settled.

use serde::{Deserialize, Serialize};

use x3_lang_compiler::ir::CostKind;

/// A cost the ledger recorded under a category this type does not know.
///
/// Refused rather than bucketed: an unknown category is a cost that would
/// otherwise vanish from the sum, and a profit figure that is too large is worse
/// than one that is missing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownCostKind(pub String);

impl std::fmt::Display for UnknownCostKind {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "cost kind '{}' is not one this version knows; it cannot be left out of the profit, and \
             guessing where it belongs would be inventing an answer",
            self.0
        )
    }
}

/// One committed cost, as the ledger records it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerCost {
    pub amount: u128,
    pub kind: String,
}

/// What a trade earned, and what it spent earning it.
///
/// Fields are per cost kind, plus the three quantities the decomposition starts
/// and ends with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profit {
    /// Proceeds before any cost.
    pub gross: u128,
    /// Capital returned rather than earned.
    pub principal: u128,
    pub gas: u128,
    pub liquidity_fee: u128,
    pub flash_liquidity_fee: u128,
    pub solver_infrastructure_fee: u128,
    pub proof_fee: u128,
    pub cross_domain_fee: u128,
    pub slippage: u128,
    pub price_impact: u128,
    pub mev_leakage: u128,
    /// The declared safety buffer, held back from profit on purpose.
    pub safety_buffer: u128,
    /// `gross - principal - costs - buffer`. Negative when the trade lost money.
    pub net: i128,
    /// `net` as basis points of `gross`, negative when `net` is. Zero when there
    /// are no proceeds: a margin against nothing is not a number.
    pub margin_bps: i64,
}

impl Profit {
    /// Assemble the decomposition from the ledger.
    ///
    /// `gross` is what the settlement asset realised before costs, `principal` is
    /// the capital returned, and `safety_buffer` is what the policy holds back.
    /// Returns the unknown kind rather than a profit figure when a recorded cost
    /// cannot be placed.
    pub fn from_ledger(
        gross: u128,
        principal: u128,
        safety_buffer: u128,
        costs: &[LedgerCost],
    ) -> Result<Profit, UnknownCostKind> {
        let mut profit = Profit {
            gross,
            principal,
            gas: 0,
            liquidity_fee: 0,
            flash_liquidity_fee: 0,
            solver_infrastructure_fee: 0,
            proof_fee: 0,
            cross_domain_fee: 0,
            slippage: 0,
            price_impact: 0,
            mev_leakage: 0,
            safety_buffer,
            net: 0,
            margin_bps: 0,
        };

        for cost in costs {
            let kind = CostKind::from_str(&cost.kind).ok_or_else(|| UnknownCostKind(cost.kind.clone()))?;
            // Exhaustive on purpose: a new `CostKind` does not compile until it
            // has somewhere to go, so no category can be left out of `net` by
            // omission.
            let bucket = match kind {
                CostKind::Gas => &mut profit.gas,
                CostKind::LiquidityFee => &mut profit.liquidity_fee,
                CostKind::FlashLiquidityFee => &mut profit.flash_liquidity_fee,
                CostKind::SolverInfrastructureFee => &mut profit.solver_infrastructure_fee,
                CostKind::ProofFee => &mut profit.proof_fee,
                CostKind::CrossDomainFee => &mut profit.cross_domain_fee,
                CostKind::Slippage => &mut profit.slippage,
                CostKind::PriceImpact => &mut profit.price_impact,
                CostKind::MevLeakage => &mut profit.mev_leakage,
            };
            *bucket = bucket
                .checked_add(cost.amount)
                .ok_or_else(|| UnknownCostKind(format!("{} overflows the cost total", cost.kind)))?;
        }

        profit.net = profit.net_profit()?;
        profit.margin_bps = profit.margin_bps()?;
        Ok(profit)
    }

    /// `gross - principal - every cost - buffer`, exactly and signed.
    fn net_profit(&self) -> Result<i128, UnknownCostKind> {
        let gross = i128::try_from(self.gross).map_err(|_| Overflow("gross"))?;
        let principal = i128::try_from(self.principal).map_err(|_| Overflow("principal"))?;
        let buffer = i128::try_from(self.safety_buffer).map_err(|_| Overflow("safety buffer"))?;
        let costs = i128::try_from(self.total_costs()).map_err(|_| Overflow("cost total"))?;
        Ok(gross - principal - costs - buffer)
    }

    fn margin_bps(&self) -> Result<i64, UnknownCostKind> {
        if self.gross == 0 {
            return Ok(0);
        }
        // Integer arithmetic: a margin is a share of money, and PHASE 43 has no
        // room for a float here. Basis points by construction, so the rounding
        // is exact for the unit the language uses.
        let numerator = self.net * 10_000;
        let denominator = i128::try_from(self.gross).map_err(|_| Overflow("gross"))?;
        Ok((numerator / denominator) as i64)
    }

    /// Every cost added together.
    pub fn total_costs(&self) -> u128 {
        [
            self.gas,
            self.liquidity_fee,
            self.flash_liquidity_fee,
            self.solver_infrastructure_fee,
            self.proof_fee,
            self.cross_domain_fee,
            self.slippage,
            self.price_impact,
            self.mev_leakage,
        ]
        .into_iter()
        .fold(0u128, |total, cost| total.saturating_add(cost))
    }

    /// The venue's cut, which the spec calls the DEX fee.
    pub fn dex_fees(&self) -> u128 {
        self.liquidity_fee
    }

    /// The flash-liquidity cut.
    pub fn flash_fees(&self) -> u128 {
        self.flash_liquidity_fee
    }

    /// Infrastructure the trade depends on: solvers, and the proof and
    /// cross-domain fees settlement costs.
    pub fn protocol_fees(&self) -> u128 {
        self.solver_infrastructure_fee
            .saturating_add(self.proof_fee)
            .saturating_add(self.cross_domain_fee)
    }

    /// What the swap itself cost on the way: slippage, price impact and leaked
    /// MEV, none of which is a fee anyone charges.
    pub fn execution_costs(&self) -> u128 {
        self.slippage
            .saturating_add(self.price_impact)
            .saturating_add(self.mev_leakage)
    }

    /// `None` — the language models no hedging, so there is no such cost to
    /// report. A zero field would claim a hedge happened and cost nothing.
    pub fn hedging_costs(&self) -> Option<u128> {
        None
    }

    /// `None` — unrealised profit needs a price, and the language has no oracle
    /// in this path. Everything in the ledger has settled, so [`Profit::net`] is
    /// the realised figure.
    pub fn unrealized(&self) -> Option<i128> {
        None
    }

    /// The realised figure: what this value is.
    pub fn realized(&self) -> i128 {
        self.net
    }
}

/// The spec's decomposition, in its own words, for a caller that wants to print
/// it: `gross - principal - costs - buffer = net`.
pub fn describe(profit: &Profit) -> String {
    format!(
        "gross {} - principal {} - gas {} - dex {} - flash {} - protocol {} - execution {} - buffer {} = net {} ({} bps)",
        profit.gross,
        profit.principal,
        profit.gas,
        profit.dex_fees(),
        profit.flash_fees(),
        profit.protocol_fees(),
        profit.execution_costs(),
        profit.safety_buffer,
        profit.net,
        profit.margin_bps
    )
}

/// An amount that does not fit the signed arithmetic the decomposition needs.
struct Overflow(&'static str);

impl From<Overflow> for UnknownCostKind {
    fn from(overflow: Overflow) -> Self {
        UnknownCostKind(format!("{} exceeds the range the profit type can express", overflow.0))
    }
}
