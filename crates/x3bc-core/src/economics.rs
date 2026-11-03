//! Deterministic economic outcomes: what a trade actually earned, what it actually cost, and
//! what a performance fee may therefore be charged on.
//!
//! This is the calculation a fee is allowed to rest on. It reads only authoritative accounting
//! already present in a [`TradingReceipt`] — the pre and post balances of the settlement asset and
//! the cost ledger — and never takes a total on trust. Two parties holding the same receipt must
//! arrive at the same answer, bit for bit, which is what makes a fee chargeable rather than
//! negotiable.
//!
//! # What "net" means here
//!
//! Everything is denominated in **one** settlement asset. A trade that ends holding more of that
//! asset than it started with has a positive net; one that ends holding less has a loss. Costs are
//! reported in three buckets so a fee receipt can show its work:
//!
//! * `embedded` costs are already inside a movement (a venue fee inside a swap's input, a flash fee
//!   inside a repayment). Adding them again would double-count them, so they are reported but never
//!   subtracted.
//! * `charged` costs were debited on their own and are already reflected in the balances. They are
//!   reported for attribution, and are *not* subtracted a second time either.
//! * `gross` is the net plus the charged costs: what the trade would have made with no explicit
//!   charges. It exists so a receipt can show "earned X, paid Y, kept Z" without the reader having
//!   to re-derive it.
//!
//! The net is therefore `post - pre` of the settlement asset, and every other figure here is
//! derived from it or from the cost ledger. There is exactly one source of truth.

use alloc::vec::Vec;

use crate::receipt::{CostEntry, CostKind, Outcome, TradingReceipt};

/// Why an economic outcome cannot be calculated or cannot be charged for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EconomicsError {
    /// The receipt describes a trade that did not commit. A rolled-back trade has no outcome to
    /// price: its balances are the pre-state and charging anything for it would be charging for an
    /// effect that never happened.
    NotCommitted,
    /// The receipt carries no balance for the settlement asset, so there is nothing to difference.
    MissingBalance,
    /// A post balance exists without a pre balance while the denomination is not the settlement
    /// asset, i.e. the receipt describes a different trade than the one being priced.
    AssetMismatch,
    /// A balance or cost figure does not fit the signed or unsigned width used for the result.
    Overflow,
    /// The fee rate is not a valid basis-point value (it must be at most 10_000, i.e. 100%).
    InvalidFeeRate,
}

/// A cost total for one kind and asset, as the receipt recorded it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CostTotal {
    pub kind: CostKind,
    pub asset: u32,
    pub amount: u128,
}

/// The economic outcome of one committed trade, in a single settlement asset.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EconomicOutcome {
    /// The asset every figure below is denominated in.
    pub asset: u32,
    /// Trader balance before the trade.
    pub pre: u128,
    /// Trader balance after the trade.
    pub post: u128,
    /// `post - pre`. Negative for a loss. This is the only figure a fee may rest on.
    pub net: i128,
    /// Costs debited on their own, already inside `net`. Reported for attribution only.
    pub charged: u128,
    /// Costs already embedded in a movement, also inside `net`. Reported, never subtracted.
    pub embedded: u128,
    /// `net + charged`: what the trade would have netted with no explicit charges.
    pub gross: i128,
    /// Every cost entry the receipt recorded, so a receipt consumer can attribute the total.
    pub costs: Vec<CostTotal>,
}

impl EconomicOutcome {
    /// Whether the trade made money in the settlement asset.
    pub fn is_profit(&self) -> bool {
        self.net > 0
    }

    /// Whether the trade cleared `minimum` net. A loss never clears a positive minimum.
    pub fn meets_minimum(&self, minimum: u128) -> bool {
        self.net > 0 && (self.net as u128) >= minimum
    }

    /// Total charged cost of one kind, in the settlement asset.
    pub fn charged_of(&self, kind: CostKind) -> u128 {
        self.costs
            .iter()
            .filter(|total| total.kind == kind)
            .map(|total| total.amount)
            .fold(0u128, |sum, amount| sum.saturating_add(amount))
    }
}

/// Derive the settlement asset's economic outcome from a receipt.
///
/// `settlement_asset` must be the asset the caller intends to account in. The receipt's own
/// `profit.asset` is what the engine settled in; if the caller asks for a different asset the trade
/// cannot be priced in it without a conversion rate this module does not have, so that is refused
/// rather than silently valued at one-to-one.
pub fn outcome(
    receipt: &TradingReceipt,
    settlement_asset: u32,
) -> Result<EconomicOutcome, EconomicsError> {
    if !matches!(receipt.outcome, Outcome::Committed) {
        return Err(EconomicsError::NotCommitted);
    }
    if receipt.profit.asset != settlement_asset {
        return Err(EconomicsError::AssetMismatch);
    }

    let pre = balance_of(&receipt.pre_balances, settlement_asset)
        .ok_or(EconomicsError::MissingBalance)?;
    let post = balance_of(&receipt.post_balances, settlement_asset)
        .ok_or(EconomicsError::MissingBalance)?;

    price_balances(settlement_asset, pre, post, &receipt.costs)
}

/// The arithmetic behind [`outcome`], over the four figures that decide it.
///
/// Separate from [`outcome`] so the calculation can be exercised directly, and so a caller that
/// already holds authoritative balances (a runtime that just executed a trade, before a receipt is
/// assembled) can price it without building a receipt first.
pub fn price_balances(
    settlement_asset: u32,
    pre: u128,
    post: u128,
    costs: &[CostEntry],
) -> Result<EconomicOutcome, EconomicsError> {
    let net = signed_difference(pre, post)?;

    // Only non-embedded entries in the settlement asset count toward `charged`: an embedded fee is
    // already inside a swap or a repayment, and a fee in another asset is not part of this
    // denomination's arithmetic.
    let mut charged: u128 = 0;
    let mut embedded: u128 = 0;
    for cost in costs {
        if cost.asset != settlement_asset {
            continue;
        }
        if cost.embedded {
            embedded = embedded
                .checked_add(cost.amount)
                .ok_or(EconomicsError::Overflow)?;
        } else {
            charged = charged
                .checked_add(cost.amount)
                .ok_or(EconomicsError::Overflow)?;
        }
    }

    let gross = net
        .checked_add(i128::try_from(charged).map_err(|_| EconomicsError::Overflow)?)
        .ok_or(EconomicsError::Overflow)?;

    Ok(EconomicOutcome {
        asset: settlement_asset,
        pre,
        post,
        net,
        charged,
        embedded,
        gross,
        costs: totals(costs),
    })
}

/// A performance fee on a verified outcome.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PerformanceFee {
    /// The asset the fee is denominated in.
    pub asset: u32,
    /// Fee taken, in the settlement asset. Zero for a loss or a loss-making outcome.
    pub fee: u128,
    /// What the trader keeps after the fee.
    pub trader_keeps: u128,
}

/// Charge `rate_bps` of the outcome's net profit.
///
/// A fee is only ever charged on a **positive** net. A loss produces a zero fee, never a negative
/// one and never a fee on gross activity: charging a percentage of fees paid, or of volume, is a
/// different product and would not be a share of verified value.
///
/// `rate_bps` is basis points (`2_000` = 20%) and must be at most `10_000`.
pub fn performance_fee(
    outcome: &EconomicOutcome,
    rate_bps: u32,
) -> Result<PerformanceFee, EconomicsError> {
    if rate_bps > 10_000 {
        return Err(EconomicsError::InvalidFeeRate);
    }
    if !outcome.is_profit() {
        return Ok(PerformanceFee {
            asset: outcome.asset,
            fee: 0,
            trader_keeps: 0,
        });
    }
    let net = outcome.net as u128;
    // Multiply before dividing, in u128: net <= i128::MAX and rate <= 10_000, so the product fits
    // comfortably. Computing `net / 10_000 * rate` first would floor away most of a small profit.
    let fee = net
        .checked_mul(u128::from(rate_bps))
        .ok_or(EconomicsError::Overflow)?
        / 10_000;
    Ok(PerformanceFee {
        asset: outcome.asset,
        fee,
        trader_keeps: net - fee,
    })
}

/// Split a fee between participants according to integer shares that must sum to the fee.
///
/// Shares are basis points of the *fee*, not of the profit. `shares` must sum to exactly `10_000`
/// so the split neither creates nor destroys value; the remainder from integer division goes to the
/// last participant, which keeps the total exact.
pub fn split_fee<'a>(
    fee: u128,
    shares: &[(u16, &'a str)],
) -> Result<Vec<(u16, &'a str, u128)>, EconomicsError> {
    let total_bps: u32 = shares.iter().map(|(bps, _)| u32::from(*bps)).sum();
    if total_bps != 10_000 {
        return Err(EconomicsError::InvalidFeeRate);
    }
    let mut out = Vec::with_capacity(shares.len());
    let mut distributed: u128 = 0;
    let last = shares.len().saturating_sub(1);
    for (index, (bps, name)) in shares.iter().enumerate() {
        let amount = if index == last {
            fee.saturating_sub(distributed)
        } else {
            fee.checked_mul(u128::from(*bps))
                .ok_or(EconomicsError::Overflow)?
                / 10_000
        };
        distributed = distributed.saturating_add(amount);
        out.push((*bps, *name, amount));
    }
    Ok(out)
}

fn balance_of(balances: &[(u32, u128)], asset: u32) -> Option<u128> {
    balances
        .iter()
        .find(|(entry, _)| *entry == asset)
        .map(|(_, amount)| *amount)
}

/// `post - pre` as an `i128`, computed in unsigned arithmetic first.
///
/// Converting balances to `i128` before subtracting misreads any balance above `i128::MAX` as
/// negative; subtracting first keeps the difference exact for every `u128` pair whose difference
/// fits an `i128`.
fn signed_difference(pre: u128, post: u128) -> Result<i128, EconomicsError> {
    if post >= pre {
        i128::try_from(post - pre).map_err(|_| EconomicsError::Overflow)
    } else {
        let decrease = pre - post;
        if decrease == (i128::MAX as u128) + 1 {
            Ok(i128::MIN)
        } else {
            i128::try_from(decrease)
                .map(|value| -value)
                .map_err(|_| EconomicsError::Overflow)
        }
    }
}

fn totals(costs: &[CostEntry]) -> Vec<CostTotal> {
    let mut out: Vec<CostTotal> = Vec::new();
    for cost in costs {
        match out
            .iter_mut()
            .find(|total| total.kind == cost.kind && total.asset == cost.asset)
        {
            Some(total) => total.amount = total.amount.saturating_add(cost.amount),
            None => out.push(CostTotal {
                kind: cost.kind,
                asset: cost.asset,
                amount: cost.amount,
            }),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const USDC: u32 = 7;

    fn cost(kind: CostKind, asset: u32, amount: u128, embedded: bool) -> CostEntry {
        CostEntry {
            kind,
            asset,
            amount,
            embedded,
        }
    }

    fn price(pre: u128, post: u128, costs: Vec<CostEntry>) -> EconomicOutcome {
        price_balances(USDC, pre, post, &costs).expect("prices")
    }

    #[test]
    fn a_profitable_trade_reports_its_net_and_a_20_percent_fee() {
        // Started with 10_000, ended with 10_800: +800 net.
        let outcome = price(10_000, 10_800, vec![]);
        assert_eq!(outcome.net, 800);
        assert!(outcome.is_profit());
        assert!(outcome.meets_minimum(800));
        assert!(!outcome.meets_minimum(801));

        let fee = performance_fee(&outcome, 2_000).unwrap();
        assert_eq!(fee.fee, 160);
        assert_eq!(fee.trader_keeps, 640);
    }

    #[test]
    fn zero_profit_earns_no_fee() {
        let outcome = price(10_000, 10_000, vec![]);
        assert_eq!(outcome.net, 0);
        assert!(!outcome.is_profit());
        assert!(
            !outcome.meets_minimum(0),
            "a break-even trade clears no minimum"
        );
        assert_eq!(performance_fee(&outcome, 2_000).unwrap().fee, 0);
    }

    #[test]
    fn a_loss_earns_no_fee_and_reports_negative_net() {
        let outcome = price(10_000, 9_500, vec![]);
        assert_eq!(outcome.net, -500);
        assert!(!outcome.is_profit());
        assert!(!outcome.meets_minimum(1));
        let fee = performance_fee(&outcome, 2_000).unwrap();
        assert_eq!(fee.fee, 0, "a loss never produces a fee");
        assert_eq!(fee.trader_keeps, 0);
    }

    #[test]
    fn charged_costs_are_attributed_but_not_subtracted_twice() {
        let outcome = price(10_000, 10_800, vec![cost(CostKind::Gas, USDC, 400, false)]);
        assert_eq!(outcome.net, 800);
        assert_eq!(outcome.charged, 400);
        assert_eq!(
            outcome.gross, 1_200,
            "gross = net + charged, for attribution"
        );
        assert_eq!(outcome.charged_of(CostKind::Gas), 400);
        // The fee rests on net, not gross.
        assert_eq!(performance_fee(&outcome, 2_000).unwrap().fee, 160);
    }

    #[test]
    fn embedded_costs_are_reported_and_never_added_to_gross() {
        let outcome = price(
            10_000,
            10_800,
            vec![cost(CostKind::VenueFee, USDC, 250, true)],
        );
        assert_eq!(outcome.embedded, 250);
        assert_eq!(outcome.charged, 0);
        assert_eq!(
            outcome.gross, 800,
            "an embedded fee is already inside the swap, so it is not added back"
        );
    }

    #[test]
    fn costs_in_another_asset_are_not_counted_in_this_denomination() {
        let outcome = price(10_000, 10_800, vec![cost(CostKind::Gas, 999, 50, false)]);
        assert_eq!(outcome.charged, 0);
        assert_eq!(outcome.gross, 800);
        // But the entry is still visible for attribution.
        assert_eq!(outcome.costs.len(), 1);
        assert_eq!(outcome.costs[0].asset, 999);
    }

    #[test]
    fn costs_of_one_kind_and_asset_are_totaled() {
        let outcome = price(
            10_000,
            10_800,
            vec![
                cost(CostKind::Gas, USDC, 100, false),
                cost(CostKind::Gas, USDC, 150, false),
                cost(CostKind::ProtocolFee, USDC, 50, false),
            ],
        );
        assert_eq!(outcome.charged_of(CostKind::Gas), 250);
        assert_eq!(outcome.charged_of(CostKind::ProtocolFee), 50);
        assert_eq!(outcome.charged, 300);
        assert_eq!(outcome.costs.len(), 2);
    }

    #[test]
    fn a_fee_rate_above_one_hundred_percent_is_refused() {
        let outcome = price(10_000, 10_800, vec![]);
        assert_eq!(
            performance_fee(&outcome, 10_001),
            Err(EconomicsError::InvalidFeeRate)
        );
        // Exactly 100% is legal arithmetic, however unwise.
        assert_eq!(performance_fee(&outcome, 10_000).unwrap().fee, 800);
    }

    #[test]
    fn a_small_profit_is_not_floored_away() {
        let outcome = price(1_000_000, 1_000_010, vec![]);
        assert_eq!(outcome.net, 10);
        assert_eq!(performance_fee(&outcome, 2_000).unwrap().fee, 2);

        let outcome = price(1_000_000, 1_000_001, vec![]);
        assert_eq!(performance_fee(&outcome, 2_000).unwrap().fee, 0);
    }

    #[test]
    fn balances_above_i128_max_difference_exactly() {
        let big = (i128::MAX as u128) + 1_000;
        let outcome = price(big, big + 500, vec![]);
        assert_eq!(outcome.net, 500);
    }

    #[test]
    fn a_loss_of_exactly_i128_min_is_representable() {
        let pre = (i128::MAX as u128) + 1;
        let outcome = price(pre, 0, vec![]);
        assert_eq!(outcome.net, i128::MIN);
    }

    #[test]
    fn a_difference_wider_than_i128_is_an_overflow_not_a_wrap() {
        let pre = u128::MAX;
        let outcome = price_balances(USDC, pre, 0, &[]);
        assert_eq!(outcome, Err(EconomicsError::Overflow));
    }

    #[test]
    fn split_fee_distributes_exactly_the_fee() {
        let parts =
            split_fee(160, &[(8_000, "trader"), (1_500, "developer"), (500, "x3")]).unwrap();
        assert_eq!(parts[0].2, 128);
        assert_eq!(parts[1].2, 24);
        assert_eq!(parts[2].2, 8);
        assert_eq!(parts.iter().map(|p| p.2).sum::<u128>(), 160);
    }

    #[test]
    fn split_fee_gives_the_rounding_remainder_to_the_last_participant() {
        let parts = split_fee(100, &[(3_333, "a"), (3_333, "b"), (3_334, "c")]).unwrap();
        assert_eq!(parts.iter().map(|p| p.2).sum::<u128>(), 100);
    }

    #[test]
    fn shares_that_do_not_sum_to_one_hundred_percent_are_refused() {
        assert_eq!(
            split_fee(100, &[(5_000, "a"), (4_000, "b")]),
            Err(EconomicsError::InvalidFeeRate)
        );
    }

    #[test]
    fn a_zero_fee_splits_into_zeros() {
        let parts = split_fee(0, &[(8_000, "trader"), (2_000, "x3")]).unwrap();
        assert!(parts.iter().all(|p| p.2 == 0));
    }
}
