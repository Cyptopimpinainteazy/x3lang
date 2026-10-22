//! Constant-product pricing — the formula `pallet_x3_dex` settles with, in integers.
//!
//! `fee = floor(amount_in * fee_bps / 10_000)`, `net = amount_in - fee`,
//! `out = floor(net * reserve_out / (reserve_in + net))`. The fee stays in the pool (the whole input
//! is added to `reserve_in`). Products are taken in 256 bits so no intermediate can overflow; a
//! result that does not fit `u128` is refused, never truncated.

/// Basis-point denominator.
pub const BPS: u128 = 10_000;

/// Why a price could not be computed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AmmError {
    ZeroInput,
    EmptyPool,
    Overflow,
    /// The output would drain the pool.
    InsufficientLiquidity,
    FeeOutOfRange,
}

/// `floor(a * b / d)` with a 256-bit intermediate, or `None` when `d == 0` or the quotient does not
/// fit `u128`.
pub fn mul_div(a: u128, b: u128, d: u128) -> Option<u128> {
    if d == 0 {
        return None;
    }
    let (hi, lo) = mul_wide(a, b);
    div_wide(hi, lo, d)
}

/// `ceil(a * b / d)`, for amounts charged *to* a trader: rounding a fee down rounds it in the
/// payer's favour, so fees owed to a provider round up.
pub fn mul_div_ceil(a: u128, b: u128, d: u128) -> Option<u128> {
    if d == 0 {
        return None;
    }
    let (hi, lo) = mul_wide(a, b);
    let floor = div_wide(hi, lo, d)?;
    let (check_hi, check_lo) = mul_wide(floor, d);
    if (check_hi, check_lo) == (hi, lo) {
        Some(floor)
    } else {
        floor.checked_add(1)
    }
}

fn mul_wide(a: u128, b: u128) -> (u128, u128) {
    const MASK: u128 = u64::MAX as u128;
    let (a_hi, a_lo) = (a >> 64, a & MASK);
    let (b_hi, b_lo) = (b >> 64, b & MASK);
    let lo_lo = a_lo * b_lo;
    let hi_lo = a_hi * b_lo;
    let lo_hi = a_lo * b_hi;
    let hi_hi = a_hi * b_hi;
    let cross = (lo_lo >> 64) + (hi_lo & MASK) + (lo_hi & MASK);
    let lo = (cross << 64) | (lo_lo & MASK);
    let hi = hi_hi + (hi_lo >> 64) + (lo_hi >> 64) + (cross >> 64);
    (hi, lo)
}

/// `(hi:lo) / d` by restoring long division, `None` if the quotient exceeds `u128`.
fn div_wide(hi: u128, lo: u128, d: u128) -> Option<u128> {
    if hi >= d {
        return None;
    }
    if hi == 0 {
        return Some(lo / d);
    }
    let mut remainder = hi;
    let mut quotient: u128 = 0;
    for bit in (0..128).rev() {
        let carry = remainder >> 127;
        remainder = (remainder << 1) | ((lo >> bit) & 1);
        quotient <<= 1;
        if carry == 1 || remainder >= d {
            remainder = remainder.wrapping_sub(d);
            quotient |= 1;
        }
    }
    Some(quotient)
}

/// One constant-product swap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Swap {
    /// Output paid to the trader.
    pub amount_out: u128,
    /// Venue fee taken from the input (stays in the pool).
    pub fee: u128,
    /// The input net of the fee, which is what was priced.
    pub net_in: u128,
    /// What the input would buy at the pre-trade spot price, before fee and impact.
    pub spot_out: u128,
    /// What the input would buy with no fee: the price impact's reference.
    pub no_fee_out: u128,
    pub reserve_in_after: u128,
    pub reserve_out_after: u128,
}

/// Price `amount_in` against `(reserve_in, reserve_out)` at `fee_bps`.
pub fn swap(
    amount_in: u128,
    reserve_in: u128,
    reserve_out: u128,
    fee_bps: u16,
) -> Result<Swap, AmmError> {
    if amount_in == 0 {
        return Err(AmmError::ZeroInput);
    }
    if reserve_in == 0 || reserve_out == 0 {
        return Err(AmmError::EmptyPool);
    }
    if u128::from(fee_bps) > BPS {
        return Err(AmmError::FeeOutOfRange);
    }
    let fee = mul_div(amount_in, u128::from(fee_bps), BPS).ok_or(AmmError::Overflow)?;
    let net_in = amount_in - fee;
    let denominator = reserve_in.checked_add(net_in).ok_or(AmmError::Overflow)?;
    let amount_out = mul_div(net_in, reserve_out, denominator).ok_or(AmmError::Overflow)?;
    if amount_out >= reserve_out {
        return Err(AmmError::InsufficientLiquidity);
    }
    let spot_out = mul_div(amount_in, reserve_out, reserve_in).ok_or(AmmError::Overflow)?;
    let no_fee_denominator = reserve_in
        .checked_add(amount_in)
        .ok_or(AmmError::Overflow)?;
    let no_fee_out =
        mul_div(amount_in, reserve_out, no_fee_denominator).ok_or(AmmError::Overflow)?;
    Ok(Swap {
        amount_out,
        fee,
        net_in,
        spot_out,
        no_fee_out,
        reserve_in_after: reserve_in
            .checked_add(amount_in)
            .ok_or(AmmError::Overflow)?,
        reserve_out_after: reserve_out - amount_out,
    })
}

/// `shortfall / reference` in basis points, rounded **up** so a ceiling check never passes a figure
/// that is over it by a fraction of a basis point. Zero when there is no shortfall.
pub fn shortfall_bps(reference: u128, actual: u128) -> Option<u128> {
    if actual >= reference {
        return Some(0);
    }
    mul_div_ceil(reference - actual, BPS, reference.max(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mul_div_is_exact_where_u128_would_overflow() {
        assert_eq!(mul_div(u128::MAX, u128::MAX, u128::MAX), Some(u128::MAX));
        assert_eq!(mul_div(u128::MAX, 2, 4), Some(u128::MAX / 2));
        assert_eq!(mul_div(1 << 100, 1 << 100, 1 << 90), Some(1 << 110));
        assert_eq!(mul_div(u128::MAX, u128::MAX, 1), None);
        assert_eq!(mul_div(1, 1, 0), None);
        for (a, b, d) in [(7u128, 9u128, 4u128), (1_000_003, 999_983, 17), (0, 5, 3)] {
            assert_eq!(mul_div(a, b, d), Some(a * b / d));
            assert_eq!(mul_div_ceil(a, b, d), Some((a * b).div_ceil(d)));
        }
    }

    #[test]
    fn swap_matches_the_documented_formula() {
        let s = swap(1_000, 100_000, 50_000, 30).expect("prices");
        assert_eq!(s.fee, 3);
        assert_eq!(s.net_in, 997);
        assert_eq!(s.amount_out, 997 * 50_000 / (100_000 + 997));
        assert_eq!(s.reserve_in_after, 101_000);
        assert_eq!(s.reserve_out_after, 50_000 - s.amount_out);
        assert_eq!(s.spot_out, 500);
        assert!(s.amount_out < s.no_fee_out && s.no_fee_out < s.spot_out);
    }

    #[test]
    fn swap_refuses_degenerate_inputs() {
        assert_eq!(swap(0, 1, 1, 0), Err(AmmError::ZeroInput));
        assert_eq!(swap(1, 0, 1, 0), Err(AmmError::EmptyPool));
        assert_eq!(swap(1, 1, 1, 10_001), Err(AmmError::FeeOutOfRange));
    }

    #[test]
    fn shortfall_rounds_against_the_check() {
        assert_eq!(shortfall_bps(10_000, 9_999), Some(1));
        assert_eq!(shortfall_bps(30_000, 29_999), Some(1)); // 0.33 bps rounds up to 1
        assert_eq!(shortfall_bps(100, 120), Some(0));
    }
}
