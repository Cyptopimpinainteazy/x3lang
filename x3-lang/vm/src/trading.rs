//! Atomic trading execution and the capability-controlled host boundary.
//!
//! The VM owns accounting, guard evaluation, atomic rollback, and receipt
//! readiness. Venue and provider behavior is supplied through an explicit
//! [`TradingHost`] capability implementation.

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;

use crate::profit::Profit;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use x3_lang_compiler::ir::{AssetKey, CompiledTradingPolicy, CostKind, InvariantKind, TradingOperation, ValueRef};

/// Whether a capability manifest represents deterministic fixtures or a real
/// production integration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityMode {
    Fixture,
    Production,
}

/// Execution mode requested by the caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionMode {
    Development,
    Production,
}

/// Declared host capabilities. Fixture manifests can never satisfy production
/// execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityManifest {
    pub mode: CapabilityMode,
    pub version: String,
    pub chain: String,
    pub state_commitment: [u8; 32],
    pub private_submission: bool,
    pub providers: BTreeSet<String>,
    pub venues: BTreeSet<String>,
    pub bridges: BTreeSet<String>,
}

/// Per-execution context supplied by the caller.
/// Safety/economic policy values are compiled into the trading artifact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TradeExecutionContext {
    pub mode: ExecutionMode,
    pub current_block: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BorrowRequest {
    pub provider: String,
    pub asset: AssetKey,
    pub principal: u128,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BorrowResult {
    pub asset: AssetKey,
    pub principal: u128,
    pub fee: u128,
    pub state_commitment: [u8; 32],
}

/// A pre-execution price quote, requested before committing to a swap so
/// the VM has a reference point to measure realized slippage against.
/// `min_output` alone is an absolute floor set once at compile time; this
/// is a live figure the host is expected to refresh per call, letting the
/// VM catch "the market moved more than the policy allows" even when the
/// actual output still clears that floor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuoteRequest {
    pub venue: String,
    pub from: AssetKey,
    pub to: AssetKey,
    pub input: u128,
}

/// An independent price reading, reported alongside the primary quote for
/// oracle-firewall cross-checking. What "independent" means is a host
/// concern (a second venue's pool, a TWAP, a signed off-chain feed) — the
/// VM only ever compares numbers, it never trusts a source because of its
/// name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PriceSource {
    pub name: String,
    pub expected_output: u128,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuoteResult {
    pub expected_output: u128,
    /// Independent readings for the same route/input, for policies that
    /// declare `max_oracle_deviation`. Empty means the host has no
    /// cross-check data — which is only a problem if the policy actually
    /// requires the check; see `enforce_oracle_firewall`.
    pub sources: Vec<PriceSource>,
    /// The block the quote was taken at, as reported by the host. Required, not
    /// optional: a policy that declares `quote_freshness` cannot enforce a
    /// ceiling without it, and a host that cannot say when its price was valid
    /// has no business satisfying a policy that bounds quote age. The VM
    /// measures age against the caller-supplied
    /// `TradeExecutionContext::current_block`; the caller is responsible for
    /// tying that block to the clock of the quote's own chain.
    pub quote_block: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwapRequest {
    pub venue: String,
    pub from: AssetKey,
    pub to: AssetKey,
    pub input: u128,
    pub min_output: u128,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwapResult {
    pub from: AssetKey,
    pub to: AssetKey,
    pub input: u128,
    pub output: u128,
    pub fee: u128,
    pub fee_asset: AssetKey,
    pub state_commitment: [u8; 32],
}

/// A cross-chain transfer request. No `min_output`/slippage: a bridge
/// transfer is proven by a cryptographic inclusion/finality proof at
/// settlement time, not a venue quote — see `x3_lang_vm::bridge` for the
/// real proof-verification machinery a production host is expected to use
/// to actually fulfill this.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeRequest {
    pub via: String,
    pub from: AssetKey,
    pub to: AssetKey,
    pub input: u128,
    pub receiver: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeTransferResult {
    pub from: AssetKey,
    pub to: AssetKey,
    pub input: u128,
    pub output: u128,
    pub fee: u128,
    pub fee_asset: AssetKey,
    pub receiver: String,
    pub state_commitment: [u8; 32],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepayRequest {
    pub debt_id: String,
    pub asset: AssetKey,
    pub amount: u128,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepayResult {
    pub debt_id: String,
    pub asset: AssetKey,
    pub amount_paid: u128,
    pub fee: u128,
    pub state_commitment: [u8; 32],
}

/// A cost committed by the host that must be included in net-profit checks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommittedCost {
    pub asset: AssetKey,
    pub amount: u128,
    pub kind: String,
}

/// Host failure type. Production hosts should map transport/provider errors
/// into explicit, stable variants.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostError {
    pub code: String,
    pub message: String,
}

impl fmt::Display for HostError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl Error for HostError {}

/// Capability-controlled external venue/provider boundary.
pub trait TradingHost {
    fn capabilities(&self) -> &CapabilityManifest;

    /// Begin a host-side transaction for an atomic trade.
    ///
    /// Production adapters must stage or journal external side effects after
    /// this call so a later VM rejection can roll them back.
    fn begin_transaction(&mut self) -> Result<(), HostError> {
        Ok(())
    }

    /// Commit the host-side transaction after every VM invariant has passed.
    fn commit_transaction(&mut self) -> Result<(), HostError> {
        Ok(())
    }

    /// Roll back every staged host-side side effect for the current trade.
    fn rollback_transaction(&mut self) -> Result<(), HostError> {
        Ok(())
    }

    fn open_debt(&mut self, request: BorrowRequest) -> Result<BorrowResult, HostError>;
    /// Return a live reference price for a prospective swap, requested
    /// immediately before the swap itself so the VM can measure realized
    /// slippage against it. Required, not optional: a host with no real
    /// pricing source has no business claiming to satisfy a policy that
    /// declares a slippage ceiling.
    fn quote(&self, request: QuoteRequest) -> Result<QuoteResult, HostError>;
    fn swap(&mut self, request: SwapRequest) -> Result<SwapResult, HostError>;
    fn close_debt(&mut self, request: RepayRequest) -> Result<RepayResult, HostError>;
    fn execution_costs(&self) -> Result<Vec<CommittedCost>, HostError>;

    /// Move a settled amount to another chain through a bridge. Defaults
    /// to a clear, explicit rejection: unlike `quote`/`swap`, most trades
    /// never bridge at all, so a host with no bridging capability
    /// shouldn't need to change to keep supporting everything else — but
    /// a trade that *does* try to bridge against a host that can't must
    /// fail closed, not silently no-op as if it succeeded. A real
    /// production host is expected to fulfill this by delegating to
    /// `x3_lang_vm::bridge`'s real proof-verifying `BridgeAdapter`
    /// machinery, not by inventing its own verification.
    fn bridge(&mut self, request: BridgeRequest) -> Result<BridgeTransferResult, HostError> {
        let _ = request;
        Err(HostError {
            code: "X3_BRIDGE_NOT_SUPPORTED".to_string(),
            message: "this host does not implement cross-chain bridging".to_string(),
        })
    }
}

/// Explicit VM-side trading execution errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TradingExecError {
    NonProductionCapability,
    CapabilityChainMismatch {
        expected: String,
        actual: String,
    },
    CapabilityVersionMismatch {
        expected: u16,
        actual: String,
    },
    PrivateSubmissionRequired,
    InconsistentSubmissionProfile,
    UnknownCapability(String),
    UnsupportedOperation(String),
    InvalidSequence(String),
    HostRejected(HostError),
    AssetMismatch(String),
    OutputBelowMinOut {
        minimum: u128,
        actual: u128,
    },
    StateCommitmentMismatch,
    DeadlineExpired {
        current: u64,
        deadline: u64,
    },
    FeeCeilingExceeded {
        ceiling_bps: u16,
        actual_bps: u128,
    },
    SlippageExceeded {
        ceiling_bps: u16,
        actual_bps: u128,
    },
    /// Policy requires an oracle-firewall check (`max_oracle_deviation` is
    /// set) but the host reported zero independent price sources. Declaring
    /// the requirement and then not being able to satisfy it fails closed,
    /// the same way an unclaimed PrivateSubmissionRequired does.
    OracleFirewallUnsatisfied,
    OracleDeviationExceeded {
        source: String,
        ceiling_bps: u16,
        actual_bps: u128,
    },
    /// The venue quote the trade was about to be priced from is older than the
    /// compiled `quote_freshness` ceiling allows. Trading against a stale price
    /// is how a "profitable" route settles at a loss, so this fails closed
    /// before the swap is attempted rather than after.
    QuoteStale {
        age_blocks: u64,
        ceiling_blocks: u64,
    },
    OpenDebtAtCommit(String),
    /// A compiled policy declares a minimum net profit and does not say which
    /// asset it is denominated in.
    ///
    /// The floor cannot be checked against an unnamed asset, and checking it
    /// against "whichever asset did best" is what the missing field used to
    /// cause.
    NetProfitFloorWithoutAsset,
    /// The assembled profit decomposition and the recorded net delta disagree.
    ///
    /// They are two computations of one quantity — `net = credits - debits -
    /// costs` is an identity of this accounting — so a disagreement means a
    /// movement or a cost was recorded in one place and not the other, and any
    /// profit figure taken from either side would be a guess.
    ProfitReconciliationMismatch {
        asset: AssetKey,
        assembled: i128,
        recorded: i128,
    },
    NetProfitBelowFloor {
        minimum: i128,
        actual: i128,
    },
    MissingReceipt,
    AccountingOverflow,
    InvariantViolated {
        kind: InvariantKind,
        asset: AssetKey,
        deficit: i128,
    },
    GasCeilingExceeded {
        asset: AssetKey,
        ceiling: u128,
        actual: u128,
    },
    /// Committing this trade would push the VM instance's running realized
    /// total for `asset` — summed across every trade it has already
    /// committed under a policy declaring `max_cumulative_loss` — below
    /// `-ceiling`. A cross-trade circuit breaker, not a per-trade guard.
    CumulativeLossCeilingExceeded {
        asset: AssetKey,
        ceiling: u128,
        projected_loss: u128,
    },
    /// A host (or a receipt) reported a cost category that is not one of the
    /// known `CostKind` values. Fail closed: an unclassifiable cost cannot be
    /// checked against the policy's `allowed_cost_kinds` allowlist, so
    /// accepting it would let a host escape that allowlist by inventing a
    /// category name.
    UnknownCostKind(String),
    /// A cost was committed in a category the compiled policy's
    /// `allowed_cost_kinds` allowlist does not permit. Enforced for both
    /// host-reported execution costs and the fees the VM itself accrues for
    /// borrow/swap/bridge/repay legs, so a venue cannot bypass the allowlist
    /// by folding an unlisted cost into a leg fee.
    CostKindNotAllowed {
        kind: CostKind,
        asset: AssetKey,
    },
}

impl fmt::Display for TradingExecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonProductionCapability => write!(f, "production execution rejected fixture capabilities"),
            Self::CapabilityChainMismatch { expected, actual } => {
                write!(
                    f,
                    "compiled policy chain '{expected}' does not match host chain '{actual}'"
                )
            }
            Self::CapabilityVersionMismatch { expected, actual } => {
                write!(
                    f,
                    "compiled policy version {expected} is not supported by host version '{actual}'"
                )
            }
            Self::PrivateSubmissionRequired => write!(f, "compiled policy requires private submission capability"),
            Self::InconsistentSubmissionProfile => {
                write!(f, "legacy private-submission flag does not match submission profile")
            }
            Self::UnknownCapability(capability) => write!(f, "unknown capability '{capability}'"),
            Self::UnsupportedOperation(operation) => write!(f, "unsupported operation '{operation}'"),
            Self::InvalidSequence(message) => write!(f, "invalid atomic sequence: {message}"),
            Self::HostRejected(error) => write!(f, "host rejected the operation: {error}"),
            Self::AssetMismatch(message) => write!(f, "asset mismatch: {message}"),
            Self::OutputBelowMinOut { minimum, actual } => {
                write!(f, "swap output {actual} is below min_out {minimum}")
            }
            Self::StateCommitmentMismatch => write!(f, "host state commitment does not match the manifest"),
            Self::DeadlineExpired { current, deadline } => {
                write!(f, "trade deadline expired at block {deadline} (current {current})")
            }
            Self::FeeCeilingExceeded {
                ceiling_bps,
                actual_bps,
            } => {
                write!(f, "host fee {actual_bps} bps exceeds ceiling {ceiling_bps} bps")
            }
            Self::SlippageExceeded {
                ceiling_bps,
                actual_bps,
            } => {
                write!(
                    f,
                    "realized slippage {actual_bps} bps exceeds compiled ceiling {ceiling_bps} bps"
                )
            }
            Self::OracleFirewallUnsatisfied => write!(
                f,
                "compiled policy requires oracle-deviation cross-checking but the host reported no independent price sources"
            ),
            Self::OracleDeviationExceeded {
                source,
                ceiling_bps,
                actual_bps,
            } => write!(
                f,
                "price source '{source}' deviates {actual_bps} bps from the primary quote, exceeding the compiled ceiling {ceiling_bps} bps"
            ),
            Self::QuoteStale {
                age_blocks,
                ceiling_blocks,
            } => write!(
                f,
                "venue quote is {age_blocks} blocks old, exceeding the compiled quote_freshness ceiling of {ceiling_blocks} blocks"
            ),
            Self::OpenDebtAtCommit(debt) => write!(f, "debt '{debt}' is still open at commit"),
            Self::NetProfitFloorWithoutAsset => write!(
                f,
                "compiled policy declares a minimum net profit with no asset; the floor cannot be \
                 checked against an asset it does not name"
            ),
            Self::ProfitReconciliationMismatch {
                asset,
                assembled,
                recorded,
            } => write!(
                f,
                "profit decomposition for {}::{} is {assembled} but the recorded net delta is \
                 {recorded}; the two are different computations of the same quantity, so one of them \
                 is missing a movement or a cost",
                asset.chain, asset.symbol
            ),
            Self::NetProfitBelowFloor { minimum, actual } => {
                write!(f, "realized net profit {actual} is below floor {minimum}")
            }
            Self::MissingReceipt => write!(f, "borrowed-capital trade did not emit a receipt"),
            Self::AccountingOverflow => write!(f, "checked trading accounting overflowed"),
            Self::InvariantViolated { kind, asset, deficit } => write!(
                f,
                "invariant '{}' violated: {} nets to a deficit of {}",
                kind.as_str(),
                asset.symbol,
                -deficit
            ),
            Self::GasCeilingExceeded { asset, ceiling, actual } => write!(
                f,
                "accrued {} cost {actual} exceeds compiled max_gas ceiling {ceiling}",
                asset.symbol
            ),
            Self::CumulativeLossCeilingExceeded {
                asset,
                ceiling,
                projected_loss,
            } => write!(
                f,
                "committing this trade would bring cumulative {} losses to {projected_loss}, exceeding the compiled max_cumulative_loss ceiling {ceiling}",
                asset.symbol
            ),
            Self::UnknownCostKind(kind) => write!(
                f,
                "host reported an unknown cost kind '{kind}' that no compiled policy can classify or bound"
            ),
            Self::CostKindNotAllowed { kind, asset } => write!(
                f,
                "cost kind '{}' is not permitted by the compiled policy's allowed_cost_kinds allowlist (charged in {})",
                kind.as_str(),
                asset.symbol
            ),
        }
    }
}

impl Error for TradingExecError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DebtRecord {
    pub asset: AssetKey,
    pub principal: u128,
    pub fee: u128,
}

/// State touched by trading operations inside the atomic journal.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TradingState {
    pub balances: BTreeMap<AssetKey, u128>,
    pub open_debts: BTreeMap<String, DebtRecord>,
    pub closed_debts: BTreeSet<String>,
    pub closed_debt_records: BTreeMap<String, DebtRecord>,
    pub bindings: BTreeMap<String, AssetKey>,
    /// Per-asset rollup of every committed cost, used for the `max_gas`
    /// ceiling. Lossy by design: it cannot say which category a cost was.
    pub costs: BTreeMap<AssetKey, u128>,
    /// Append-ordered ledger of every committed cost with its category
    /// preserved. This is what receipts carry, so an independent verifier can
    /// check each cost against the policy's `allowed_cost_kinds` allowlist.
    /// Before this existed, receipts wrote the placeholder
    /// `kind: "committed"` for every cost, which made `allowed_cost_kinds`
    /// structurally unverifiable after the fact.
    pub cost_ledger: Vec<CommittedCost>,
    /// Everything credited to each asset, and everything debited from it.
    ///
    /// Recorded so the profit decomposition can be assembled from the parts and
    /// then reconciled against `net_deltas`: `net = credits - debits - costs` is
    /// an identity of this accounting, and a profit figure that disagrees with
    /// the recorded delta means a movement or a cost was written in one place and
    /// not the other.
    pub credits: BTreeMap<AssetKey, u128>,
    pub debits: BTreeMap<AssetKey, u128>,
    pub net_deltas: BTreeMap<AssetKey, i128>,
    pub receipt_emitted: bool,
    pub committed: bool,
}

/// Minimal VM-side atomic journal over trading state.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TradingVm {
    pub trading_state: TradingState,
    expected_commitment: Option<[u8; 32]>,
    compiled_policy: Option<CompiledTradingPolicy>,
    /// Realized net proceeds, per asset, summed across every trade this VM
    /// instance has actually committed (never a simulated or failed one).
    /// Lives for the lifetime of the `TradingVm`, not any single trade —
    /// this is the state a cross-trade circuit breaker like
    /// `max_cumulative_loss` reads and updates. A fresh `TradingVm::new()`
    /// starts a fresh strategy session with an empty ledger.
    cumulative_realized: BTreeMap<AssetKey, i128>,
}

/// Result of a successful atomic trading execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TradeExecution {
    pub committed_state: TradingState,
}

impl TradingVm {
    pub fn new() -> Self {
        Self::default()
    }

    /// Execute a lowered trading operation sequence atomically. On success,
    /// the host-side transaction is committed for real.
    pub fn execute_atomic(
        &mut self,
        operations: &[TradingOperation],
        host: &mut dyn TradingHost,
        context: TradeExecutionContext,
    ) -> Result<TradeExecution, TradingExecError> {
        self.run_atomic(operations, host, context, true)
    }

    /// Run a trade through the exact same policy validation, host calls,
    /// and guard checks as `execute_atomic` — but never let it land.
    /// `host.commit_transaction()` is never called, `host.rollback_transaction()`
    /// is always called, and `self.trading_state` is always restored to its
    /// pre-call snapshot, whether the guards passed or failed.
    ///
    /// This is a real dry run against the real host, not a synthetic
    /// re-implementation: it exercises the same `TradingHost` staging
    /// contract (`begin_transaction`/`rollback_transaction`) that
    /// production adapters already have to support for rollback-on-
    /// rejection, so the projected `TradeExecution` returned on success
    /// reflects exactly what would have committed. Useful for previewing a
    /// trade — or screening many candidate routes — without paying for a
    /// real settlement or risking a host-side transaction actually
    /// landing.
    ///
    /// `committed_state.committed` on the returned execution reflects that
    /// the trade's own `CommitAtomicTrade` guard passed, not that any host
    /// funds moved: nothing a simulation returns is final.
    pub fn simulate_atomic(
        &mut self,
        operations: &[TradingOperation],
        host: &mut dyn TradingHost,
        context: TradeExecutionContext,
    ) -> Result<TradeExecution, TradingExecError> {
        self.run_atomic(operations, host, context, false)
    }

    fn run_atomic(
        &mut self,
        operations: &[TradingOperation],
        host: &mut dyn TradingHost,
        context: TradeExecutionContext,
        commit: bool,
    ) -> Result<TradeExecution, TradingExecError> {
        let manifest = host.capabilities();
        if context.mode == ExecutionMode::Production && manifest.mode != CapabilityMode::Production {
            return Err(TradingExecError::NonProductionCapability);
        }
        let first = operations
            .first()
            .ok_or_else(|| TradingExecError::InvalidSequence("empty trading program".to_string()))?;
        let policy = match first {
            TradingOperation::BeginAtomicTrade { policy, .. } => policy,
            _ => {
                return Err(TradingExecError::InvalidSequence(
                    "trading program must begin with BeginAtomicTrade".to_string(),
                ))
            }
        };
        self.validate_compiled_policy(policy, manifest)?;
        if context.current_block > policy.deadline_blocks {
            return Err(TradingExecError::DeadlineExpired {
                current: context.current_block,
                deadline: policy.deadline_blocks,
            });
        }

        let snapshot = self.trading_state.clone();
        self.expected_commitment = Some(manifest.state_commitment);
        self.compiled_policy = Some(policy.clone());

        host.begin_transaction().map_err(TradingExecError::HostRejected)?;

        let outcome = self.execute_inner(operations, host, context);
        if commit {
            match outcome {
                Ok(execution) => {
                    if let Err(error) = host.commit_transaction() {
                        let _ = host.rollback_transaction();
                        self.trading_state = snapshot;
                        return Err(TradingExecError::HostRejected(error));
                    }
                    self.record_cumulative_realized(&execution.committed_state);
                    Ok(execution)
                }
                Err(error) => {
                    let rollback = host.rollback_transaction();
                    self.trading_state = snapshot;
                    if let Err(rollback_error) = rollback {
                        return Err(TradingExecError::HostRejected(rollback_error));
                    }
                    Err(error)
                }
            }
        } else {
            // Simulation: always roll back and restore state, regardless
            // of outcome — nothing is allowed to land. If the rollback
            // itself fails, we can't vouch the host is actually clean, so
            // that failure takes priority over handing back a projected
            // "this would have succeeded" result.
            let rollback = host.rollback_transaction();
            self.trading_state = snapshot;
            if let Err(rollback_error) = rollback {
                return Err(TradingExecError::HostRejected(rollback_error));
            }
            outcome
        }
    }

    fn execute_inner(
        &mut self,
        operations: &[TradingOperation],
        host: &mut dyn TradingHost,
        context: TradeExecutionContext,
    ) -> Result<TradeExecution, TradingExecError> {
        let mut saw_commit = false;
        for operation in operations {
            match operation {
                TradingOperation::BeginAtomicTrade { .. } => {
                    self.trading_state = TradingState::default();
                }
                TradingOperation::OpenDebt {
                    debt_id,
                    provider,
                    asset,
                    principal,
                } => {
                    if !host.capabilities().providers.contains(provider) {
                        return Err(TradingExecError::UnknownCapability(provider.clone()));
                    }
                    let result = host
                        .open_debt(BorrowRequest {
                            provider: provider.clone(),
                            asset: asset.clone(),
                            principal: *principal,
                        })
                        .map_err(TradingExecError::HostRejected)?;
                    self.check_commitment(&result.state_commitment)?;
                    if &result.asset != asset || result.principal != *principal {
                        return Err(TradingExecError::AssetMismatch(format!(
                            "borrow result for {debt_id} does not match the requested asset/principal"
                        )));
                    }
                    self.check_fee_bps(result.principal, result.fee, self.compiled_policy().max_flash_fee_bps)?;
                    self.credit(asset, result.principal)?;
                    // Borrowing is debt financing, so its fee is a
                    // flash-liquidity cost, not a venue swap fee.
                    self.accrue_cost(asset, result.fee, CostKind::FlashLiquidityFee)?;
                    self.trading_state.open_debts.insert(
                        debt_id.clone(),
                        DebtRecord {
                            asset: asset.clone(),
                            principal: result.principal,
                            fee: result.fee,
                        },
                    );
                }
                TradingOperation::ExecuteSwap {
                    binding,
                    venue,
                    from,
                    to,
                    input,
                    min_output,
                } => {
                    if !host.capabilities().venues.contains(venue) {
                        return Err(TradingExecError::UnknownCapability(venue.clone()));
                    }
                    let input_units = match input {
                        ValueRef::Literal(amount) => *amount,
                        ValueRef::Binding(name) => {
                            if let Some((debt_id, _field)) = name.split_once('.') {
                                self.trading_state
                                    .open_debts
                                    .get(debt_id)
                                    .map(|debt| debt.principal)
                                    .ok_or_else(|| {
                                        TradingExecError::InvalidSequence(format!(
                                            "binding '{name}' references unknown debt '{debt_id}'"
                                        ))
                                    })?
                            } else {
                                *self.trading_state.balances.get(from).unwrap_or(&0)
                            }
                        }
                    };
                    // Quote first, before the swap actually executes, so the
                    // reference price can't be influenced by the swap's own
                    // effects.
                    let quote = host
                        .quote(QuoteRequest {
                            venue: venue.clone(),
                            from: from.clone(),
                            to: to.clone(),
                            input: input_units,
                        })
                        .map_err(TradingExecError::HostRejected)?;
                    // Checked before `swap()`, not after: a stale price must
                    // abort the leg before the host is asked to move value, so
                    // there is no host-side side effect to unwind.
                    self.enforce_quote_freshness(&quote, context.current_block)?;
                    let result = host
                        .swap(SwapRequest {
                            venue: venue.clone(),
                            from: from.clone(),
                            to: to.clone(),
                            input: input_units,
                            min_output: *min_output,
                        })
                        .map_err(TradingExecError::HostRejected)?;
                    self.check_commitment(&result.state_commitment)?;
                    if &result.from != from || &result.to != to || result.input != input_units {
                        return Err(TradingExecError::AssetMismatch(format!(
                            "swap result for {binding} does not match the requested route"
                        )));
                    }
                    if result.output < *min_output {
                        return Err(TradingExecError::OutputBelowMinOut {
                            minimum: *min_output,
                            actual: result.output,
                        });
                    }
                    self.check_slippage_bps(
                        quote.expected_output,
                        result.output,
                        self.compiled_policy().max_slippage_bps,
                    )?;
                    if let Some(ceiling_bps) = self.compiled_policy().max_oracle_deviation_bps {
                        self.enforce_oracle_firewall(quote.expected_output, &quote.sources, ceiling_bps)?;
                    }
                    self.debit(from, result.input)?;
                    self.credit(to, result.output)?;
                    self.accrue_cost(&result.fee_asset, result.fee, CostKind::LiquidityFee)?;
                    self.trading_state.bindings.insert(binding.clone(), to.clone());
                }
                TradingOperation::Bridge {
                    via,
                    from,
                    to,
                    input,
                    receiver,
                } => {
                    if !host.capabilities().bridges.contains(via) {
                        return Err(TradingExecError::UnknownCapability(via.clone()));
                    }
                    let input_units = match input {
                        ValueRef::Literal(amount) => *amount,
                        ValueRef::Binding(name) => {
                            if let Some((debt_id, _field)) = name.split_once('.') {
                                self.trading_state
                                    .open_debts
                                    .get(debt_id)
                                    .map(|debt| debt.principal)
                                    .ok_or_else(|| {
                                        TradingExecError::InvalidSequence(format!(
                                            "binding '{name}' references unknown debt '{debt_id}'"
                                        ))
                                    })?
                            } else {
                                *self.trading_state.balances.get(from).unwrap_or(&0)
                            }
                        }
                    };
                    let result = host
                        .bridge(BridgeRequest {
                            via: via.clone(),
                            from: from.clone(),
                            to: to.clone(),
                            input: input_units,
                            receiver: receiver.clone(),
                        })
                        .map_err(TradingExecError::HostRejected)?;
                    self.check_commitment(&result.state_commitment)?;
                    if &result.from != from
                        || &result.to != to
                        || result.input != input_units
                        || result.receiver != *receiver
                    {
                        return Err(TradingExecError::AssetMismatch(format!(
                            "bridge result via {via} does not match the requested transfer"
                        )));
                    }
                    if result.output == 0 {
                        return Err(TradingExecError::AssetMismatch(format!(
                            "bridge via {via} reported zero output"
                        )));
                    }
                    self.debit(from, result.input)?;
                    self.credit(to, result.output)?;
                    // A bridge leg's fee is a cross-domain cost: the trade
                    // paid a different domain to move value.
                    self.accrue_cost(&result.fee_asset, result.fee, CostKind::CrossDomainFee)?;
                }
                TradingOperation::CloseDebt { debt_id } => {
                    let record = self
                        .trading_state
                        .open_debts
                        .get(debt_id)
                        .cloned()
                        .ok_or_else(|| TradingExecError::InvalidSequence(format!("unknown debt '{debt_id}'")))?;
                    let required = record
                        .principal
                        .checked_add(record.fee)
                        .ok_or(TradingExecError::AccountingOverflow)?;
                    let result = host
                        .close_debt(RepayRequest {
                            debt_id: debt_id.clone(),
                            asset: record.asset.clone(),
                            amount: required,
                        })
                        .map_err(TradingExecError::HostRejected)?;
                    self.check_commitment(&result.state_commitment)?;
                    if result.debt_id != *debt_id || result.asset != record.asset {
                        return Err(TradingExecError::AssetMismatch(format!(
                            "repayment result for {debt_id} does not match the debt"
                        )));
                    }
                    let paid = result
                        .amount_paid
                        .checked_add(result.fee)
                        .ok_or(TradingExecError::AccountingOverflow)?;
                    if paid < required {
                        return Err(TradingExecError::AssetMismatch(format!(
                            "repayment for {debt_id} is below principal plus fee"
                        )));
                    }
                    self.debit(&record.asset, paid)?;
                    // Closing a debt is the other half of the same
                    // financing instrument opened above.
                    self.accrue_cost(&record.asset, result.fee, CostKind::FlashLiquidityFee)?;
                    self.trading_state.open_debts.remove(debt_id);
                    self.trading_state.closed_debts.insert(debt_id.clone());
                    self.trading_state.closed_debt_records.insert(
                        debt_id.clone(),
                        DebtRecord {
                            asset: record.asset,
                            principal: record.principal,
                            fee: record.fee,
                        },
                    );
                }
                TradingOperation::AssertMinNetProfit {
                    settlement_asset,
                    minimum,
                } => {
                    self.accrue_host_execution_costs(host)?;
                    let actual = self.net_profit(settlement_asset)?;
                    let minimum = i128::try_from(*minimum).map_err(|_| TradingExecError::AccountingOverflow)?;
                    if actual < minimum {
                        return Err(TradingExecError::NetProfitBelowFloor { minimum, actual });
                    }
                }
                TradingOperation::AssertAllDebtsClosed => {
                    if let Some((debt, _)) = self.trading_state.open_debts.iter().next() {
                        return Err(TradingExecError::OpenDebtAtCommit(debt.clone()));
                    }
                }
                TradingOperation::AssertInvariant { kind } => {
                    self.accrue_host_execution_costs(host)?;
                    match kind {
                        InvariantKind::Solvent => {
                            if let Some((asset, deficit)) =
                                self.trading_state.net_deltas.iter().find(|(_, delta)| **delta < 0)
                            {
                                return Err(TradingExecError::InvariantViolated {
                                    kind: *kind,
                                    asset: asset.clone(),
                                    deficit: *deficit,
                                });
                            }
                        }
                    }
                }
                TradingOperation::EmitTradeReceipt => {
                    self.trading_state.receipt_emitted = true;
                }
                TradingOperation::CommitAtomicTrade => {
                    if let Some((debt, _)) = self.trading_state.open_debts.iter().next() {
                        return Err(TradingExecError::OpenDebtAtCommit(debt.clone()));
                    }
                    // Guaranteed, not conditional: a trade using a
                    // policy-level minimum_net_profit instead of an
                    // explicit `require net_profit`/`invariant solvent`
                    // statement would otherwise never call
                    // accrue_host_execution_costs, and the gas ceiling
                    // would silently never be checked.
                    self.accrue_host_execution_costs(host)?;
                    self.trading_state.committed = true;
                    saw_commit = true;
                }
                TradingOperation::AbortAtomicTrade => {
                    return Err(TradingExecError::HostRejected(HostError {
                        code: "X3_TRADING_ABORTED".to_string(),
                        message: "trade aborted by explicit AbortAtomicTrade".to_string(),
                    }));
                }
            }
        }
        if !saw_commit {
            return Err(TradingExecError::InvalidSequence(
                "trading program did not commit".to_string(),
            ));
        }
        if !self.trading_state.open_debts.is_empty() {
            return Err(TradingExecError::OpenDebtAtCommit(
                self.trading_state.open_debts.keys().next().cloned().unwrap_or_default(),
            ));
        }
        if !self.trading_state.closed_debts.is_empty() && !self.trading_state.receipt_emitted {
            return Err(TradingExecError::MissingReceipt);
        }
        self.accrue_host_execution_costs(host)?;
        if let Some(minimum) = self.compiled_policy().minimum_net_profit {
            // The floor is denominated in the policy's settlement asset, so the
            // check reads that asset. It used to take the *maximum* delta across
            // every asset, which passed a trade that lost on the settlement asset
            // whenever some side asset happened to gain.
            let asset = self
                .compiled_policy()
                .minimum_net_profit_asset
                .clone()
                .ok_or(TradingExecError::NetProfitFloorWithoutAsset)?;
            let actual = self.net_profit(&asset)?;
            let minimum = i128::try_from(minimum).map_err(|_| TradingExecError::AccountingOverflow)?;
            if actual < minimum {
                return Err(TradingExecError::NetProfitBelowFloor { minimum, actual });
            }
        }
        if let (Some(ceiling), Some(asset)) = (
            self.compiled_policy().max_cumulative_loss,
            self.compiled_policy().max_cumulative_loss_asset.clone(),
        ) {
            let prior = self.cumulative_realized.get(&asset).copied().unwrap_or(0);
            let this_trade = self.trading_state.net_deltas.get(&asset).copied().unwrap_or(0);
            let projected = prior
                .checked_add(this_trade)
                .ok_or(TradingExecError::AccountingOverflow)?;
            let ceiling_i128 = i128::try_from(ceiling).map_err(|_| TradingExecError::AccountingOverflow)?;
            if projected < -ceiling_i128 {
                let projected_loss = u128::try_from(-projected).map_err(|_| TradingExecError::AccountingOverflow)?;
                return Err(TradingExecError::CumulativeLossCeilingExceeded {
                    asset,
                    ceiling,
                    projected_loss,
                });
            }
        }
        Ok(TradeExecution {
            committed_state: self.trading_state.clone(),
        })
    }

    /// Merge a just-committed trade's realized deltas into the VM's
    /// cross-trade ledger. Only called from the real-commit path in
    /// `run_atomic` — never for a simulated or failed trade — so
    /// `max_cumulative_loss` only ever reflects trades that actually
    /// landed. Uses `saturating_add`: by the time this runs, the host has
    /// already committed for real, so a ledger update can no longer fail
    /// the trade; the guard check above already used `checked_add` on the
    /// one asset a policy actually cares about; a general ledger entry at
    /// the practical limits of `i128` is not a case worth failing an
    /// already-landed trade over.
    fn record_cumulative_realized(&mut self, state: &TradingState) {
        for (asset, delta) in &state.net_deltas {
            let entry = self.cumulative_realized.entry(asset.clone()).or_insert(0);
            *entry = entry.saturating_add(*delta);
        }
    }

    /// Realized net proceeds for `asset`, summed across every trade this
    /// VM instance has committed so far (0 if the asset has never been
    /// touched by a committed trade). Never affected by `simulate_atomic`.
    pub fn cumulative_realized(&self, asset: &AssetKey) -> i128 {
        self.cumulative_realized.get(asset).copied().unwrap_or(0)
    }

    fn validate_compiled_policy(
        &self,
        policy: &CompiledTradingPolicy,
        manifest: &CapabilityManifest,
    ) -> Result<(), TradingExecError> {
        if !policy.submission_profile_is_consistent() {
            return Err(TradingExecError::InconsistentSubmissionProfile);
        }
        if policy.chain != manifest.chain {
            return Err(TradingExecError::CapabilityChainMismatch {
                expected: policy.chain.clone(),
                actual: manifest.chain.clone(),
            });
        }
        let supported_version = manifest
            .version
            .strip_prefix("trading-policy-v")
            .and_then(|value| value.parse::<u16>().ok());
        if supported_version != Some(policy.policy_version) {
            return Err(TradingExecError::CapabilityVersionMismatch {
                expected: policy.policy_version,
                actual: manifest.version.clone(),
            });
        }
        if policy.require_private_submission && !manifest.private_submission {
            return Err(TradingExecError::PrivateSubmissionRequired);
        }
        Ok(())
    }

    fn compiled_policy(&self) -> &CompiledTradingPolicy {
        self.compiled_policy
            .as_ref()
            .expect("compiled policy must be set before execution")
    }

    fn accrue_host_execution_costs(&mut self, host: &dyn TradingHost) -> Result<(), TradingExecError> {
        let costs = host.execution_costs().map_err(TradingExecError::HostRejected)?;
        for cost in costs {
            // Classify before accruing. An unparseable category used to be
            // silently discarded along with the rest of the report's
            // structure; now it is a hard failure, because a cost the policy
            // cannot classify is a cost the policy cannot bound.
            let kind =
                CostKind::from_str(&cost.kind).ok_or_else(|| TradingExecError::UnknownCostKind(cost.kind.clone()))?;
            let already = self.trading_state.costs.get(&cost.asset).copied().unwrap_or(0);
            if cost.amount > already {
                self.accrue_cost(&cost.asset, cost.amount - already, kind)?;
            }
        }
        self.enforce_gas_ceiling()
    }

    /// Compiled `max_gas` used to be dead metadata — parsed, carried through
    /// IR, never actually checked against anything. This is the actual
    /// enforcement: total accrued cost in the policy's declared gas asset
    /// must not exceed the compiled ceiling.
    fn enforce_gas_ceiling(&self) -> Result<(), TradingExecError> {
        let policy = self.compiled_policy();
        let actual = self
            .trading_state
            .costs
            .get(&policy.max_gas_asset)
            .copied()
            .unwrap_or(0);
        if actual > policy.max_gas {
            return Err(TradingExecError::GasCeilingExceeded {
                asset: policy.max_gas_asset.clone(),
                ceiling: policy.max_gas,
                actual,
            });
        }
        Ok(())
    }

    fn check_commitment(&self, commitment: &[u8; 32]) -> Result<(), TradingExecError> {
        // The host result commitment must equal the manifest commitment the
        // caller supplied to the capability boundary.
        let manifest_commitment = self
            .expected_commitment
            .ok_or(TradingExecError::StateCommitmentMismatch)?;
        if commitment != &manifest_commitment {
            return Err(TradingExecError::StateCommitmentMismatch);
        }
        Ok(())
    }

    fn check_fee_bps(&self, principal: u128, fee: u128, ceiling_bps: u16) -> Result<(), TradingExecError> {
        let actual_bps = fee
            .checked_mul(10_000)
            .and_then(|value| value.checked_div(principal.max(1)))
            .ok_or(TradingExecError::AccountingOverflow)?;
        if actual_bps > ceiling_bps as u128 {
            return Err(TradingExecError::FeeCeilingExceeded {
                ceiling_bps,
                actual_bps,
            });
        }
        Ok(())
    }

    /// Realized slippage is only ever a shortfall against the quote: an
    /// `actual` at or above `expected` is zero slippage (or better than
    /// quoted), never negative. `min_output` is a separate, absolute floor
    /// checked elsewhere — this catches "still above the floor, but worse
    /// than the policy's tolerance for how far the market can move."
    fn check_slippage_bps(&self, expected: u128, actual: u128, ceiling_bps: u16) -> Result<(), TradingExecError> {
        if actual >= expected {
            return Ok(());
        }
        let shortfall = expected - actual;
        let actual_bps = shortfall
            .checked_mul(10_000)
            .and_then(|value| value.checked_div(expected.max(1)))
            .ok_or(TradingExecError::AccountingOverflow)?;
        if actual_bps > ceiling_bps as u128 {
            return Err(TradingExecError::SlippageExceeded {
                ceiling_bps,
                actual_bps,
            });
        }
        Ok(())
    }

    /// Reject a venue quote older than the compiled `quote_freshness` ceiling.
    ///
    /// Age is measured against the caller-supplied `current_block`. A quote
    /// whose block is *ahead* of `current_block` saturates to age 0 rather than
    /// being rejected: a trade's policy chain and a destination-chain venue do
    /// not share a block clock, so a quote block above the policy chain's
    /// current block is an ordinary cross-chain reading, not evidence of
    /// staleness.
    ///
    /// A policy that does not declare `quote_freshness` imposes no bound, so
    /// this is a no-op for it.
    fn enforce_quote_freshness(&self, quote: &QuoteResult, current_block: u64) -> Result<(), TradingExecError> {
        let Some(ceiling_blocks) = self.compiled_policy().quote_freshness_blocks else {
            return Ok(());
        };
        let age_blocks = current_block.saturating_sub(quote.quote_block);
        if age_blocks > ceiling_blocks {
            return Err(TradingExecError::QuoteStale {
                age_blocks,
                ceiling_blocks,
            });
        }
        Ok(())
    }

    /// Cross-check the primary quote against every independent source the
    /// host reported. Deviation is measured both directions — a source
    /// quoting *higher* than the primary is just as much a disagreement
    /// (and just as suspicious a manipulation signal) as one quoting lower.
    /// A policy that opts into this check and gets zero sources back fails
    /// closed rather than silently skipping the check it asked for.
    fn enforce_oracle_firewall(
        &self,
        primary_expected: u128,
        sources: &[PriceSource],
        ceiling_bps: u16,
    ) -> Result<(), TradingExecError> {
        if sources.is_empty() {
            return Err(TradingExecError::OracleFirewallUnsatisfied);
        }
        for source in sources {
            let diff = primary_expected.abs_diff(source.expected_output);
            let actual_bps = diff
                .checked_mul(10_000)
                .and_then(|value| value.checked_div(primary_expected.max(1)))
                .ok_or(TradingExecError::AccountingOverflow)?;
            if actual_bps > ceiling_bps as u128 {
                return Err(TradingExecError::OracleDeviationExceeded {
                    source: source.name.clone(),
                    ceiling_bps,
                    actual_bps,
                });
            }
        }
        Ok(())
    }

    fn credit(&mut self, asset: &AssetKey, amount: u128) -> Result<(), TradingExecError> {
        let entry = self.trading_state.balances.entry(asset.clone()).or_insert(0);
        *entry = entry.checked_add(amount).ok_or(TradingExecError::AccountingOverflow)?;
        let credited = self.trading_state.credits.entry(asset.clone()).or_insert(0);
        *credited = credited
            .checked_add(amount)
            .ok_or(TradingExecError::AccountingOverflow)?;
        let delta = self.trading_state.net_deltas.entry(asset.clone()).or_insert(0);
        *delta = delta
            .checked_add(i128::try_from(amount).map_err(|_| TradingExecError::AccountingOverflow)?)
            .ok_or(TradingExecError::AccountingOverflow)?;
        Ok(())
    }

    fn debit(&mut self, asset: &AssetKey, amount: u128) -> Result<(), TradingExecError> {
        let entry = self.trading_state.balances.entry(asset.clone()).or_insert(0);
        *entry = entry.checked_sub(amount).ok_or(TradingExecError::AccountingOverflow)?;
        let debited = self.trading_state.debits.entry(asset.clone()).or_insert(0);
        *debited = debited
            .checked_add(amount)
            .ok_or(TradingExecError::AccountingOverflow)?;
        let delta = self.trading_state.net_deltas.entry(asset.clone()).or_insert(0);
        *delta = delta
            .checked_sub(i128::try_from(amount).map_err(|_| TradingExecError::AccountingOverflow)?)
            .ok_or(TradingExecError::AccountingOverflow)?;
        Ok(())
    }

    /// Commit `amount` of `asset` as a cost of category `kind`.
    ///
    /// The category is validated against the compiled policy's
    /// `allowed_cost_kinds` allowlist *before* any accounting state is
    /// touched, so a rejected cost leaves balances and net deltas exactly as
    /// they were. A zero amount is not a cost and is skipped: failing a trade
    /// because a host reported a zero-amount category the policy happens not
    /// to list would be noise, not safety.
    fn accrue_cost(&mut self, asset: &AssetKey, amount: u128, kind: CostKind) -> Result<(), TradingExecError> {
        if amount == 0 {
            return Ok(());
        }
        if !self.compiled_policy().allowed_cost_kinds.contains(&kind) {
            return Err(TradingExecError::CostKindNotAllowed {
                kind,
                asset: asset.clone(),
            });
        }
        let entry = self.trading_state.costs.entry(asset.clone()).or_insert(0);
        *entry = entry.checked_add(amount).ok_or(TradingExecError::AccountingOverflow)?;
        let delta = self.trading_state.net_deltas.entry(asset.clone()).or_insert(0);
        *delta = delta
            .checked_sub(i128::try_from(amount).map_err(|_| TradingExecError::AccountingOverflow)?)
            .ok_or(TradingExecError::AccountingOverflow)?;
        self.trading_state.cost_ledger.push(CommittedCost {
            asset: asset.clone(),
            amount,
            kind: kind.as_str().to_string(),
        });
        Ok(())
    }

    /// Assemble the profit decomposition for one asset from the ledger.
    ///
    /// `gross` is what was credited to the asset and `principal` what was
    /// debited from it — in this accounting those are the proceeds and the
    /// capital committed. The safety buffer is zero because the policy declares
    /// none: the spec lists one, the language has no field for it, and a
    /// non-zero figure here would be invented.
    ///
    /// The result is reconciled against the recorded net delta before it is
    /// returned. The two must agree, and a mismatch is an error rather than a
    /// preference for one of them.
    pub fn profit(&self, asset: &AssetKey) -> Result<Profit, TradingExecError> {
        let costs: Vec<crate::profit::LedgerCost> = self
            .trading_state
            .cost_ledger
            .iter()
            .filter(|cost| &cost.asset == asset)
            .map(|cost| crate::profit::LedgerCost {
                amount: cost.amount,
                kind: cost.kind.clone(),
            })
            .collect();
        let profit = Profit::from_ledger(
            self.trading_state.credits.get(asset).copied().unwrap_or(0),
            self.trading_state.debits.get(asset).copied().unwrap_or(0),
            0,
            &costs,
        )
        .map_err(|unknown| TradingExecError::UnknownCostKind(unknown.0))?;

        let recorded = self.trading_state.net_deltas.get(asset).copied().unwrap_or(0);
        if profit.net != recorded {
            return Err(TradingExecError::ProfitReconciliationMismatch {
                asset: asset.clone(),
                assembled: profit.net,
                recorded,
            });
        }
        Ok(profit)
    }

    /// The realised net profit for an asset, signed: a loss is negative.
    pub fn net_profit(&self, asset: &AssetKey) -> Result<i128, TradingExecError> {
        Ok(self.profit(asset)?.net)
    }
}

/// Helper for constructing a deterministic capability manifest in tests or
/// fixture execution.
pub fn fixture_manifest(state_commitment: [u8; 32]) -> CapabilityManifest {
    CapabilityManifest {
        mode: CapabilityMode::Fixture,
        version: "trading-policy-v1".to_string(),
        chain: "ethereum".to_string(),
        state_commitment,
        private_submission: false,
        providers: BTreeSet::new(),
        venues: BTreeSet::new(),
        bridges: BTreeSet::new(),
    }
}

/// Success or failure classification carried by a trade receipt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TradeOutcome {
    Success,
    Failure { reason: String },
}

/// Typed amount inside a receipt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypedReceiptAmount {
    pub asset: AssetKey,
    pub amount: u128,
}

/// Final per-asset delta committed by the trade.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetDelta {
    pub asset: AssetKey,
    pub delta: i128,
}

/// Debt repayment status recorded in a receipt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DebtReceipt {
    pub debt_id: String,
    pub asset: AssetKey,
    pub principal: u128,
    pub fee: u128,
    pub repaid: bool,
}

/// Deterministic, tamper-evident trading receipt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TradeReceipt {
    pub format_version: u16,
    pub compiler_version: String,
    pub artifact_hash: [u8; 32],
    pub trade_id: String,
    pub policy_id: String,
    pub state_commitment: [u8; 32],
    pub operations: Vec<TradingOperation>,
    pub costs: Vec<CommittedCost>,
    pub debts: Vec<DebtReceipt>,
    pub deltas: Vec<AssetDelta>,
    pub realized_net_profit: Option<TypedReceiptAmount>,
    pub outcome: TradeOutcome,
    pub receipt_hash: [u8; 32],
    pub attestation: Option<ReceiptAttestation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReceiptAttestation {
    pub key_id: String,
    pub public_key: [u8; 32],
    pub signature: Vec<u8>,
}

/// Receipt encoding and validation errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReceiptError {
    Encoding(String),
    HashMismatch {
        expected: [u8; 32],
        actual: [u8; 32],
    },
    OpenDebtInSuccessfulReceipt(String),
    ProfitInFailedReceipt,
    EmptyOperationList,
    EconomicReplayMismatch(String),
    MissingAttestation,
    UntrustedAttestor(String),
    InvalidAttestation,
    /// This exact receipt (by `receipt_hash`) has already been accepted by
    /// this `ReceiptReplayLedger` once before. Distinct from
    /// `EconomicReplayMismatch`, which is about re-deriving a receipt's
    /// reported numbers from its own operations — this is about the same
    /// valid, correctly signed receipt being presented for settlement a
    /// second time.
    ReceiptAlreadySettled([u8; 32]),
}

impl fmt::Display for ReceiptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Encoding(message) => write!(f, "receipt encoding error: {message}"),
            Self::HashMismatch { expected, actual } => {
                write!(f, "receipt hash mismatch: expected {expected:?}, actual {actual:?}")
            }
            Self::OpenDebtInSuccessfulReceipt(debt) => {
                write!(f, "successful receipt contains open debt '{debt}'")
            }
            Self::ProfitInFailedReceipt => write!(f, "failed receipt must not report realized profit"),
            Self::EmptyOperationList => write!(f, "receipt must contain at least one operation"),
            Self::EconomicReplayMismatch(message) => write!(f, "receipt economic replay mismatch: {message}"),
            Self::MissingAttestation => write!(f, "receipt is missing a trusted attestation"),
            Self::UntrustedAttestor(key_id) => write!(f, "receipt attestor '{key_id}' is not trusted"),
            Self::InvalidAttestation => write!(f, "receipt attestation signature is invalid"),
            Self::ReceiptAlreadySettled(hash) => {
                write!(f, "receipt {hash:?} has already been settled once")
            }
        }
    }
}

impl Error for ReceiptError {}

/// Canonical receipt bytes with `receipt_hash` zeroed.
pub fn canonical_receipt_bytes(receipt: &TradeReceipt) -> Result<Vec<u8>, ReceiptError> {
    let mut canonical = receipt.clone();
    canonical.receipt_hash = [0u8; 32];
    canonical.attestation = None;
    serde_json::to_vec(&canonical).map_err(|err| ReceiptError::Encoding(err.to_string()))
}

/// Compute the deterministic receipt hash over canonical bytes.
pub fn compute_receipt_hash(receipt: &TradeReceipt) -> Result<[u8; 32], ReceiptError> {
    let bytes = canonical_receipt_bytes(receipt)?;
    let digest = Sha256::digest(bytes);
    let mut hash = [0u8; 32];
    hash.copy_from_slice(&digest);
    Ok(hash)
}

/// Fill in a receipt's hash from its canonical encoding.
pub fn finalize_receipt(mut receipt: TradeReceipt) -> Result<TradeReceipt, ReceiptError> {
    receipt.receipt_hash = compute_receipt_hash(&receipt)?;
    Ok(receipt)
}

/// Verify a receipt hash and its accounting/tamper invariants.
pub fn verify_receipt(receipt: &TradeReceipt) -> Result<(), ReceiptError> {
    if receipt.operations.is_empty() {
        return Err(ReceiptError::EmptyOperationList);
    }
    let actual = compute_receipt_hash(receipt)?;
    if actual != receipt.receipt_hash {
        return Err(ReceiptError::HashMismatch {
            expected: receipt.receipt_hash,
            actual,
        });
    }
    match &receipt.outcome {
        TradeOutcome::Failure { .. } => {
            if receipt.realized_net_profit.is_some() {
                return Err(ReceiptError::ProfitInFailedReceipt);
            }
        }
        TradeOutcome::Success => {
            if let Some(debt) = receipt.debts.iter().find(|debt| !debt.repaid) {
                return Err(ReceiptError::OpenDebtInSuccessfulReceipt(debt.debt_id.clone()));
            }
        }
    }
    Ok(())
}

pub fn verify_receipt_economics(receipt: &TradeReceipt) -> Result<(), ReceiptError> {
    let first = receipt.operations.first().ok_or(ReceiptError::EmptyOperationList)?;
    let (trade_id, compiled_policy) = match first {
        TradingOperation::BeginAtomicTrade { trade_id, policy } => (trade_id, policy),
        _ => {
            return Err(ReceiptError::EconomicReplayMismatch(
                "operation sequence does not begin with BeginAtomicTrade".to_string(),
            ))
        }
    };
    if trade_id != &receipt.trade_id {
        return Err(ReceiptError::EconomicReplayMismatch(
            "receipt trade_id does not match compiled operation sequence".to_string(),
        ));
    }
    if compiled_policy.policy_id != receipt.policy_id {
        return Err(ReceiptError::EconomicReplayMismatch(
            "receipt policy_id does not match compiled operation sequence".to_string(),
        ));
    }
    if !matches!(receipt.operations.last(), Some(TradingOperation::CommitAtomicTrade)) {
        return Err(ReceiptError::EconomicReplayMismatch(
            "successful receipt operation sequence must terminate in CommitAtomicTrade".to_string(),
        ));
    }

    let mut open_debts: BTreeMap<String, (AssetKey, u128)> = BTreeMap::new();
    let mut closed_debts: BTreeSet<String> = BTreeSet::new();
    let mut saw_profit_guard = false;
    let mut saw_all_debts_guard = false;
    let mut saw_receipt_emit = false;

    for operation in &receipt.operations {
        match operation {
            TradingOperation::BeginAtomicTrade { .. } => {}
            TradingOperation::OpenDebt {
                debt_id,
                asset,
                principal,
                ..
            } => {
                if open_debts
                    .insert(debt_id.clone(), (asset.clone(), *principal))
                    .is_some()
                    || closed_debts.contains(debt_id)
                {
                    return Err(ReceiptError::EconomicReplayMismatch(format!(
                        "debt '{debt_id}' opens more than once"
                    )));
                }
            }
            TradingOperation::CloseDebt { debt_id } => {
                if open_debts.remove(debt_id).is_none() || !closed_debts.insert(debt_id.clone()) {
                    return Err(ReceiptError::EconomicReplayMismatch(format!(
                        "debt '{debt_id}' closes without a matching open debt"
                    )));
                }
            }
            TradingOperation::AssertMinNetProfit { .. } => saw_profit_guard = true,
            TradingOperation::AssertAllDebtsClosed => {
                if !open_debts.is_empty() {
                    return Err(ReceiptError::EconomicReplayMismatch(
                        "all-debts guard appears while debts remain open".to_string(),
                    ));
                }
                saw_all_debts_guard = true;
            }
            TradingOperation::AssertInvariant { .. } => {}
            TradingOperation::EmitTradeReceipt => saw_receipt_emit = true,
            TradingOperation::CommitAtomicTrade => {}
            TradingOperation::AbortAtomicTrade => {
                return Err(ReceiptError::EconomicReplayMismatch(
                    "successful receipt cannot contain AbortAtomicTrade".to_string(),
                ))
            }
            TradingOperation::ExecuteSwap { .. } => {}
            TradingOperation::Bridge { .. } => {}
        }
    }

    if !open_debts.is_empty() {
        return Err(ReceiptError::EconomicReplayMismatch(
            "receipt operation sequence leaves debt open".to_string(),
        ));
    }
    if !saw_profit_guard || !saw_all_debts_guard || !saw_receipt_emit {
        return Err(ReceiptError::EconomicReplayMismatch(
            "receipt operation sequence is missing required final guards/receipt emission".to_string(),
        ));
    }

    let mut expected_debts: BTreeMap<String, (&AssetKey, u128)> = BTreeMap::new();
    for operation in &receipt.operations {
        if let TradingOperation::OpenDebt {
            debt_id,
            asset,
            principal,
            ..
        } = operation
        {
            expected_debts.insert(debt_id.clone(), (asset, *principal));
        }
    }
    for debt in &receipt.debts {
        let Some((asset, principal)) = expected_debts.get(&debt.debt_id) else {
            return Err(ReceiptError::EconomicReplayMismatch(format!(
                "receipt reports unknown debt '{}'",
                debt.debt_id
            )));
        };
        if *asset != &debt.asset || *principal != debt.principal {
            return Err(ReceiptError::EconomicReplayMismatch(format!(
                "receipt debt '{}' does not match compiled operation",
                debt.debt_id
            )));
        }
        if debt.repaid && debt.fee > debt.principal {
            return Err(ReceiptError::EconomicReplayMismatch(format!(
                "receipt debt '{}' reports an implausible fee above principal",
                debt.debt_id
            )));
        }
    }
    if expected_debts.len() != receipt.debts.len() {
        return Err(ReceiptError::EconomicReplayMismatch(
            "receipt debt set does not match compiled operation sequence".to_string(),
        ));
    }

    let mut deltas: BTreeMap<AssetKey, i128> = BTreeMap::new();
    for delta in &receipt.deltas {
        let entry = deltas.entry(delta.asset.clone()).or_insert(0);
        *entry = entry
            .checked_add(delta.delta)
            .ok_or_else(|| ReceiptError::EconomicReplayMismatch("delta overflow".to_string()))?;
    }

    for cost in &receipt.costs {
        // A cost the policy cannot classify is a cost the policy cannot
        // bound, so an unknown category is a replay failure rather than a
        // value to be carried through the totals.
        let kind = CostKind::from_str(&cost.kind).ok_or_else(|| {
            ReceiptError::EconomicReplayMismatch(format!("receipt reports unknown cost kind '{}'", cost.kind))
        })?;
        if !compiled_policy.allowed_cost_kinds.contains(&kind) {
            return Err(ReceiptError::EconomicReplayMismatch(format!(
                "receipt reports cost kind '{}', which the compiled policy's allowed_cost_kinds allowlist does not permit",
                kind.as_str()
            )));
        }
        let entry = deltas.entry(cost.asset.clone()).or_insert(0);
        *entry = entry
            .checked_add(
                i128::try_from(cost.amount)
                    .map_err(|_| ReceiptError::EconomicReplayMismatch("cost conversion overflow".to_string()))?,
            )
            .ok_or_else(|| ReceiptError::EconomicReplayMismatch("cost replay overflow".to_string()))?;
    }

    if let Some(profit) = &receipt.realized_net_profit {
        let replayed = receipt
            .deltas
            .iter()
            .find(|delta| delta.asset == profit.asset)
            .map(|delta| u128::try_from(delta.delta.max(0)).unwrap_or(0))
            .unwrap_or(0);
        if replayed != profit.amount {
            return Err(ReceiptError::EconomicReplayMismatch(format!(
                "reported profit {} does not equal replayed delta {} for {}",
                profit.amount, replayed, profit.asset.symbol
            )));
        }
    }

    for debt in &receipt.debts {
        if matches!(receipt.outcome, TradeOutcome::Success) && !debt.repaid {
            return Err(ReceiptError::OpenDebtInSuccessfulReceipt(debt.debt_id.clone()));
        }
    }

    Ok(())
}

pub fn sign_receipt(
    mut receipt: TradeReceipt,
    key_id: &str,
    signing_key: &SigningKey,
) -> Result<TradeReceipt, ReceiptError> {
    receipt.attestation = None;
    receipt.receipt_hash = compute_receipt_hash(&receipt)?;
    let signature = signing_key.sign(&receipt.receipt_hash);
    receipt.attestation = Some(ReceiptAttestation {
        key_id: key_id.to_string(),
        public_key: signing_key.verifying_key().to_bytes(),
        signature: signature.to_bytes().to_vec(),
    });
    Ok(receipt)
}

pub fn verify_receipt_attestation(
    receipt: &TradeReceipt,
    trusted_keys: &BTreeMap<String, [u8; 32]>,
) -> Result<(), ReceiptError> {
    let attestation = receipt.attestation.as_ref().ok_or(ReceiptError::MissingAttestation)?;
    let trusted = trusted_keys
        .get(&attestation.key_id)
        .ok_or_else(|| ReceiptError::UntrustedAttestor(attestation.key_id.clone()))?;
    if trusted != &attestation.public_key {
        return Err(ReceiptError::UntrustedAttestor(attestation.key_id.clone()));
    }
    let verifying_key = VerifyingKey::from_bytes(trusted).map_err(|_| ReceiptError::InvalidAttestation)?;
    let signature_bytes: [u8; 64] = attestation
        .signature
        .as_slice()
        .try_into()
        .map_err(|_| ReceiptError::InvalidAttestation)?;
    let signature = Signature::from_bytes(&signature_bytes);
    verifying_key
        .verify(&receipt.receipt_hash, &signature)
        .map_err(|_| ReceiptError::InvalidAttestation)
}

pub fn verify_receipt_trusted(
    receipt: &TradeReceipt,
    trusted_keys: &BTreeMap<String, [u8; 32]>,
) -> Result<(), ReceiptError> {
    verify_receipt(receipt)?;
    verify_receipt_economics(receipt)?;
    verify_receipt_attestation(receipt, trusted_keys)
}

/// Replay protection for receipt settlement.
///
/// `verify_receipt_trusted` alone checks that a receipt is well-formed,
/// economically consistent, and signed by a trusted key — but it is a pure
/// function with no memory: the identical valid, correctly signed receipt
/// verifies successfully every single time it's presented. A settlement
/// service that used `verify_receipt_trusted` as its sole admission check
/// would accept (and presumably act on) the same trade's receipt twice.
///
/// This ledger closes that gap: it wraps `verify_receipt_trusted` with a
/// record of every `receipt_hash` already accepted, so the second
/// presentation of an identical receipt is rejected even though every
/// other check about it still passes. It is deliberately not persisted or
/// distributed by this crate — a real settlement service is expected to
/// back this (or an equivalent check) with whatever durable, possibly
/// shared storage its deployment actually needs; this type documents and
/// enforces the invariant in-process.
#[derive(Debug, Clone, Default)]
pub struct ReceiptReplayLedger {
    seen: BTreeSet<[u8; 32]>,
}

impl ReceiptReplayLedger {
    pub fn new() -> Self {
        Self::default()
    }

    /// Verify `receipt` exactly as `verify_receipt_trusted` does, and
    /// additionally reject it if this ledger has already accepted the same
    /// `receipt_hash`. On success, the hash is recorded, so a later replay
    /// of the identical receipt is rejected even though it would otherwise
    /// still pass every other check.
    pub fn verify_and_record(
        &mut self,
        receipt: &TradeReceipt,
        trusted_keys: &BTreeMap<String, [u8; 32]>,
    ) -> Result<(), ReceiptError> {
        verify_receipt_trusted(receipt, trusted_keys)?;
        if !self.seen.insert(receipt.receipt_hash) {
            return Err(ReceiptError::ReceiptAlreadySettled(receipt.receipt_hash));
        }
        Ok(())
    }

    /// Whether `receipt_hash` has already been accepted by this ledger.
    pub fn has_settled(&self, receipt_hash: &[u8; 32]) -> bool {
        self.seen.contains(receipt_hash)
    }
}

/// Build and finalize a canonical receipt from an execution result.
#[allow(clippy::too_many_arguments)]
pub fn build_receipt(
    compiler_version: &str,
    artifact_hash: [u8; 32],
    trade_id: &str,
    policy_id: &str,
    state_commitment: [u8; 32],
    operations: &[TradingOperation],
    state: &TradingState,
    settlement_asset: Option<&AssetKey>,
    outcome: TradeOutcome,
) -> Result<TradeReceipt, ReceiptError> {
    // The ledger, not the per-asset rollup: the rollup cannot say which
    // category each cost belonged to, and an independent verifier needs that
    // category to check the cost against the policy's
    // `allowed_cost_kinds` allowlist.
    let costs = state.cost_ledger.clone();
    let mut debts: Vec<DebtReceipt> = state
        .closed_debt_records
        .iter()
        .map(|(debt_id, record)| DebtReceipt {
            debt_id: debt_id.clone(),
            asset: record.asset.clone(),
            principal: record.principal,
            fee: record.fee,
            repaid: true,
        })
        .collect();
    for (debt_id, record) in &state.open_debts {
        debts.push(DebtReceipt {
            debt_id: debt_id.clone(),
            asset: record.asset.clone(),
            principal: record.principal,
            fee: record.fee,
            repaid: false,
        });
    }
    let deltas = state
        .net_deltas
        .iter()
        .map(|(asset, delta)| AssetDelta {
            asset: asset.clone(),
            delta: *delta,
        })
        .collect();
    let realized_net_profit = match outcome {
        TradeOutcome::Success => match settlement_asset {
            Some(asset) => {
                let raw = state.net_deltas.get(asset).copied().unwrap_or(0).max(0);
                Some(TypedReceiptAmount {
                    asset: asset.clone(),
                    amount: u128::try_from(raw)
                        .map_err(|_| ReceiptError::EconomicReplayMismatch("profit conversion overflow".to_string()))?,
                })
            }
            None => None,
        },
        TradeOutcome::Failure { .. } => None,
    };

    finalize_receipt(TradeReceipt {
        format_version: 1,
        compiler_version: compiler_version.to_string(),
        artifact_hash,
        trade_id: trade_id.to_string(),
        policy_id: policy_id.to_string(),
        state_commitment,
        operations: operations.to_vec(),
        costs,
        debts,
        deltas,
        realized_net_profit,
        outcome,
        receipt_hash: [0u8; 32],
        attestation: None,
    })
}
