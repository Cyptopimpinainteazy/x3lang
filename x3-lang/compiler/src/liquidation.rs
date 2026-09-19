//! Atomic liquidations — spec PHASE 10.
//!
//! The phase asks for a liquidation primitive and says what the verifier owes it:
//! *ensure repayment and valid final position*. With the five clauses the surface
//! states, both are decidable from the program's own numbers:
//!
//! ```text
//! liquidate  what is advanced to repay the borrower's debt
//! receive    what the liquidation seizes
//! swap       collateral → the asset being repaid, with a `min_output`
//! repay      the capital handed back
//! net_profit the floor the caller claims is left over
//! ```
//!
//! - **repayment**: the swap's `min_output` — the least the collateral converts to —
//!   must cover `repay`. A liquidation whose own bound cannot repay its capital is
//!   refused with both figures;
//! - **valid final position**: the swap must convert *all* the collateral seized (a
//!   swap of less leaves a position nobody accounted for), the collateral must be
//!   what the swap spends, and the swap's output must be the asset being repaid;
//! - **the floor**: `min_output − repaid` is the least the liquidation leaves, and a
//!   floor above it is refused with the figures. It is a minimum, not a prediction:
//!   a swap that delivers more than `min_output` leaves more.
//!
//! What it cannot do is execute: `liquidate` and `receive` are calls into a lending
//! protocol this VM has no adapter for, so the IR verifier refuses the operation
//! rather than emitting a plan nothing can run (TICKET-069).

use x3_lang_ast::ast::{AssetRef, AtomicLiquidationDecl, Item, Program};
use x3_lang_common::{ErrorAccumulator, Span, X3Error};

/// What a liquidation's own figures say about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiquidationLedger {
    pub position: String,
    /// The asset the debt is denominated in — what is repaid and what the swap
    /// produces.
    pub debt_asset: String,
    /// The asset seized.
    pub collateral_asset: String,
    pub capital: u128,
    pub collateral: u128,
    pub swapped_in: u128,
    pub min_output: u128,
    pub repaid: u128,
    pub profit_floor: Option<u128>,
}

impl LiquidationLedger {
    /// What the liquidation leaves at the swap's own minimum: output minus repayment.
    pub fn net_at_minimum(&self) -> u128 {
        self.min_output.saturating_sub(self.repaid)
    }
}

/// Decide a liquidation from its own figures.
pub fn ledger(decl: &AtomicLiquidationDecl) -> Result<LiquidationLedger, String> {
    let (capital, capital_asset) = &decl.capital;
    let (collateral, collateral_asset) = &decl.collateral;
    let (repaid, repaid_asset) = &decl.repaid;
    let _ = capital_asset;

    if *capital == 0 {
        return Err("the liquidation advances no capital, so there is nothing to repay".to_string());
    }
    if *collateral == 0 {
        return Err("the liquidation seizes no collateral, so the swap has nothing to convert".to_string());
    }
    if *repaid == 0 {
        return Err("the liquidation repays nothing, so its capital would stay outstanding".to_string());
    }
    if decl.swap.amount != *collateral {
        return Err(format!(
            "the swap converts {} of the {} collateral received, leaving {} unaccounted for; a \
             liquidation's final position is the point of the clause, so the whole collateral has to \
             be converted or repaid",
            decl.swap.amount,
            collateral,
            collateral.saturating_sub(decl.swap.amount)
        ));
    }
    let swap_from = key(&decl.swap.from);
    if swap_from != key(collateral_asset) {
        return Err(format!(
            "the swap spends '{swap_from}' but the collateral seized is '{}'; swapping an asset the \
             liquidation did not receive is a plan about something else",
            key(collateral_asset)
        ));
    }
    let swap_to = key(&decl.swap.to);
    if swap_to != key(repaid_asset) {
        return Err(format!(
            "the swap produces '{swap_to}' but the repayment is in '{}'; the conversion has to \
             produce what is repaid",
            key(repaid_asset)
        ));
    }
    if decl.swap.min_output < *repaid {
        return Err(format!(
            "the liquidation cannot repay its capital: the swap's own minimum output is {} but {} \
             has to be repaid — the shortfall is {}",
            decl.swap.min_output,
            repaid,
            repaid.saturating_sub(decl.swap.min_output)
        ));
    }

    let floor = decl.profit_floor.as_ref().map(|(amount, asset)| (*amount, key(asset)));
    if let Some((_, asset)) = &floor {
        if *asset != key(repaid_asset) {
            return Err(format!(
                "the profit floor is in '{asset}' while the liquidation repays in '{}'; a floor in \
                 another asset is a claim about a price this compiler does not have",
                key(repaid_asset)
            ));
        }
    }
    let ledger = LiquidationLedger {
        position: decl.position.as_str().to_string(),
        debt_asset: key(repaid_asset),
        collateral_asset: key(collateral_asset),
        capital: *capital,
        collateral: *collateral,
        swapped_in: decl.swap.amount,
        min_output: decl.swap.min_output,
        repaid: *repaid,
        profit_floor: floor.map(|(amount, _)| amount),
    };
    if let Some(floor) = ledger.profit_floor {
        if ledger.net_at_minimum() < floor {
            return Err(format!(
                "the liquidation cannot meet its declared floor at the output it itself bounds: it \
                 requires at least {floor} {} but at the swap's minimum output of {} it leaves {} \
                 after repaying {}",
                ledger.debt_asset,
                ledger.min_output,
                ledger.net_at_minimum(),
                ledger.repaid
            ));
        }
    }
    Ok(ledger)
}

/// Verify every liquidation in a program.
pub fn verify(program: &Program, acc: &mut ErrorAccumulator) {
    for item in &program.items {
        let Item::AtomicLiquidation(decl) = &item.node else {
            continue;
        };
        if let Err(reason) = ledger(decl) {
            acc.add_error(err(reason));
        }
    }
}

/// `chain.ASSET`, the identity the clauses have to agree on.
fn key(asset: &AssetRef) -> String {
    format!("{}.{}", asset.chain.as_str(), asset.name.as_str())
}

fn err(message: impl Into<String>) -> X3Error {
    X3Error::SemanticError {
        message: message.into(),
        span: Span::DUMMY,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOUND: &str = "atomic_liquidation {\n    liquidate 1_000 ethereum.USDC of \
                         borrower.position;\n    receive 1_200 ethereum.ETH collateral;\n    swap \
                         1_200 ethereum.ETH -> ethereum.USDC min_output 1_100;\n    repay 1_000 \
                         ethereum.USDC;\n    require net_profit >= 100 ethereum.USDC;\n}\n";

    fn analysed(source: &str) -> Result<LiquidationLedger, String> {
        let program = crate::parser::parse_source(source).expect("the fixture must parse");
        let Item::AtomicLiquidation(decl) = &program.items[0].node else {
            panic!("the fixture is a liquidation");
        };
        ledger(decl)
    }

    #[test]
    fn a_sound_liquidation_reports_its_ledger() {
        let ledger = analysed(SOUND).expect("the fixture repays and meets its floor");
        assert_eq!(ledger.position, "borrower.position");
        assert_eq!(ledger.capital, 1_000);
        assert_eq!(ledger.collateral, 1_200);
        assert_eq!(ledger.min_output, 1_100);
        assert_eq!(ledger.repaid, 1_000);
        assert_eq!(
            ledger.net_at_minimum(),
            100,
            "the floor is exactly what the declared minimum leaves"
        );
    }

    #[test]
    fn a_swap_that_cannot_cover_the_repayment_is_refused() {
        let source = SOUND.replace("min_output 1_100", "min_output 900");
        let reason = analysed(&source).expect_err("the swap cannot repay the capital");
        assert!(
            reason.contains("cannot repay its capital")
                && reason.contains("minimum output is 900")
                && reason.contains("shortfall is 100"),
            "{reason}"
        );
    }

    #[test]
    fn a_floor_above_what_the_minimum_leaves_is_refused() {
        let source = SOUND.replace("net_profit >= 100", "net_profit >= 250");
        let reason = analysed(&source).expect_err("the floor is above the minimum");
        assert!(
            reason.contains("at least 250") && reason.contains("leaves 100 after repaying 1000"),
            "{reason}"
        );
    }

    #[test]
    fn collateral_left_unconverted_is_a_position_nobody_accounted_for() {
        let source = SOUND.replace("swap 1_200 ethereum.ETH", "swap 1_000 ethereum.ETH");
        let reason = analysed(&source).expect_err("the leftovers are a position");
        assert!(reason.contains("leaving 200 unaccounted for"), "{reason}");
    }

    #[test]
    fn the_clauses_have_to_agree_on_their_assets() {
        let wrong_collateral = SOUND.replace("swap 1_200 ethereum.ETH", "swap 1_200 ethereum.WBTC");
        assert!(
            analysed(&wrong_collateral)
                .expect_err("the swap spends what was not received")
                .contains("swapping an asset the liquidation did not receive"),
            "{wrong_collateral}"
        );

        let wrong_output = SOUND.replace("-> ethereum.USDC min_output", "-> ethereum.DAI min_output");
        assert!(
            analysed(&wrong_output)
                .expect_err("the conversion must produce what is repaid")
                .contains("the conversion has to produce what is repaid"),
            "{wrong_output}"
        );

        let wrong_floor = SOUND.replace("net_profit >= 100 ethereum.USDC", "net_profit >= 100 ethereum.DAI");
        assert!(
            analysed(&wrong_floor)
                .expect_err("a floor in another asset is a claim about a price")
                .contains("a claim about a price this compiler does not have"),
            "{wrong_floor}"
        );
    }

    #[test]
    fn zero_amounts_are_refused_because_there_is_nothing_to_repay() {
        let no_capital = SOUND.replace("liquidate 1_000", "liquidate 0");
        assert!(
            analysed(&no_capital)
                .expect_err("no capital")
                .contains("advances no capital"),
            "{no_capital}"
        );
    }
}
