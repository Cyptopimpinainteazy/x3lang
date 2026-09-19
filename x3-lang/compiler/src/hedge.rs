//! Atomic hedges — spec PHASE 9.
//!
//! The phase asks for "a hedge primitive" whose legs live in one economic plan, and
//! for the compiler to *reason about directional exposure*: a hedge is two legs on
//! one asset (a long and a short, spot or perp) whose net is what is left open, and
//! `require delta <= 0.01%` is a claim about that net.
//!
//! So the exposure is computed, not taken on trust:
//!
//! ```text
//! delta_bps = |long − short| × 10_000 / long
//! ```
//!
//! with `long` the notional being hedged — the denominator is the position, because
//! "0.01% of what I am hedging" is the claim a hedge makes. A leg written
//! `equivalent` takes the size of the other side, which is how the language says
//! "short the same notional" without repeating the number, and two `equivalent` legs
//! (nothing to be equivalent *to*) are refused.
//!
//! The asset is part of the net: legs on `ethereum.ETH` and `solana.ETH` are two
//! different assets, and netting them would be a claim about a price this compiler
//! does not have. A hedge with one leg is a position rather than a hedge, and it is
//! refused for the same reason.
//!
//! What this cannot do: execute the legs. A perp leg needs a venue this VM has no
//! adapter for, so the emitter refuses to produce an artifact containing one (the
//! IR verifier says so in `verify.rs`) — the exposure is decided, the execution is
//! not pretended.

use x3_lang_ast::ast::{AtomicHedgeDecl, HedgeLeg, HedgeQuantity, HedgeSide, HedgeVenue, Item, Program};
use x3_lang_common::{ErrorAccumulator, Span, X3Error};

/// The two sides of a hedge, resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HedgeExposure {
    pub asset: String,
    pub long: u128,
    pub short: u128,
}

impl HedgeExposure {
    /// The net left open, as basis points of the notional being hedged.
    ///
    /// Saturating and integer: a hedge is a claim about a position, and a fractional
    /// basis point is not a quantity this language compares (PHASE 43).
    pub fn delta_bps(&self) -> u128 {
        if self.long == 0 {
            return 0;
        }
        self.long.abs_diff(self.short).saturating_mul(10_000) / self.long
    }
}

/// Resolve a hedge's legs into a net exposure, or say what is wrong with them.
pub fn exposure(decl: &AtomicHedgeDecl) -> Result<HedgeExposure, String> {
    if decl.legs.len() < 2 {
        return Err(format!(
            "a hedge with {} leg is a position, not a hedge: write the long and the short",
            decl.legs.len()
        ));
    }
    let asset = leg_key(&decl.legs[0]);
    for leg in &decl.legs {
        let key = leg_key(leg);
        if key != asset {
            return Err(format!(
                "the legs hedge different assets ('{asset}' and '{key}'); a delta is per asset, and \
                 netting two assets would be a claim about a price this compiler does not have"
            ));
        }
    }

    let equivalent_legs: Vec<&HedgeLeg> = decl
        .legs
        .iter()
        .filter(|leg| leg.quantity == HedgeQuantity::Equivalent)
        .collect();
    if equivalent_legs.len() > 1 {
        return Err(format!(
            "{} legs say `equivalent` and none of them states a size: at least one side has to say \
             what it is equivalent to",
            equivalent_legs.len()
        ));
    }

    let written = |side: HedgeSide| -> u128 {
        decl.legs
            .iter()
            .filter(|leg| leg.side == side)
            .filter_map(|leg| match leg.quantity {
                HedgeQuantity::Amount(amount) => Some(amount),
                HedgeQuantity::Equivalent => None,
            })
            .fold(0u128, u128::saturating_add)
    };

    let (long, short) = match equivalent_legs.first().map(|leg| leg.side) {
        // The `equivalent` leg takes the other side's written size.
        Some(HedgeSide::Long) => (written(HedgeSide::Short), written(HedgeSide::Short)),
        Some(HedgeSide::Short) => (written(HedgeSide::Long), written(HedgeSide::Long)),
        None => (written(HedgeSide::Long), written(HedgeSide::Short)),
    };
    if long == 0 && short == 0 {
        return Err("both sides of the hedge are zero, so there is nothing to hedge".to_string());
    }
    Ok(HedgeExposure { asset, long, short })
}

/// Verify every hedge in a program: the legs have to net to an exposure, and the
/// exposure has to satisfy the guard the hedge states.
pub fn verify(program: &Program, acc: &mut ErrorAccumulator) {
    for item in &program.items {
        let Item::AtomicHedge(decl) = &item.node else {
            continue;
        };
        let exposure = match exposure(decl) {
            Ok(exposure) => exposure,
            Err(reason) => {
                acc.add_error(err(reason));
                continue;
            }
        };
        let Some(bound) = decl.delta_bound_bps else {
            acc.add_error(err(format!(
                "the hedge on '{}' states no `require delta <= <pct>` bound, so nothing checks that \
                 it hedges anything: the legs are recorded and the net they leave is not claimed",
                exposure.asset
            )));
            continue;
        };
        let delta = exposure.delta_bps();
        if delta > u128::from(bound) {
            // The figures, not just the verdict: which side is short of the other is
            // what the author needs to fix it.
            let (larger, smaller) = if exposure.long >= exposure.short {
                ("long", "short")
            } else {
                ("short", "long")
            };
            acc.add_error(err(format!(
                "the hedge on '{}' leaves a delta of {delta} bps, above the declared bound of {bound} \
                 bps: {larger} {} against {smaller} {} (of the {} notional being hedged)",
                exposure.asset,
                if larger == "long" {
                    exposure.long
                } else {
                    exposure.short
                },
                if larger == "long" {
                    exposure.short
                } else {
                    exposure.long
                },
                exposure.long.max(exposure.short)
            )));
        }
    }
}

/// `chain.ASSET`, the identity two hedge legs have to share.
fn leg_key(leg: &HedgeLeg) -> String {
    format!("{}.{}", leg.asset.chain.as_str(), leg.asset.name.as_str())
}

/// Whether a leg is on a venue that requires execution outside the VM.
pub fn needs_external_venue(leg: &HedgeLeg) -> bool {
    leg.venue == HedgeVenue::Perp
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

    fn hedge(source: &str) -> Program {
        crate::parser::parse_source(source).expect("the fixture must parse")
    }

    const BALANCED: &str = "atomic_hedge {\n    buy 1_000 ETH spot;\n    short equivalent ETH perp;\n    \
                            require delta <= 0.01%;\n}\n";

    #[test]
    fn the_equivalents_leg_takes_the_other_sides_size() {
        let program = hedge(BALANCED);
        let Item::AtomicHedge(decl) = &program.items[0].node else {
            panic!("the fixture is a hedge");
        };
        let exposure = exposure(decl).expect("the legs net");
        assert_eq!(
            exposure,
            HedgeExposure {
                asset: "unknown.ETH".to_string(),
                long: 1_000,
                short: 1_000,
            }
        );
        assert_eq!(exposure.delta_bps(), 0, "a balanced hedge leaves nothing open");
    }

    #[test]
    fn a_hedge_that_does_not_net_is_refused_with_the_figures() {
        // A 0.01% bound is 1 bp; shorting 900 against a 1_000 long leaves 1_000 bps.
        let source = BALANCED.replace("short equivalent ETH perp", "short 900 ETH perp");
        let program = hedge(&source);
        let mut acc = ErrorAccumulator::new();
        verify(&program, &mut acc);
        let errors: Vec<String> = acc.errors().iter().map(|error| error.to_string()).collect();
        assert_eq!(errors.len(), 1, "{errors:?}");
        let message = &errors[0];
        for expected in ["1000 bps", "bound of 1 bps", "long 1000", "short 900"] {
            assert!(message.contains(expected), "missing {expected:?} in {message}");
        }
    }

    #[test]
    fn a_hedge_without_a_bound_is_refused() {
        let source = BALANCED.replace("    require delta <= 0.01%;\n", "");
        let program = hedge(&source);
        let mut acc = ErrorAccumulator::new();
        verify(&program, &mut acc);
        let errors: Vec<String> = acc.errors().iter().map(|error| error.to_string()).collect();
        assert!(
            errors.iter().any(|error| error.contains("states no `require delta")),
            "{errors:?}"
        );
    }

    #[test]
    fn one_leg_is_not_a_hedge_and_two_assets_are_not_a_delta() {
        let one_leg = "atomic_hedge {\n    buy 1_000 ETH spot;\n    require delta <= 0.01%;\n}\n";
        let mut acc = ErrorAccumulator::new();
        verify(&hedge(one_leg), &mut acc);
        assert!(
            acc.errors()
                .iter()
                .any(|error| error.to_string().contains("is a position, not a hedge")),
            "{:?}",
            acc.errors()
        );

        let two_assets = "atomic_hedge {\n    buy 1_000 ethereum.ETH spot;\n    short equivalent \
                          solana.ETH perp;\n    require delta <= 0.01%;\n}\n";
        let mut acc = ErrorAccumulator::new();
        verify(&hedge(two_assets), &mut acc);
        assert!(
            acc.errors()
                .iter()
                .any(|error| error.to_string().contains("hedge different assets")),
            "{:?}",
            acc.errors()
        );
    }

    #[test]
    fn two_equivalent_legs_have_nothing_to_be_equivalent_to() {
        let source = BALANCED.replace("buy 1_000 ETH spot", "buy equivalent ETH spot");
        let mut acc = ErrorAccumulator::new();
        verify(&hedge(&source), &mut acc);
        assert!(
            acc.errors()
                .iter()
                .any(|error| error.to_string().contains("none of them states a size")),
            "{:?}",
            acc.errors()
        );
    }
}

#[cfg(test)]
mod format_tests {
    use super::*;

    fn parse(source: &str) -> Program {
        crate::parser::parse_source(source).expect("the hedge must parse")
    }

    fn bound(source: &str) -> u32 {
        let program = parse(source);
        let mut acc = ErrorAccumulator::new();
        super::verify(&program, &mut acc);
        assert!(!acc.has_errors(), "{:?}", acc.errors());
        program
            .items
            .iter()
            .find_map(|item| match &item.node {
                Item::AtomicHedge(hedge) => hedge.delta_bound_bps,
                _ => None,
            })
            .expect("the program declares a hedge")
    }

    /// The three spellings of one basis point mean the same basis point.
    #[test]
    fn the_delta_bound_is_read_from_a_percentage_a_count_or_a_count_with_its_unit() {
        for source in [
            "atomic_hedge {\n    buy 1_000 ethereum.ETH spot;\n    short equivalent ethereum.ETH \
             perp;\n    require delta <= 0.01%;\n}\n",
            "atomic_hedge {\n    buy 1_000 ethereum.ETH spot;\n    short equivalent ethereum.ETH \
             perp;\n    require delta <= 1;\n}\n",
            "atomic_hedge {\n    buy 1_000 ethereum.ETH spot;\n    short equivalent ethereum.ETH \
             perp;\n    require delta <= 1 bps;\n}\n",
        ] {
            assert_eq!(bound(source), 1, "one basis point, however it is written: {source}");
        }
    }

    /// `x3c fmt` must not produce a hedge the parser cannot read back. The
    /// formatter emitted `require delta <= 1 bps;`, which the delta guard rejected,
    /// so formatting an `atomic_hedge` corrupted it.
    #[test]
    fn a_formatted_hedge_parses_back_to_the_same_bound() {
        let source = "atomic_hedge {\n    buy 1_000 ethereum.ETH spot;\n    short equivalent \
                      ethereum.ETH perp;\n    require delta <= 0.01%;\n}\n";
        let formatted = crate::formatter::X3Formatter::new().format_program(&parse(source));
        assert_eq!(
            bound(&formatted),
            bound(source),
            "the formatted hedge must decide the same bound: {formatted}"
        );
        assert_eq!(
            formatted,
            crate::formatter::X3Formatter::new().format_program(&parse(&formatted)),
            "formatting must be idempotent over a hedge"
        );
    }
}
