//! The decimal-literal conversion and the fixed-point types, compared (TICKET-081).
//!
//! PHASE 43 put the amount arithmetic in `x3_lang_common::fixed`. The language does not
//! call it: it calls `trading_semantic::decimal_to_base_units`, which parses a literal
//! string and then implements the same rounding rule a second time. This walks a table of
//! `(literal, decimals, rounding)` cases through **both** and asserts they agree where both
//! are defined.
//!
//! Two things the comparison rests on, stated so the test cannot be read as more than it is:
//!
//! - The split of a literal into whole and fraction is done here, in the test. It is the
//!   part `decimal_to_base_units` does for itself and `Decimal::from_parts` takes as
//!   arguments, so sharing it in the test is not sharing the behaviour under test.
//! - `fixed::MAX_SCALE` is 18 and the compiler's `MAX_DECIMALS` is 38, so there is a domain
//!   where only one of them is defined. That edge is asserted at the end rather than
//!   quietly skipped.

use x3_lang_common::fixed::{Decimal, RoundingMode};
use x3_lang_compiler::trading_semantic::{decimal_to_base_units, TradingTypeError};

/// Every case this test walks: a literal, the asset's decimals, and the direction.
const LITERALS: &[&str] = &[
    "0",
    "1",
    "123",
    "1_000",
    "1.5",
    "1.23",
    "0.10",
    "1.0",
    "0.000000000000000001",
    "1.234567890123456789",
    "123456789.987654321",
    "0.5",
    "0.05",
];
const DECIMALS: &[u8] = &[0, 1, 2, 6, 9, 18];
const ROUNDING: &[RoundingMode] = &[RoundingMode::Down, RoundingMode::Up, RoundingMode::Exact];

fn split(literal: &str) -> (String, String) {
    let normalized: String = literal.chars().filter(|ch| *ch != '_').collect();
    match normalized.split_once('.') {
        Some((whole, fraction)) => (whole.to_string(), fraction.to_string()),
        None => (normalized, String::new()),
    }
}

#[test]
fn the_two_conversions_agree_wherever_both_are_defined() {
    let mut compared = 0;
    for literal in LITERALS {
        for decimals in DECIMALS {
            for rounding in ROUNDING {
                let (whole, fraction) = split(literal);
                let compiler = decimal_to_base_units(literal, *decimals, *rounding);
                let typed = Decimal::<18>::from_parts(whole.parse().expect("a whole part"), &fraction)
                    .and_then(|value| value.to_base_units(*decimals, *rounding));

                match (&compiler, typed) {
                    (Ok(left), Some(right)) => {
                        assert_eq!(
                            *left, right,
                            "{literal:?} at {decimals} decimals, {rounding:?}: the compiler says \
                             {left} and the fixed-point type says {right}"
                        );
                        compared += 1;
                    }
                    (Err(TradingTypeError::PrecisionLoss), None) => {
                        compared += 1;
                    }
                    (other, typed) => panic!(
                        "{literal:?} at {decimals} decimals, {rounding:?}: the two disagree — \
                         compiler {other:?}, fixed-point {typed:?}"
                    ),
                }
            }
        }
    }
    // A table that compared nothing would pass every assertion above.
    assert_eq!(
        compared,
        LITERALS.len() * DECIMALS.len() * ROUNDING.len(),
        "every case must be compared, on one side of the agreement or the other"
    );
}

/// The edge: `MAX_SCALE` is 18 and `MAX_DECIMALS` is 38, so an asset finer than eighteen
/// decimals is a value the compiler converts and the fixed-point types cannot represent.
///
/// This is the reason the duplication cannot be closed by delegating in the obvious
/// direction, and it is asserted so the boundary is a fact in the tree rather than a note
/// in a ticket.
#[test]
fn an_asset_finer_than_the_fixed_scale_is_the_compilers_alone() {
    // Nineteen decimals: one more than the fixed scale.
    let literal = "1.0000000000000000001";
    let (whole, fraction) = split(literal);

    assert!(
        decimal_to_base_units(literal, 19, RoundingMode::Exact).is_ok(),
        "the compiler converts a nineteen-decimal asset exactly"
    );
    assert_eq!(
        Decimal::<18>::from_parts(whole.parse().expect("a whole part"), &fraction),
        None,
        "and the fixed-point type refuses the same literal, because its scale is eighteen"
    );

    // The same, at the compiler's own ceiling of thirty-eight.
    assert!(decimal_to_base_units("0.00000000000000000000000000000000000001", 38, RoundingMode::Exact).is_ok());
    assert_eq!(x3_lang_common::fixed::MAX_SCALE, 18);
}
