//! X3 Intermediate Representation (X3IR)
//!
//! Semantic IR for cross-chain atomic operations. Each operation represents
//! a concrete runtime action that can be executed, tracked, and verified.
//!
//! X3IR is generated from AST lowering and consumed by the emitter.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};

/// Root IR program - list of operations to execute in sequence
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct X3IR {
    pub operations: Vec<Operation>,
    /// Metadata about the program (nonce, chain_id, etc.)
    pub metadata: ProgramMetadata,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProgramMetadata {
    /// Unique identifier for this execution (prevents replay)
    pub nonce: Option<String>,
    /// Chain context where this executes
    pub chain_id: Option<u64>,
    /// Timeout in blocks/seconds before automatic rollback
    pub timeout_blocks: Option<u32>,
}

impl X3IR {
    pub fn new() -> Self {
        X3IR {
            operations: Vec::new(),
            metadata: ProgramMetadata {
                nonce: None,
                chain_id: None,
                timeout_blocks: None,
            },
        }
    }

    pub fn push(&mut self, op: Operation) {
        self.operations.push(op);
    }
}

impl Default for X3IR {
    fn default() -> Self {
        X3IR::new()
    }
}

/// A single IR operation - the atomic unit of execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Operation {
    // ===== Asset Transfer Operations =====
    /// Lock assets on source chain (begin settlement)
    Lock {
        chain: String,
        asset: String,
        amount: u128,
        from: String,
    },
    /// Mint assets on destination chain
    Mint {
        chain: String,
        asset: String,
        amount: u128,
        to: String,
    },
    /// Burn assets (remove from circulation)
    Burn {
        chain: String,
        asset: String,
        amount: u128,
        from: String,
    },
    /// Release locked assets (settlement complete)
    Release {
        chain: String,
        asset: String,
        to: String,
    },

    // ===== Swap Operations =====
    /// Execute a DEX swap
    Swap {
        from_chain: String,
        from_asset: String,
        to_asset: String,
        input_amount: u128,
        min_output: u128,
        dex: Option<String>,
    },
    /// Bridge assets across chains through a configured bridge route.
    Bridge {
        via: String,
        from_chain: String,
        from_asset: String,
        to_chain: String,
        to_asset: String,
        amount: u128,
        receiver: String,
        source_finality_proof: Vec<u8>,
        transfer_proof: Vec<u8>,
    },

    // ===== Control Flow =====
    /// Conditional execution
    If {
        condition: Condition,
        then_ops: Vec<Operation>,
        else_ops: Option<Vec<Operation>>,
    },
    /// Loop (bounded by step count)
    Loop {
        max_iterations: u32,
        body: Vec<Operation>,
    },
    /// Mark beginning of atomic block (all-or-nothing)
    AtomicBegin,
    /// Mark end of atomic block
    AtomicEnd,

    // ===== Guard Operations =====
    /// Require a condition to be true
    Require {
        kind: RequireKind,
        /// Optional subject: chain for Finality, invariant name, etc.
        subject: Option<String>,
        condition: Condition,
        error_msg: Option<String>,
    },
    /// On failure, execute recovery action
    OnFail {
        action: FailureAction,
    },
    /// On timeout, execute recovery action
    OnTimeout {
        duration_blocks: u32,
        action: FailureAction,
    },

    // ===== Utility Operations =====
    /// No operation (filler/padding)
    Nop,
    /// Emit a named event
    Emit {
        name: String,
        data: HashMap<String, String>,
    },
    /// Call external function
    Call {
        function: String,
        args: Vec<String>,
    },

    // ===== X3 capability operations =====
    GpuDispatch {
        kernel: String,
        args: Vec<String>,
        is_simd: bool,
    },
    Simulate {
        body: Vec<Operation>,
        receipt_slot: String,
    },
    ScheduledDispatch {
        period_blocks: u32,
        entry: Vec<Operation>,
    },
    IntentResolve {
        constraints: Vec<String>,
        resolver: String,
    },
    CrdtOp {
        kind: CrdtKind,
        key: String,
        value: Option<String>,
    },
    ProofVerify {
        kind: ProofKind,
        proof: String,
        input: String,
        key_or_threshold: String,
    },
    StorageOp {
        kind: StorageKind,
        data: String,
    },
    Pathfind {
        from: String,
        to: String,
        max_depth: u32,
    },
    MempoolScan {
        max_results: u32,
    },
    OracleRequest {
        token: String,
        reward: u128,
    },
    EmergencyControl {
        kind: EmergencyKind,
    },
    Lifecycle {
        kind: LifecycleKind,
        target: Option<String>,
    },
    Serialize {
        format: SerialFormat,
        data: String,
    },
    Deserialize {
        format: SerialFormat,
        data: String,
    },
    GasEstimate {
        chain: String,
        route: String,
    },
    ChainMetric {
        metric: ChainMetricKind,
    },
    EventProvenance {
        event_type: String,
        data: String,
    },
    MultiHopSwap {
        path: Vec<String>,
        amount: u128,
    },
    VectorMath {
        op: VectorOp,
        a: String,
        b: String,
        size: u32,
    },
    RoleCheck {
        role: String,
    },
    MultisigCheck {
        required: u32,
        total: u32,
    },
    VersionMeta {
        version: String,
        upgrade_from: Option<String>,
    },
    StorageNamespace {
        package: String,
        key: String,
    },
    AbiExport {
        function: String,
        params: Vec<String>,
        ret: String,
    },
    DocEmbed {
        content: String,
    },
    GasAdaptive {
        high_gas_ops: Vec<Operation>,
        low_gas_ops: Vec<Operation>,
    },
    Bounty {
        amount: u128,
        condition: String,
    },

    // ===== B-52 Feature Lock Operations =====
    /// Route scoring with strategy name and weight map
    RouteScore {
        strategy: String,
        weights: HashMap<String, u32>,
    },
    /// Solver bid with fee, bond, and asset pair
    SolverBid {
        solver: String,
        receive_asset: String,
        deliver_asset: String,
        fee: String,
        bond: u128,
    },
    /// Relayer attestation with quorum and signatures
    RelayerAttest {
        relayers: Vec<String>,
        quorum: (u32, u32),
        signatures: Vec<String>,
    },
    /// RPC consensus requirement
    RpcConsensus {
        chain: String,
        require: (u32, u32),
        reject_on: Vec<String>,
    },
    /// Risk score evaluation
    RiskScore {
        score: u32,
        category: String,
    },
    /// Named invariant check
    InvariantCheck {
        name: String,
        assert_expr: String,
    },
    /// Privacy commitment configuration
    PrivacyCommit {
        reveal_on: String,
        encrypted: bool,
    },
    /// Required proof declaration
    ProofRequired {
        proof_type: String,
        source: String,
    },
    /// VM adapter call
    VmAdapterCall {
        vm: String,
        adapter: String,
        calldata: String,
    },
    /// Mode check restriction
    ModeCheck {
        mode: String,
        restriction: String,
    },
    /// Package import with optional alias
    PackageImport {
        path: Vec<String>,
        alias: Option<String>,
    },
    /// Refund policy configuration
    RefundPolicy {
        action: String,
        target: String,
        after_blocks: u32,
    },

    // ===== Trading Core v1 Operations =====
    /// A deterministic accounting-oriented trading operation.
    Trading(TradingOperation),
}

/// Stable typed asset identity carried by trading IR.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct AssetKey {
    pub vm_family: String,
    pub chain: String,
    pub canonical_id: String,
    pub symbol: String,
    pub decimals: u8,
}

/// Stable submission/privacy requirement for economic execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum SubmissionProfile {
    Public,
    Protected,
    Private,
}

/// Strength of the state commitment bound into an economic proof.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum StateBindingMode {
    None,
    HostCommitment,
    Exact,
}

/// Stable cost categories understood by economic policy version 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum CostKind {
    Gas,
    LiquidityFee,
    FlashLiquidityFee,
    SolverInfrastructureFee,
    ProofFee,
    CrossDomainFee,
    Slippage,
    PriceImpact,
    MevLeakage,
}

impl CostKind {
    /// Every cost kind understood by economic policy version 1, in a fixed
    /// order so callers can iterate deterministically.
    pub const ALL: [CostKind; 9] = [
        CostKind::Gas,
        CostKind::LiquidityFee,
        CostKind::FlashLiquidityFee,
        CostKind::SolverInfrastructureFee,
        CostKind::ProofFee,
        CostKind::CrossDomainFee,
        CostKind::Slippage,
        CostKind::PriceImpact,
        CostKind::MevLeakage,
    ];

    /// Canonical, stable name for this cost kind.
    ///
    /// Hosts report committed costs as `CommittedCost { kind: String }`, and
    /// receipts persist those reports. Before this mapping existed the wire
    /// string was never compared against anything, so a policy's
    /// `allowed_cost_kinds` allowlist could not be enforced: any string a
    /// host sent was accepted, and receipts stored the placeholder
    /// `"committed"` instead of the real category. This is the single
    /// source of truth for both directions.
    pub fn as_str(self) -> &'static str {
        match self {
            CostKind::Gas => "gas",
            CostKind::LiquidityFee => "liquidity_fee",
            CostKind::FlashLiquidityFee => "flash_liquidity_fee",
            CostKind::SolverInfrastructureFee => "solver_infrastructure_fee",
            CostKind::ProofFee => "proof_fee",
            CostKind::CrossDomainFee => "cross_domain_fee",
            CostKind::Slippage => "slippage",
            CostKind::PriceImpact => "price_impact",
            CostKind::MevLeakage => "mev_leakage",
        }
    }

    /// Parse a host- or receipt-reported cost kind.
    ///
    /// Returns `None` for anything unrecognized. Callers in the execution and
    /// verification paths must treat `None` as a hard failure rather than a
    /// default: an unclassifiable cost cannot be checked against a policy
    /// allowlist, and silently accepting it would let a host bypass the
    /// allowlist by inventing a category.
    pub fn from_str(name: &str) -> Option<CostKind> {
        CostKind::ALL.into_iter().find(|kind| kind.as_str() == name)
    }
}

/// Reference to a literal base-unit amount or a prior trading binding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ValueRef {
    Literal(u128),
    Binding(String),
}

/// Immutable, compile-time snapshot of the risk policy carried by a trade.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompiledTradingPolicy {
    pub policy_id: String,
    pub policy_version: u16,
    pub chain: String,
    pub max_slippage_bps: u16,
    pub max_gas: u128,
    /// Asset `max_gas` is denominated in. Without this, `max_gas` is an
    /// unenforceable bare number: the VM has no way to know which of a
    /// trade's several accrued costs it's supposed to cap.
    pub max_gas_asset: AssetKey,
    pub max_flash_fee_bps: u16,
    pub deadline_blocks: u64,
    /// Legacy migration field. It must agree with `submission_profile`.
    pub require_private_submission: bool,
    pub minimum_net_profit: Option<u128>,
    pub max_total_cost: u128,
    pub max_price_impact_bps: u16,
    pub max_mev_leakage_bps: u16,
    pub quote_freshness_blocks: u64,
    pub submission_profile: SubmissionProfile,
    pub state_binding: StateBindingMode,
    pub allowed_cost_kinds: BTreeSet<CostKind>,
    pub allow_mint: bool,
    pub allow_burn: bool,
    /// Oracle-firewall ceiling: maximum allowed disagreement, in basis
    /// points, between the venue's primary quote and any other independent
    /// price source the host reports. `None` means no cross-source check
    /// is required for this trade.
    pub max_oracle_deviation_bps: Option<u16>,
    /// Cross-trade circuit breaker: maximum realized loss, in
    /// `max_cumulative_loss_asset`, the VM instance executing this trade
    /// may have accumulated across every trade it has already committed
    /// before this one is allowed to commit too. `None` means no
    /// cross-trade ceiling is enforced. Always `Some` exactly when
    /// `max_cumulative_loss_asset` is `Some` — the VM has no way to
    /// enforce an amount with no asset to measure it in.
    pub max_cumulative_loss: Option<u128>,
    /// Asset `max_cumulative_loss` is denominated in.
    pub max_cumulative_loss_asset: Option<AssetKey>,
}

impl CompiledTradingPolicy {
    /// Returns true only when the legacy privacy flag and versioned profile agree.
    pub fn submission_profile_is_consistent(&self) -> bool {
        self.require_private_submission == (self.submission_profile == SubmissionProfile::Private)
    }
}

/// Trading Core v1 IR operations. Field names are stable and explicit for
/// deterministic receipt and artifact hashing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TradingOperation {
    BeginAtomicTrade {
        trade_id: String,
        policy: CompiledTradingPolicy,
    },
    OpenDebt {
        debt_id: String,
        provider: String,
        asset: AssetKey,
        principal: u128,
    },
    ExecuteSwap {
        binding: String,
        venue: String,
        from: AssetKey,
        to: AssetKey,
        input: ValueRef,
        min_output: u128,
    },
    CloseDebt {
        debt_id: String,
    },
    /// Move a settled amount to another chain through a bridge. No
    /// `min_output`: a bridge transfer is proven by a cryptographic
    /// inclusion/finality proof at settlement time (see
    /// `x3_lang_vm::bridge`), not subject to venue-side slippage the way
    /// a DEX swap quote is.
    Bridge {
        via: String,
        from: AssetKey,
        to: AssetKey,
        input: ValueRef,
        receiver: String,
    },
    AssertMinNetProfit {
        settlement_asset: AssetKey,
        minimum: u128,
    },
    AssertAllDebtsClosed,
    AssertInvariant {
        kind: InvariantKind,
    },
    EmitTradeReceipt,
    CommitAtomicTrade,
    AbortAtomicTrade,
}

/// IR-level mirror of `x3_lang_ast::InvariantKind`. Kept as its own type
/// (matching how `AssetKey`/`CompiledTradingPolicy` mirror their AST
/// counterparts) so bytecode encoding stays stable independent of AST shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum InvariantKind {
    /// Every asset touched by the trade nets to a non-negative delta at
    /// commit time.
    Solvent,
}

impl InvariantKind {
    pub fn as_str(self) -> &'static str {
        match self {
            InvariantKind::Solvent => "solvent",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CrdtKind {
    Get,
    Set,
    Append,
    Merge,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ProofKind {
    Zk,
    Mpc,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StorageKind {
    Store,
    Load,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EmergencyKind {
    Pause,
    Resume,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LifecycleKind {
    Destroy,
    Migrate,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SerialFormat {
    Rlp,
    Cbor,
    Json,
    Ssz,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ChainMetricKind {
    Snapshot,
    Congestion,
    BaseFee,
    FinalityLag,
    BlockTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum VectorOp {
    Add,
    DotProduct,
    Mul,
    Sub,
}

/// Conditions used in If and Require operations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Condition {
    /// Balance comparison: balance(chain, asset, account) >= amount
    BalanceGte {
        chain: String,
        asset: String,
        account: String,
        amount: u128,
    },
    /// Nonce comparison: nonce(account) == expected
    NonceEq { account: String, expected: u64 },
    /// Proof verification: verify_proof(proof_data, expected_hash)
    ProofValid { proof: String, expected_hash: String },
    /// Boolean expression evaluation
    Expression { expr: String },
    /// Always true
    True,
    /// Always false
    False,
}

/// Types of require guards (for invariant/correctness checks)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RequireKind {
    /// Check canonical supply is preserved
    CanonicalSupply,
    /// Check nonce has not been used
    NonceUnused,
    /// Check bridge liquidity is sufficient
    BridgeLiquidity,
    /// Check slippage is within tolerance
    SlippageTolerance,
    /// Check profit/gains meet threshold
    ProfitThreshold,
    /// Check finality (confirmations)
    Finality,
    /// Check route score meets threshold
    RouteScore,
    /// Check solver bond is sufficient
    SolverBond,
    /// Check relayer quorum met
    RelayerQuorum,
    /// Check proof was completed
    ProofComplete,
    /// Check refund path exists
    RefundPath,
    /// Explicit finality check
    FinalityExplicit,
    /// Check VM is supported
    VmSupported,
    /// Mainnet safety check
    MainnetSafe,
    /// Custom user-defined check
    Custom(String),
}

/// Recovery actions on failure or timeout
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FailureAction {
    /// Roll back all operations in atomic block
    Rollback,
    /// Refund specific asset to account
    Refund { chain: String, asset: String, to: String },
    /// Halt the bridge (stop processing)
    Halt,
    /// Quarantine for manual review
    Quarantine,
}
