//! PHASE 43's audit list, one test per item.
//!
//! The phase asks for safe fixed-point types and names seven properties they have to
//! hold. Each of those is a test below rather than a claim in a module doc, so a
//! regression in the arithmetic fails the build:
//!
//! | Item | Test |
//! |---|---|
//! | overflow | [`overflow_is_refused_and_never_wraps`] |
//! | underflow | [`underflow_is_refused_rather_than_going_negative`] |
//! | rounding direction | [`rounding_direction_is_stated_and_obeyed`] |
//! | precision loss | [`exact_refuses_precision_loss`] |
//! | division by zero | [`division_by_zero_is_a_refusal_not_an_infinity`] |
//! | conversion bounds | [`conversion_bounds_are_checked`] |
//! | asset decimal conversion | [`asset_decimals_convert_in_both_directions`] |

use x3_lang_common::{pow10, Bps, Decimal, Price, Rate, Ratio, RoundingMode, MAX_SCALE};

/// `10^18`, the scale of [`Ratio`], [`Rate`] and [`Price`].
const E18: u128 = 1_000_000_000_000_000_000;
/// `10^17`, one decimal place short of the scale.
const E17: u128 = 100_000_000_000_000_000;

#[test]
fn basis_points_have_a_named_whole() {
    // Before PHASE 43 the whole was the bare literal `10_000`, written in a dozen
    // places and named in none of them. These are the figures those sites mean.
    assert_eq!(Bps::WHOLE.raw(), 10_000);
    assert_eq!(Bps::ZERO.raw(), 0);
    assert!(Bps::WHOLE.is_within_whole());
    assert!(!Bps::from_raw(10_001).is_within_whole());
    assert!(!Bps::from_raw(u32::MAX).is_within_whole());

    // A share of an amount, truncated — the conversion the fee arithmetic does.
    assert_eq!(Bps::from_raw(500).of_floor(1_000).unwrap(), 50);
    assert_eq!(Bps::from_raw(1).of_floor(9_999).unwrap(), 0);
    // The same conversion, rounded half up.
    assert_eq!(Bps::from_raw(1).of_round_half_up(9_999).unwrap(), 1);
    assert_eq!(Bps::from_raw(1).of_round_half_up(4_999).unwrap(), 0);

    // A share above a whole is not a share of anything, and refuses to become a
    // decimal rather than becoming a plausible wrong number.
    let one = Decimal::<18>::from_mantissa(E18);
    assert_eq!(Bps::WHOLE.to_decimal::<18>(), Some(one));
    assert_eq!(Bps::from_raw(10_001).to_decimal::<18>(), None);
}

#[test]
fn overflow_is_refused_and_never_wraps() {
    // Addition past the width of the mantissa.
    assert_eq!(Bps::from_raw(u32::MAX).checked_add(Bps::from_raw(1)), None);
    assert_eq!(Bps::WHOLE.checked_add(Bps::ZERO), Some(Bps::WHOLE));
    let max = Decimal::<18>::from_mantissa(u128::MAX);
    assert_eq!(max.checked_add(Decimal::from_mantissa(1)), None);

    // Multiplication past the width of the mantissa. The product is formed first and
    // *then* rescaled, so this is the multiplication that overflows, not the
    // division — a wrapping implementation would return a small, plausible number.
    let big = Decimal::<18>::from_mantissa(u128::MAX / 2);
    assert_eq!(big.checked_mul(Decimal::from_mantissa(4), RoundingMode::Down), None);

    // And a product that fits is exact rather than saturated.
    let one = Decimal::<18>::from_mantissa(E18);
    assert_eq!(one.checked_mul(one, RoundingMode::Down), Some(one));
}

#[test]
fn underflow_is_refused_rather_than_going_negative() {
    // An amount, a price and a rate in this language are non-negative, so the mantissa
    // is unsigned and there is no negative value to reach: subtracting more than there
    // is returns `None`.
    let small = Decimal::<18>::from_mantissa(3);
    let large = Decimal::<18>::from_mantissa(4);
    assert_eq!(small.checked_sub(large), None);
    assert_eq!(large.checked_sub(small), Some(Decimal::from_mantissa(1)));
    assert_eq!(Bps::ZERO.checked_sub(Bps::from_raw(1)), None);
    assert_eq!(Bps::from_raw(1).checked_sub(Bps::from_raw(1)), Some(Bps::ZERO));
}

#[test]
fn rounding_direction_is_stated_and_obeyed() {
    // A third is not representable at any fixed scale, so the direction is the whole
    // difference between the two answers.
    let one = Decimal::<18>::from_mantissa(E18);
    let three = Decimal::<18>::from_mantissa(3 * E18);
    let down = one.checked_div(three, RoundingMode::Down).unwrap();
    let up = one.checked_div(three, RoundingMode::Up).unwrap();
    assert_eq!(up.mantissa(), down.mantissa() + 1);
    assert_eq!(down.mantissa(), 333_333_333_333_333_333);

    // 1.5 at scale 1 is 1.5, and at scale 0 it is 1 or 2 depending on the direction.
    let one_point_five = Decimal::<1>::from_mantissa(15);
    assert_eq!(one_point_five.rescale::<0>(RoundingMode::Down).unwrap().mantissa(), 1);
    assert_eq!(one_point_five.rescale::<0>(RoundingMode::Up).unwrap().mantissa(), 2);

    // Widening loses nothing, so it is the same value under every direction.
    assert_eq!(
        one_point_five.rescale::<18>(RoundingMode::Exact).unwrap().mantissa(),
        15 * E17
    );

    // An exact division is the same under every direction: `Up` does not add one.
    assert_eq!(three.checked_div(three, RoundingMode::Up).unwrap().mantissa(), E18);
    assert_eq!(three.checked_div(three, RoundingMode::Down).unwrap().mantissa(), E18);
}

#[test]
fn exact_refuses_precision_loss() {
    // `Exact` is the mode that refuses rather than states the loss.
    let one = Decimal::<18>::from_mantissa(E18);
    let three = Decimal::<18>::from_mantissa(3 * E18);
    assert_eq!(one.checked_div(three, RoundingMode::Exact), None);

    let one_point_five = Decimal::<1>::from_mantissa(15);
    assert_eq!(one_point_five.rescale::<0>(RoundingMode::Exact), None);
    assert_eq!(one_point_five.to_base_units(0, RoundingMode::Exact), None);

    // A division that is exact is accepted under `Exact`.
    let half = Decimal::<18>::from_mantissa(5 * E17); // 0.5
    let three_halves = Decimal::<18>::from_mantissa(15 * E17); // 1.5
    assert_eq!(
        three_halves.checked_div(half, RoundingMode::Exact).unwrap().mantissa(),
        3 * E18
    );

    // A literal with more precision than the type holds is refused at the constructor
    // rather than truncated, because which way it rounds is the caller's decision.
    assert_eq!(Decimal::<18>::from_parts(1, "0000000000000000000"), None);
    assert!(Decimal::<18>::from_parts(1, "000000000000000000").is_some());
}

#[test]
fn division_by_zero_is_a_refusal_not_an_infinity() {
    // An infinite fee is not a fee, and a NaN price is not a price.
    let one = Decimal::<18>::from_mantissa(E18);
    let zero = Decimal::<18>::from_mantissa(0);
    assert_eq!(one.checked_div(zero, RoundingMode::Down), None);
    assert_eq!(zero.checked_div(zero, RoundingMode::Exact), None);
    assert_eq!(one.checked_div(zero, RoundingMode::Up), None);

    // Multiplication by zero is zero, which is a value rather than a refusal.
    assert_eq!(one.checked_mul(zero, RoundingMode::Down), Some(zero));
}

#[test]
fn conversion_bounds_are_checked() {
    assert_eq!(MAX_SCALE, 18);
    assert_eq!(pow10(0), Some(1));
    assert_eq!(pow10(4), Some(10_000));
    assert_eq!(pow10(18), Some(E18));
    // Above the scale the type can hold the conversion is refused rather than
    // wrapped, and `u8::MAX` is refused as readily as `19`.
    assert_eq!(pow10(19), None);
    assert_eq!(pow10(u8::MAX), None);

    // A value that cannot be an asset amount is refused rather than wrapped.
    let max = Decimal::<18>::from_mantissa(u128::MAX);
    assert_eq!(max.to_base_units(19, RoundingMode::Down), None);
    assert_eq!(Decimal::<18>::from_parts(u128::MAX, "0"), None);
    // At the scale itself nothing changes, so the largest value still fits.
    assert_eq!(max.to_base_units(18, RoundingMode::Down), Some(u128::MAX));
}

#[test]
fn asset_decimals_convert_in_both_directions() {
    // Six decimals, as a stablecoin: 1.5 is 1_500_000 base units.
    let one_point_five = Decimal::<18>::from_parts(1, "5").unwrap();
    assert_eq!(one_point_five.to_base_units(6, RoundingMode::Exact), Some(1_500_000));
    assert_eq!(
        Decimal::<18>::from_base_units(1_500_000, 6, RoundingMode::Exact),
        Some(one_point_five)
    );

    // Zero decimals: the amount is the value.
    let one = Decimal::<18>::from_mantissa(E18);
    assert_eq!(one.to_base_units(0, RoundingMode::Exact), Some(1));
    assert_eq!(Decimal::<18>::from_base_units(1, 0, RoundingMode::Exact), Some(one));

    // Eighteen decimals, which is the scale itself: the smallest value is one base
    // unit, so nothing is lost in either direction.
    let smallest = Decimal::<18>::from_mantissa(1);
    assert_eq!(smallest.to_base_units(18, RoundingMode::Exact), Some(1));
    assert_eq!(
        Decimal::<18>::from_base_units(1, 18, RoundingMode::Exact),
        Some(smallest)
    );

    // A base unit smaller than the scale cannot be represented, and `Exact` refuses it
    // where `Down` states the loss.
    assert_eq!(Decimal::<18>::from_base_units(1, 20, RoundingMode::Exact), None);
    assert_eq!(
        Decimal::<18>::from_base_units(1, 20, RoundingMode::Down),
        Some(Decimal::from_mantissa(0))
    );

    // One and a half base units of a zero-decimal asset is not a whole amount, so the
    // conversion to base units is where the direction has to be given.
    let one_and_a_half = Decimal::<18>::from_mantissa(E18 + E18 / 2);
    assert_eq!(one_and_a_half.to_base_units(0, RoundingMode::Exact), None);
    assert_eq!(one_and_a_half.to_base_units(0, RoundingMode::Down), Some(1));
    assert_eq!(one_and_a_half.to_base_units(0, RoundingMode::Up), Some(2));
}

#[test]
fn ratios_say_what_they_are_a_ratio_of() {
    // A ratio is what one unit of an input yields, so `output / input`.
    let ratio = Ratio::of(1_000, 999).unwrap();
    assert_eq!(ratio.to_bps(RoundingMode::Down).unwrap().raw(), 9_990);
    assert!(ratio.to_bps(RoundingMode::Down).unwrap().is_within_whole());

    // A gain above the whole reads above 10_000 bps rather than wrapping, which is
    // what lets a caller compare it against a ceiling.
    let gain = Ratio::of(1_000, 1_200).unwrap();
    assert_eq!(gain.to_bps(RoundingMode::Down).unwrap().raw(), 12_000);
    assert!(!gain.to_bps(RoundingMode::Down).unwrap().is_within_whole());

    // A ratio of an amount rounds as the caller asks.
    let half = Ratio::of(2, 1).unwrap();
    assert_eq!(half.of_amount(1_000, RoundingMode::Down), Some(500));
    assert_eq!(half.of_amount(1_001, RoundingMode::Down), Some(500));
    assert_eq!(half.of_amount(1_001, RoundingMode::Up), Some(501));

    // An input of zero is not a ratio of anything, and is refused. An output of zero
    // *is* a ratio — of nothing — and is the value zero.
    assert_eq!(Ratio::of(0, 1), None);
    assert!(Ratio::of(1, 0).unwrap().as_decimal().is_zero());
}

#[test]
fn a_rate_carries_the_interval_it_is_per() {
    let whole = Ratio::of(1, 1).unwrap();
    // A rate per nothing is not a rate.
    assert_eq!(Rate::new(0, whole), None);

    let rate = Rate::new(10, whole).unwrap();
    assert_eq!(rate.per_blocks(), 10);
    assert_eq!(rate.ratio(), whole);

    // Over twice the interval, twice the ratio; over half, half. The interval travels
    // with the value, which is the whole reason it is a field and not a convention.
    assert_eq!(
        rate.over(20, RoundingMode::Down)
            .unwrap()
            .to_bps(RoundingMode::Down)
            .unwrap()
            .raw(),
        20_000
    );
    assert_eq!(
        rate.over(5, RoundingMode::Down)
            .unwrap()
            .to_bps(RoundingMode::Down)
            .unwrap()
            .raw(),
        5_000
    );
}

#[test]
fn a_price_carries_the_pair_it_is_between() {
    let one_and_a_half = Ratio::of(2, 3).unwrap(); // 1.5
    let price = Price::new("ethereum.ETH", "ethereum.USDC", one_and_a_half);
    assert_eq!(price.pair(), ("ethereum.ETH", "ethereum.USDC"));
    assert_eq!(price.ratio(), one_and_a_half);

    // Two ETH, at 18 decimals, is 3 USDC at six decimals.
    assert_eq!(
        price.convert("ethereum.ETH", "ethereum.USDC", 2 * E18, 18, 6, RoundingMode::Exact),
        Some(3_000_000)
    );

    // A million ETH is not a toy amount, and it converts without overflowing: the
    // powers of ten both sides share are cancelled before the product is formed, so the
    // intermediate is as small as it can be rather than the size of the answer squared.
    assert_eq!(
        price.convert(
            "ethereum.ETH",
            "ethereum.USDC",
            1_000_000 * E18,
            18,
            6,
            RoundingMode::Exact
        ),
        Some(1_500_000_000_000)
    );

    // One wei of the base asset is worth less than a base unit of the quote asset, so
    // the direction is the caller's to state rather than the conversion's to guess.
    assert_eq!(
        price.convert("ethereum.ETH", "ethereum.USDC", 1, 18, 6, RoundingMode::Exact),
        None
    );
    assert_eq!(
        price.convert("ethereum.ETH", "ethereum.USDC", 1, 18, 6, RoundingMode::Down),
        Some(0)
    );
    assert_eq!(
        price.convert("ethereum.ETH", "ethereum.USDC", 1, 18, 6, RoundingMode::Up),
        Some(1)
    );

    // The pair asked for has to be the pair this price is about. Without the pair in
    // the value, this conversion would return three million and be silently wrong.
    assert_eq!(
        price.convert("ethereum.USDC", "ethereum.ETH", 2 * E18, 18, 6, RoundingMode::Exact),
        None
    );
}

#[test]
fn the_module_itself_holds_no_float_type() {
    // PHASE 43's first sentence is a prohibition, and about the module that *is* the
    // vocabulary it is worth asserting directly: no native floating-point type appears
    // in this arithmetic. The prose above the code is allowed to name the thing it
    // forbids, so documentation lines are skipped.
    let source = include_str!("../src/fixed.rs");
    for (number, line) in source.lines().enumerate() {
        if line.trim_start().starts_with("//") {
            continue;
        }
        assert!(
            !line.contains("f64") && !line.contains("f32"),
            "fixed.rs:{} names a native float: {line}",
            number + 1
        );
    }
}

/// The rounding decision is one function, so the language's decimal-literal conversion and
/// this module's arithmetic cannot disagree about it (TICKET-081).
///
/// `compiler/src/trading_semantic.rs::decimal_to_base_units` decides the direction from string
/// digits, because it carries up to 38 asset decimals where this module's scale is 18 — the
/// arithmetic cannot be shared across that boundary, but the decision can, and a rule stated
/// twice is a rule that drifts. `compiler/tests/test_amount_conversion_agreement.rs` walks a
/// table of cases through both and asserts they agree; this pins the rule they share.
#[test]
fn the_rounding_decision_is_one_function() {
    use x3_lang_common::fixed::apply_rounding;

    // Nothing discarded: every direction is the identity, including `Exact`.
    for rounding in [RoundingMode::Down, RoundingMode::Up, RoundingMode::Exact] {
        assert_eq!(apply_rounding(7, false, rounding), Some(7));
    }

    // Something discarded: `Down` truncates, `Up` rounds away from the remainder, and
    // `Exact` refuses rather than stating the loss.
    assert_eq!(apply_rounding(7, true, RoundingMode::Down), Some(7));
    assert_eq!(apply_rounding(7, true, RoundingMode::Up), Some(8));
    assert_eq!(apply_rounding(7, true, RoundingMode::Exact), None);

    // A rounded value that does not fit is not a value: `Up` at the top of the range
    // refuses instead of wrapping, which is the same rule `checked_*` follows everywhere
    // else in this module.
    assert_eq!(apply_rounding(u128::MAX, true, RoundingMode::Up), None);
    assert_eq!(apply_rounding(u128::MAX, false, RoundingMode::Up), Some(u128::MAX));
    assert_eq!(apply_rounding(u128::MAX, true, RoundingMode::Down), Some(u128::MAX));
}
