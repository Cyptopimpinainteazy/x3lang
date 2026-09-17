//! Atomic trading execution and the capability-controlled host boundary.
//!
//! The VM owns accounting, guard evaluation, atomic rollback, and receipt
//! readiness. Venue and provider behavior is supplied through an explicit
//! [`TradingHost`] capability implementation.

use std::collections::{BTreeMap, BTreeSet};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use std::error::Error;
use std::fmt;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use x3_lang_compiler::ir::{AssetKey, CompiledTradingPolicy, TradingOperation, ValueRef};

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
    fn swap(&mut self, request: SwapRequest) -> Result<SwapResult, HostError>;
    fn close_debt(&mut self, request: RepayRequest) -> Result<RepayResult, HostError>;
    fn execution_costs(&self) -> Result<Vec<CommittedCost>, HostError>;
}

/// Explicit VM-side trading execution errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TradingExecError {
    NonProductionCapability,
    CapabilityChainMismatch { expected: String, actual: String },
    CapabilityVersionMismatch { expected: u16, actual: String },
    PrivateSubmissionRequired,
    InconsistentSubmissionProfile,
    UnknownCapability(String),
    UnsupportedOperation(String),
    InvalidSequence(String),
    HostRejected(HostError),
    AssetMismatch(String),
    OutputBelowMinOut { minimum: u128, actual: u128 },
    StateCommitmentMismatch,
    DeadlineExpired { current: u64, deadline: u64 },
    FeeCeilingExceeded { ceiling_bps: u16, actual_bps: u128 },
    OpenDebtAtCommit(String),
    NetProfitBelowFloor { minimum: u128, actual: u128 },
    MissingReceipt,
    AccountingOverflow,
}

impl fmt::Display for TradingExecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonProductionCapability => write!(f, "production execution rejected fixture capabilities"),
            Self::CapabilityChainMismatch { expected, actual } => {
                write!(f, "compiled policy chain '{expected}' does not match host chain '{actual}'")
            }
            Self::CapabilityVersionMismatch { expected, actual } => {
                write!(f, "compiled policy version {expected} is not supported by host version '{actual}'")
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
            Self::OpenDebtAtCommit(debt) => write!(f, "debt '{debt}' is still open at commit"),
            Self::NetProfitBelowFloor { minimum, actual } => {
                write!(f, "realized net profit {actual} is below floor {minimum}")
            }
            Self::MissingReceipt => write!(f, "borrowed-capital trade did not emit a receipt"),
            Self::AccountingOverflow => write!(f, "checked trading accounting overflowed"),
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
    pub costs: BTreeMap<AssetKey, u128>,
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

    /// Execute a lowered trading operation sequence atomically.
    pub fn execute_atomic(
        &mut self,
        operations: &[TradingOperation],
        host: &mut dyn TradingHost,
        context: TradeExecutionContext,
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

        match self.execute_inner(operations, host, context) {
            Ok(execution) => {
                if let Err(error) = host.commit_transaction() {
                    let _ = host.rollback_transaction();
                    self.trading_state = snapshot;
                    return Err(TradingExecError::HostRejected(error));
                }
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
                    self.accrue_cost(asset, result.fee)?;
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
                    self.debit(from, result.input)?;
                    self.credit(to, result.output)?;
                    self.accrue_cost(&result.fee_asset, result.fee)?;
                    self.trading_state.bindings.insert(binding.clone(), to.clone());
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
                    self.accrue_cost(&record.asset, result.fee)?;
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
                    let actual = self.net_profit(settlement_asset);
                    if actual < *minimum {
                        return Err(TradingExecError::NetProfitBelowFloor {
                            minimum: *minimum,
                            actual,
                        });
                    }
                }
                TradingOperation::AssertAllDebtsClosed => {
                    if let Some((debt, _)) = self.trading_state.open_debts.iter().next() {
                        return Err(TradingExecError::OpenDebtAtCommit(debt.clone()));
                    }
                }
                TradingOperation::EmitTradeReceipt => {
                    self.trading_state.receipt_emitted = true;
                }
                TradingOperation::CommitAtomicTrade => {
                    if let Some((debt, _)) = self.trading_state.open_debts.iter().next() {
                        return Err(TradingExecError::OpenDebtAtCommit(debt.clone()));
                    }
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
            // The settlement asset is checked by AssertMinNetProfit; this is
            // only a defence-in-depth check when a policy carries a floor.
            let best = self
                .trading_state
                .net_deltas
                .values()
                .copied()
                .max()
                .unwrap_or(0)
                .max(0);
            let best = u128::try_from(best).map_err(|_| TradingExecError::AccountingOverflow)?;
            if best < minimum {
                return Err(TradingExecError::NetProfitBelowFloor { minimum, actual: best });
            }
        }
        Ok(TradeExecution {
            committed_state: self.trading_state.clone(),
        })
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
            let already = self.trading_state.costs.get(&cost.asset).copied().unwrap_or(0);
            if cost.amount > already {
                self.accrue_cost(&cost.asset, cost.amount - already)?;
            }
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

    fn credit(&mut self, asset: &AssetKey, amount: u128) -> Result<(), TradingExecError> {
        let entry = self.trading_state.balances.entry(asset.clone()).or_insert(0);
        *entry = entry.checked_add(amount).ok_or(TradingExecError::AccountingOverflow)?;
        let delta = self.trading_state.net_deltas.entry(asset.clone()).or_insert(0);
        *delta = delta
            .checked_add(i128::try_from(amount).map_err(|_| TradingExecError::AccountingOverflow)?)
            .ok_or(TradingExecError::AccountingOverflow)?;
        Ok(())
    }

    fn debit(&mut self, asset: &AssetKey, amount: u128) -> Result<(), TradingExecError> {
        let entry = self.trading_state.balances.entry(asset.clone()).or_insert(0);
        *entry = entry.checked_sub(amount).ok_or(TradingExecError::AccountingOverflow)?;
        let delta = self.trading_state.net_deltas.entry(asset.clone()).or_insert(0);
        *delta = delta
            .checked_sub(i128::try_from(amount).map_err(|_| TradingExecError::AccountingOverflow)?)
            .ok_or(TradingExecError::AccountingOverflow)?;
        Ok(())
    }

    fn accrue_cost(&mut self, asset: &AssetKey, amount: u128) -> Result<(), TradingExecError> {
        let entry = self.trading_state.costs.entry(asset.clone()).or_insert(0);
        *entry = entry.checked_add(amount).ok_or(TradingExecError::AccountingOverflow)?;
        let delta = self.trading_state.net_deltas.entry(asset.clone()).or_insert(0);
        *delta = delta
            .checked_sub(i128::try_from(amount).map_err(|_| TradingExecError::AccountingOverflow)?)
            .ok_or(TradingExecError::AccountingOverflow)?;
        Ok(())
    }

    pub fn net_profit(&self, asset: &AssetKey) -> u128 {
        self.trading_state.net_deltas.get(asset).copied().unwrap_or(0).max(0) as u128
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
    HashMismatch { expected: [u8; 32], actual: [u8; 32] },
    OpenDebtInSuccessfulReceipt(String),
    ProfitInFailedReceipt,
    EmptyOperationList,
    EconomicReplayMismatch(String),
    MissingAttestation,
    UntrustedAttestor(String),
    InvalidAttestation,
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
    let first = receipt
        .operations
        .first()
        .ok_or(ReceiptError::EmptyOperationList)?;
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
            TradingOperation::EmitTradeReceipt => saw_receipt_emit = true,
            TradingOperation::CommitAtomicTrade => {}
            TradingOperation::AbortAtomicTrade => {
                return Err(ReceiptError::EconomicReplayMismatch(
                    "successful receipt cannot contain AbortAtomicTrade".to_string(),
                ))
            }
            TradingOperation::ExecuteSwap { .. } => {}
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
        *entry = entry.checked_add(delta.delta).ok_or_else(|| {
            ReceiptError::EconomicReplayMismatch("delta overflow".to_string())
        })?;
    }

    for cost in &receipt.costs {
        let entry = deltas.entry(cost.asset.clone()).or_insert(0);
        *entry = entry.checked_add(cost.i128::try_from(amount).map_err(|_| TradingExecError::AccountingOverflow)?).ok_or_else(|| {
            ReceiptError::EconomicReplayMismatch("cost replay overflow".to_string())
        })?;
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
    let costs = state
        .costs
        .iter()
        .map(|(asset, amount)| CommittedCost {
            asset: asset.clone(),
            amount: *amount,
            kind: "committed".to_string(),
        })
        .collect();
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
        TradeOutcome::Success => settlement_asset.map(|asset| TypedReceiptAmount {
            asset: asset.clone(),
            amount: u128::try_from(state.net_deltas.get(asset).copied().unwrap_or(0).max(0))
                .map_err(|_| ReceiptError::EconomicReplayMismatch("profit conversion overflow".to_string()))?,
        }),
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
