//! Safe fixed-point financial math — spec PHASE 43.
//!
//! "Do not use native floating-point values for consensus-sensitive financial
//! calculations. Implement safe fixed-point types." The compiler and the runtime already
//! hold to the first sentence — the only `f64` left in a consensus-sensitive crate is a
//! Solana stake-account wire field, deserialized and never computed with — and these are
//! the second: the vocabulary the arithmetic is written in.
//!
//! ## What each type is about
//!
//! - [`Bps`] — a share in basis points, one ten-thousandth. The unit the language already
//!   uses for fees, slippage, floors and portfolio weights.
//! - [`Decimal`] — a fixed-point value `mantissa / 10^SCALE`. `Decimal<18>` is the phase's
//!   example.
//! - [`Ratio`] — a dimensionless quantity: what one unit of an input yields.
//! - [`Rate`] — a [`Ratio`] **per interval**, with the interval part of the value. A rate
//!   that does not say over what is half a rate.
//! - [`Price`] — a [`Ratio`] **between two named assets**, with the pair part of the
//!   value, so a price of one pair cannot be used where another is meant.
//!
//! Those last two are the phase's own names and this is the reading they are given: a
//! ratio is the arithmetic, and a rate and a price are ratios that carry the identity they
//! are about. The reading is stated in each type's documentation, so a reader can disagree
//! with it rather than be misled by it.
//!
//! ## The audit, answered by construction and by test
//!
//! PHASE 43's audit list is not a comment here; each item is a property of these types:
//!
//! | Item | How it is answered |
//! |---|---|
//! | overflow | every operation is `checked_*` and returns `None` rather than wrapping |
//! | underflow | the mantissa is **unsigned**, so subtracting more than there is returns `None` — a negative amount is not a value this module can represent |
//! | rounding direction | every lossy operation takes a [`RoundingMode`], and `Exact` refuses a result that would lose anything |
//! | precision loss | `Exact` is the mode that refuses it; `Down` and `Up` are the modes that state it |
//! | division by zero | `checked_div` returns `None` |
//! | conversion bounds | a scale above `MAX_SCALE` and a value that does not fit `u128` are both `None` |
//! | asset decimal conversion | `from_base_units`/`to_base_units` convert between an asset's decimals and the fixed scale, with the rounding stated |
//!
//! `RoundingMode` lives here rather than in the AST because the AST is the *syntax* of a
//! program and this is the arithmetic a program's numbers are computed with; the AST
//! re-exports it, so the language's own spelling and this one are a single vocabulary.

use serde::{Deserialize, Serialize};

/// Explicit rounding direction for arithmetic that can lose precision.
///
/// Moved here from `x3-ast/src/trading.rs`, which re-exports it, so there is one
/// vocabulary: a value that rounds "down" in an amount conversion and "down" in a basis
/// point computation have to mean the same thing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RoundingMode {
    Down,
    Up,
    Exact,
}

/// The largest scale this module supports, which is the phase's own example: `10^18` is
/// the most precision a `u128` mantissa can carry exactly.
pub const MAX_SCALE: u8 = 18;

/// `10^n`, or `None` above [`MAX_SCALE`].
///
/// A `const fn` over a match rather than a table, and `None` rather than a panic: a scale
/// this module cannot represent is a refusal, and a panicking arithmetic helper would be a
/// denial of service in anything that reads a scale from a declaration.
pub const fn pow10(n: u8) -> Option<u128> {
    Some(match n {
        0 => 1,
        1 => 10,
        2 => 100,
        3 => 1_000,
        4 => 10_000,
        5 => 100_000,
        6 => 1_000_000,
        7 => 10_000_000,
        8 => 100_000_000,
        9 => 1_000_000_000,
        10 => 10_000_000_000,
        11 => 100_000_000_000,
        12 => 1_000_000_000_000,
        13 => 10_000_000_000_000,
        14 => 100_000_000_000_000,
        15 => 1_000_000_000_000_000,
        16 => 10_000_000_000_000_000,
        17 => 100_000_000_000_000_000,
        18 => 1_000_000_000_000_000_000,
        _ => return None,
    })
}

/// A share in basis points: one ten-thousandth of a whole.
///
/// The unit the language uses for every bounded fraction — a venue's fee, a slippage
/// ceiling, a profit floor, a portfolio weight — and the reason this type exists is that
/// those computations were written as bare `u32`s against a literal `10_000`, which is a
/// unit spelled in a dozen places and enforced in none of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Bps(u32);

impl Bps {
    /// A whole: 10_000 basis points.
    pub const WHOLE: Bps = Bps(10_000);
    pub const ZERO: Bps = Bps(0);

    pub const fn from_raw(bps: u32) -> Bps {
        Bps(bps)
    }

    pub const fn raw(self) -> u32 {
        self.0
    }

    /// Whether this share is a whole or less — the bound every fee and slippage ceiling
    /// is, and the one a portfolio weight has to meet exactly.
    pub const fn is_within_whole(self) -> bool {
        self.0 <= 10_000
    }

    /// This share of `amount`, rounded down. `None` on overflow.
    pub fn of_floor(self, amount: u128) -> Option<u128> {
        amount.checked_mul(u128::from(self.0)).map(|v| v / 10_000)
    }

    /// This share of `amount`, rounded half up. `None` on overflow.
    pub fn of_round_half_up(self, amount: u128) -> Option<u128> {
        let scaled = amount.checked_mul(u128::from(self.0))?;
        scaled.checked_add(5_000).map(|v| v / 10_000)
    }

    pub const fn checked_add(self, other: Bps) -> Option<Bps> {
        match self.0.checked_add(other.0) {
            Some(sum) => Some(Bps(sum)),
            None => None,
        }
    }

    pub const fn checked_sub(self, other: Bps) -> Option<Bps> {
        match self.0.checked_sub(other.0) {
            Some(difference) => Some(Bps(difference)),
            None => None,
        }
    }

    /// This share as a fixed-point decimal at `SCALE` digits, or `None` when it is more
    /// than a whole — a share above one is not a fraction of anything, and converting it
    /// would turn a wrong number into a plausible one.
    pub fn to_decimal<const SCALE: u8>(self) -> Option<Decimal<SCALE>> {
        if !self.is_within_whole() {
            return None;
        }
        let scale = pow10(SCALE)?;
        u128::from(self.0)
            .checked_mul(scale)
            .map(|v| v / 10_000)
            .map(Decimal::from_mantissa)
    }
}

/// A fixed-point value: `mantissa / 10^SCALE`.
///
/// The mantissa is **unsigned**, and that is the design rather than a limitation: an
/// amount, a price and a rate in this language are non-negative, so making the sign
/// unrepresentable is what turns the phase's "underflow" audit item into a property —
/// subtracting more than there is cannot produce a negative value, it returns `None`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Decimal<const SCALE: u8>(u128);

impl<const SCALE: u8> Decimal<SCALE> {
    pub const fn from_mantissa(mantissa: u128) -> Self {
        Decimal(mantissa)
    }

    pub const fn mantissa(self) -> u128 {
        self.0
    }

    pub const fn is_zero(self) -> bool {
        self.0 == 0
    }

    /// `whole.fraction`, with the fraction read at `SCALE` digits. A fraction **longer**
    /// than the scale is refused rather than truncated: a value with more precision than
    /// the type holds is a rounding decision, and the caller has to make it.
    pub fn from_parts(whole: u128, fraction: &str) -> Option<Self> {
        let fraction = fraction.trim();
        if fraction.len() > usize::from(SCALE) || !fraction.chars().all(|ch| ch.is_ascii_digit()) {
            return None;
        }
        let scale = pow10(SCALE)?;
        let mut padded = String::from(fraction);
        while padded.len() < usize::from(SCALE) {
            padded.push('0');
        }
        let fractional: u128 = if padded.is_empty() { 0 } else { padded.parse().ok()? };
        whole.checked_mul(scale)?.checked_add(fractional).map(Decimal)
    }

    pub const fn checked_add(self, other: Decimal<SCALE>) -> Option<Decimal<SCALE>> {
        match self.0.checked_add(other.0) {
            Some(sum) => Some(Decimal(sum)),
            None => None,
        }
    }

    /// `None` when the result would be negative, which is this module's underflow.
    pub const fn checked_sub(self, other: Decimal<SCALE>) -> Option<Decimal<SCALE>> {
        match self.0.checked_sub(other.0) {
            Some(difference) => Some(Decimal(difference)),
            None => None,
        }
    }

    /// `self * other`, rescaled back to `SCALE`, rounded as asked.
    pub fn checked_mul(self, other: Decimal<SCALE>, rounding: RoundingMode) -> Option<Decimal<SCALE>> {
        let product = self.0.checked_mul(other.0)?;
        div_rounded(product, pow10(SCALE)?, rounding).map(Decimal)
    }

    /// `self / other`, rounded as asked. `None` when `other` is zero: division by zero is a
    /// refusal and not an infinity, because an infinite fee is not a fee.
    pub fn checked_div(self, other: Decimal<SCALE>, rounding: RoundingMode) -> Option<Decimal<SCALE>> {
        if other.0 == 0 {
            return None;
        }
        let scaled = self.0.checked_mul(pow10(SCALE)?)?;
        div_rounded(scaled, other.0, rounding).map(Decimal)
    }

    /// The same value at another scale, rounded as asked. `None` when it does not fit.
    pub fn rescale<const TARGET: u8>(self, rounding: RoundingMode) -> Option<Decimal<TARGET>> {
        if TARGET >= SCALE {
            let factor = pow10(TARGET - SCALE)?;
            return self.0.checked_mul(factor).map(Decimal);
        }
        let divisor = pow10(SCALE - TARGET)?;
        div_rounded(self.0, divisor, rounding).map(Decimal)
    }

    /// The value in an asset's base units — `value * 10^decimals` — rounded as asked.
    ///
    /// `None` when the result does not fit `u128`, which is the phase's "conversion
    /// bounds" item: a value too large to be an amount is refused rather than wrapped.
    pub fn to_base_units(self, decimals: u8, rounding: RoundingMode) -> Option<u128> {
        if decimals >= SCALE {
            let factor = pow10(decimals - SCALE)?;
            self.0.checked_mul(factor)
        } else {
            div_rounded(self.0, pow10(SCALE - decimals)?, rounding)
        }
    }

    /// Read an asset's base units into this scale, rounded as asked.
    pub fn from_base_units(amount: u128, decimals: u8, rounding: RoundingMode) -> Option<Self> {
        if SCALE >= decimals {
            let factor = pow10(SCALE - decimals)?;
            amount.checked_mul(factor).map(Decimal)
        } else {
            div_rounded(amount, pow10(decimals - SCALE)?, rounding).map(Decimal)
        }
    }
}

/// `value / divisor`, rounded as asked. `None` on a zero divisor, on overflow, and when
/// `Exact` was asked for and the division would lose something.
fn div_rounded(value: u128, divisor: u128, rounding: RoundingMode) -> Option<u128> {
    if divisor == 0 {
        return None;
    }
    let quotient = value / divisor;
    let remainder = value % divisor;
    match rounding {
        RoundingMode::Down => Some(quotient),
        RoundingMode::Up => {
            if remainder == 0 {
                Some(quotient)
            } else {
                quotient.checked_add(1)
            }
        }
        RoundingMode::Exact => {
            if remainder == 0 {
                Some(quotient)
            } else {
                None
            }
        }
    }
}

/// `10^n` for the intermediate powers this module needs, which reach past [`MAX_SCALE`]
/// because a conversion between two assets' decimals composes two scales.
///
/// Still `None` rather than a panic when the power does not fit a `u128`, for the reason
/// [`pow10`] gives.
fn pow10_ext(n: u32) -> Option<u128> {
    let mut value: u128 = 1;
    let mut remaining = n;
    while remaining > 0 {
        let step = remaining.min(u32::from(MAX_SCALE)) as u8;
        value = value.checked_mul(pow10(step)?)?;
        remaining -= u32::from(step);
    }
    Some(value)
}

/// A dimensionless ratio: what one unit of an input yields, at 18 digits.
///
/// The arithmetic of profit and slippage: `output / input` is a [`Ratio`], and a ratio is
/// what a basis-point floor is compared against.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Ratio(Decimal<18>);

impl Ratio {
    pub const fn new(value: Decimal<18>) -> Self {
        Ratio(value)
    }

    /// `output / input`, rounded down, or `None` when the input is zero or the ratio
    /// does not fit.
    ///
    /// One scaling, not two: the mantissa is `output × 10^18 / input`, computed as that.
    /// Scaling both sides to 18 digits and then dividing would multiply two 18-digit
    /// mantissas and overflow for amounts of a few hundred base units — which is every
    /// amount this language actually writes.
    pub fn of(input: u128, output: u128) -> Option<Self> {
        if input == 0 {
            return None;
        }
        let scaled = output.checked_mul(pow10(18)?)?;
        div_rounded(scaled, input, RoundingMode::Down).map(|mantissa| Ratio(Decimal::from_mantissa(mantissa)))
    }

    pub const fn as_decimal(self) -> Decimal<18> {
        self.0
    }

    /// The ratio in basis points, rounded as asked.
    pub fn to_bps(self, rounding: RoundingMode) -> Option<Bps> {
        let bps = self.0.checked_mul(Decimal::from_mantissa(10_000), rounding)?;
        u32::try_from(bps.mantissa()).ok().map(Bps::from_raw)
    }

    /// This ratio of `amount`, rounded as asked.
    ///
    /// `mantissa × amount / 10^18` as integers rather than as two scaled values
    /// multiplied together, for the reason [`Ratio::of`] gives.
    pub fn of_amount(self, amount: u128, rounding: RoundingMode) -> Option<u128> {
        let scaled = self.0.mantissa().checked_mul(amount)?;
        div_rounded(scaled, pow10(18)?, rounding)
    }
}

/// A [`Ratio`] **per interval**: how much accrues over a stated number of blocks.
///
/// The interval is part of the value because a rate that does not say over what is half a
/// rate — "0.25" is a very different fee over one block and over a million.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rate {
    /// The interval the ratio is per, in blocks. Zero is refused by `new`, because a rate
    /// per nothing is not a rate.
    per_blocks: u32,
    ratio: Ratio,
}

impl Rate {
    pub fn new(per_blocks: u32, ratio: Ratio) -> Option<Self> {
        if per_blocks == 0 {
            return None;
        }
        Some(Rate { per_blocks, ratio })
    }

    pub const fn per_blocks(self) -> u32 {
        self.per_blocks
    }

    pub const fn ratio(self) -> Ratio {
        self.ratio
    }

    /// The ratio over `blocks`, rounded as asked.
    ///
    /// `ratio × blocks / per_blocks` as integers, so the interval cancels before the
    /// result is rescaled and the intermediate cannot overflow on an ordinary block
    /// count.
    pub fn over(self, blocks: u32, rounding: RoundingMode) -> Option<Ratio> {
        let scaled = self.ratio.as_decimal().mantissa().checked_mul(u128::from(blocks))?;
        div_rounded(scaled, u128::from(self.per_blocks), rounding)
            .map(|mantissa| Ratio::new(Decimal::from_mantissa(mantissa)))
    }
}

/// A [`Ratio`] **between two named assets**: what one unit of `base` is worth in `quote`.
///
/// The pair is part of the value, so a price of one pair cannot be used where another is
/// meant — the same reasoning that puts the interval in [`Rate`]. This compiler constructs
/// no `Price` of its own, and that is deliberate rather than an omission: it has no price
/// source (the opportunity graph holds venue attributes and no price — PHASE 37), so a
/// price can only come from a host or a declaration, and this is the type it arrives in
/// rather than an `f64` that nothing can check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Price {
    base: String,
    quote: String,
    ratio: Ratio,
}

impl Price {
    pub fn new(base: impl Into<String>, quote: impl Into<String>, ratio: Ratio) -> Self {
        Price {
            base: base.into(),
            quote: quote.into(),
            ratio,
        }
    }

    pub fn pair(&self) -> (&str, &str) {
        (self.base.as_str(), self.quote.as_str())
    }

    pub const fn ratio(&self) -> Ratio {
        self.ratio
    }

    /// What `amount` of base units of `base` is worth, in base units of `quote`.
    ///
    /// `None` when the pair asked for is not the pair this price is about — which is the
    /// whole reason the pair is carried — or when the conversion does not fit.
    ///
    /// The conversion is one integer division,
    ///
    /// ```text
    /// amount × ratio_mantissa × 10^quote_decimals / 10^(18 + base_decimals)
    /// ```
    ///
    /// with the powers of ten both sides share cancelled first — the ratio's trailing
    /// zeros, then any the amount has. Cancelling is exact, so there is still a single
    /// rounding and it is the one the caller asked for; and the product stays inside a
    /// `u128` for the amounts a program actually writes, rather than overflowing at a
    /// few hundred units the way the uncancelled product does.
    pub fn convert(
        &self,
        base: &str,
        quote: &str,
        amount: u128,
        base_decimals: u8,
        quote_decimals: u8,
        rounding: RoundingMode,
    ) -> Option<u128> {
        if base != self.base || quote != self.quote {
            return None;
        }

        // `10^quote_decimals` sits in the numerator and `10^18 × 10^base_decimals` in the
        // denominator; the smaller of the two is cancelled outright.
        let mut numerator_power = u32::from(quote_decimals);
        let mut denominator_power = u32::from(MAX_SCALE) + u32::from(base_decimals);
        let shared = numerator_power.min(denominator_power);
        numerator_power -= shared;
        denominator_power -= shared;

        // Then the trailing zeros of the mantissa and of the amount, which is where the
        // remaining headroom comes from. Each step divides the denominator by the same
        // ten it divides a factor by, so the quotient is unchanged.
        let mut mantissa = self.ratio.as_decimal().mantissa();
        let mut amount = amount;
        while denominator_power > 0 {
            if mantissa % 10 == 0 {
                mantissa /= 10;
            } else if amount % 10 == 0 {
                amount /= 10;
            } else {
                break;
            }
            denominator_power -= 1;
        }

        let numerator = amount.checked_mul(mantissa)?.checked_mul(pow10_ext(numerator_power)?)?;
        div_rounded(numerator, pow10_ext(denominator_power)?, rounding)
    }
}
