//! Receipt-visible trap identities.
//!
//! Every way an X3BC trading execution can stop is one of these, with a stable numeric code a
//! receipt carries and a replay compares. The interpreter raises them, the verifier names the ones
//! an opcode can raise in `spec`, and nothing maps an unexpected failure to "success".

use serde::{Deserialize, Serialize};

/// Why a trading execution rolled back.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[repr(u16)]
pub enum TrapId {
    /// The executing chain is not the chain the artifact was compiled for.
    ChainMismatch = 1,
    /// A provider, venue or oracle is offered at a version the artifact does not accept.
    CapabilityVersion = 2,
    /// A capability the artifact requires is not offered by the host.
    MissingCapability = 3,
    /// The execution ran out of gas.
    GasExhausted = 4,
    /// The newest oracle observation is older than the policy's freshness bound.
    OracleStale = 5,
    /// The policy requires an oracle cross-check and no observation exists for the pair.
    OracleMissing = 6,
    /// Two oracle sources disagree by more than the policy allows.
    OracleDivergent = 7,
    /// The venue's spot price is further from the oracle than the policy allows.
    OracleDeviation = 8,
    /// The realized output fell short of the venue's spot-implied output by more than the policy
    /// allows.
    Slippage = 9,
    /// The trade's own price impact exceeds the policy ceiling.
    PriceImpact = 10,
    /// The flash provider charges more than the policy's ceiling.
    FlashFeeExceeded = 11,
    /// Net profit, after every cost, is below the compiled floor.
    ProfitBelowFloor = 12,
    /// Total cost in the policy's cost asset exceeds the policy ceiling.
    CostCeiling = 13,
    /// The host charges a cost kind the policy does not admit.
    CostKindForbidden = 14,
    /// A provider or venue refused the operation (insufficient liquidity, unknown pair).
    HostRefused = 15,
    /// A swap's output is below its compiled `min_out`.
    MinOutput = 16,
    /// The trader cannot cover a debit.
    InsufficientBalance = 17,
    /// Checked arithmetic overflowed.
    ArithmeticOverflow = 18,
    /// A checked numeric conversion did not fit its target type.
    InvalidConversion = 19,
    /// The trade executed after its compiled deadline.
    DeadlineExceeded = 20,
    /// The artifact demands private submission and the context has no private channel.
    PrivateSubmissionUnavailable = 21,
    /// A debt was still open when the trade tried to commit.
    DebtOutstanding = 22,
    /// An asset's net delta is negative where the program asserted solvency.
    Insolvent = 23,
    /// An atomic window was rolled back, which aborts the execution.
    AtomicAborted = 24,
    /// A trading opcode ran out of the order the extension requires.
    SequenceViolation = 25,
    /// The engine failed for a reason that is not a trading rule (named in the receipt by code).
    EngineFault = 26,
}

impl TrapId {
    /// Every trap, in code order.
    pub const ALL: [TrapId; 26] = [
        TrapId::ChainMismatch,
        TrapId::CapabilityVersion,
        TrapId::MissingCapability,
        TrapId::GasExhausted,
        TrapId::OracleStale,
        TrapId::OracleMissing,
        TrapId::OracleDivergent,
        TrapId::OracleDeviation,
        TrapId::Slippage,
        TrapId::PriceImpact,
        TrapId::FlashFeeExceeded,
        TrapId::ProfitBelowFloor,
        TrapId::CostCeiling,
        TrapId::CostKindForbidden,
        TrapId::HostRefused,
        TrapId::MinOutput,
        TrapId::InsufficientBalance,
        TrapId::ArithmeticOverflow,
        TrapId::InvalidConversion,
        TrapId::DeadlineExceeded,
        TrapId::PrivateSubmissionUnavailable,
        TrapId::DebtOutstanding,
        TrapId::Insolvent,
        TrapId::AtomicAborted,
        TrapId::SequenceViolation,
        TrapId::EngineFault,
    ];

    pub const fn code(self) -> u16 {
        self as u16
    }

    pub fn from_code(code: u16) -> Option<TrapId> {
        TrapId::ALL.iter().copied().find(|trap| trap.code() == code)
    }

    pub const fn name(self) -> &'static str {
        match self {
            TrapId::ChainMismatch => "chain_mismatch",
            TrapId::CapabilityVersion => "capability_version",
            TrapId::MissingCapability => "missing_capability",
            TrapId::GasExhausted => "gas_exhausted",
            TrapId::OracleStale => "oracle_stale",
            TrapId::OracleMissing => "oracle_missing",
            TrapId::OracleDivergent => "oracle_divergent",
            TrapId::OracleDeviation => "oracle_deviation",
            TrapId::Slippage => "slippage",
            TrapId::PriceImpact => "price_impact",
            TrapId::FlashFeeExceeded => "flash_fee_exceeded",
            TrapId::ProfitBelowFloor => "profit_below_floor",
            TrapId::CostCeiling => "cost_ceiling",
            TrapId::CostKindForbidden => "cost_kind_forbidden",
            TrapId::HostRefused => "host_refused",
            TrapId::MinOutput => "min_output",
            TrapId::InsufficientBalance => "insufficient_balance",
            TrapId::ArithmeticOverflow => "arithmetic_overflow",
            TrapId::InvalidConversion => "invalid_conversion",
            TrapId::DeadlineExceeded => "deadline_exceeded",
            TrapId::PrivateSubmissionUnavailable => "private_submission_unavailable",
            TrapId::DebtOutstanding => "debt_outstanding",
            TrapId::Insolvent => "insolvent",
            TrapId::AtomicAborted => "atomic_aborted",
            TrapId::SequenceViolation => "sequence_violation",
            TrapId::EngineFault => "engine_fault",
        }
    }
}

impl core::fmt::Display for TrapId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "X3T{:03} {}", self.code(), self.name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_unique_dense_and_round_trip() {
        for (index, trap) in TrapId::ALL.iter().enumerate() {
            assert_eq!(usize::from(trap.code()), index + 1);
            assert_eq!(TrapId::from_code(trap.code()), Some(*trap));
        }
        assert_eq!(TrapId::from_code(0), None);
        assert_eq!(TrapId::from_code(TrapId::ALL.len() as u16 + 1), None);
    }
}
