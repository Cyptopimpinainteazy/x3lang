//! X3 Intermediate Representation (X3IR)
//!
//! Semantic IR for cross-chain atomic operations. Each operation represents
//! a concrete runtime action that can be executed, tracked, and verified.
//!
//! X3IR is generated from AST lowering and consumed by the emitter.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// How an `atomic_choice` ranked its paths.
///
/// Re-exported from the AST so the compiler has one definition of the closed
/// criterion set: an IR and an AST that could disagree about which criteria
/// exist would be a way for an unverified criterion to reach the artifact.
pub use x3_lang_ast::ast::ChoiceCriterion;
/// Re-exported for the same reason: one definition of what a guard's operator
/// is, so the AST and the IR cannot disagree about `<=`.
pub use x3_lang_ast::ast::ComparisonOp;
/// Re-exported for the same reason again: PHASE 39's settlement shapes are a closed
/// set, and an IR that could name a shape the AST does not have would be a way for an
/// unverified claim to reach the artifact.
pub use x3_lang_ast::ast::SettlementGuarantee;

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
        /// Chain the output asset lives on. Carried explicitly because
        /// `ethereum.DAI -> solana.SOL` is a swap that moves value between
        /// chains, and a reader that only knew `from_chain` could not tell.
        to_chain: String,
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

    /// A target portfolio and the criterion a plan for it would be ranked by
    /// (spec PHASE 11).
    ///
    /// The decided weights travel so `x3c lower` shows the portfolio the verifier
    /// checked. No artifact is emitted with this operation today: the phase's own
    /// last sentence — the compiler generating the transaction graph — is not
    /// implemented, so a record without legs would be a plan that does nothing
    /// (TICKET-070).
    Rebalance {
        name: String,
        /// `chain.ASSET` and what the account holds of it, in the asset's own units, in the
        /// order written. Empty means the program stated no holdings — a different fact from
        /// holding nothing, and the one the target alone leaves a host with (TICKET-070).
        holdings: Vec<(String, u128)>,
        /// `chain.ASSET` and its weight in percent, in the order written.
        weights: Vec<(String, u32)>,
        /// The first `minimize` target's name: the metric an optimizer would rank by.
        criterion: String,
    },

    /// A book of obligations reduced to the transfers that remain (spec PHASE 22).
    ///
    /// The decided residual travels so `x3c lower` shows what netting left standing,
    /// because nothing executes it: the parties in a book are symbols rather than
    /// accounts, so there is no balance for the VM to debit. The IR verifier and the
    /// emitter both refuse it for that reason (TICKET-071).

    /// A hedge: two directional legs on one asset, and the net they leave open
    /// (spec PHASE 9).
    ///
    /// The *checked* net travels with the operation, so the record says what the guard
    /// was checked against rather than only that something was checked — the rule the
    /// finality depth follows (TICKET-059). No artifact is emitted with this
    /// operation today: a perp leg needs a venue adapter this VM does not have, so the
    /// IR verifier refuses it (see `verify.rs`), and `x3c lower` is where the decided
    /// net is visible.
    /// An order to a venue (spec PHASE 9's hedge legs, and PHASE 10's liquidation
    /// calls).
    ///
    /// The instruction a hedge and a liquidation need in order to execute at all. A
    /// hedge's legs name a *kind* — spot or perp — rather than a venue, and a
    /// liquidation's calls name a position, so neither is expressible as a swap: what
    /// they need is a host that can act on a venue, asked in a form the artifact can
    /// check. `Operation::Call` would have carried it as an untyped host call, and the
    /// VM routes `CALL_HOST` to `BridgeAdapter::svm_call`, so a perp short on Ethereum
    /// would arrive at a host as an SVM call.
    VenueOrder {
        /// What the venue is asked to do, from the closed vocabulary the compiler owns:
        /// `spot_buy`, `spot_sell`, `perp_long`, `perp_short`, `liquidate`,
        /// `receive_collateral`.
        action: String,
        /// What the action is *about*: a position reference for a liquidation, and empty
        /// when the action names an asset rather than a position.
        subject: String,
        /// `chain.ASSET` the quantity is denominated in.
        asset: String,
        quantity: u128,
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
    /// A bounded branch set: every path was parsed, lowered and verified, and
    /// exactly one was chosen by a criterion the compiler evaluates.
    ///
    /// The record carries the *whole* branch set, not just the winner, because
    /// the artifact's claim is "these were the permitted branches and this is
    /// the one that was taken". The operations that follow are the selected
    /// path's body; nothing at run time can select a different one, which is
    /// what makes the choice bounded rather than dynamic.
    AtomicChoice {
        /// Number of paths the compiler considered.
        paths: u32,
        /// The criterion the choice was made by.
        criterion: ChoiceCriterion,
        /// Index of the path whose body follows, into the declared path list.
        selected: u32,
    },
    /// The venues a route's failing legs may be re-routed through.
    ///
    /// The artifact carries the approved list itself, not a count of it: a
    /// runtime can only restrict itself to the compiler's approvals if the
    /// approvals are in the artifact. Every venue here was verified as a route
    /// in its own right before it was admitted, which is what "no arbitrary
    /// dynamic contract substitution" means in practice.
    RouteFallback {
        /// Venues approved as substitutes, in declaration order.
        approved: Vec<String>,
    },
    /// How a declared venue's leg actually settles — spec PHASE 39.
    ///
    /// Carried in the artifact because the guarantee is the *assumption the trade
    /// rests on*, and an assumption only the source states is one a counterparty, an
    /// auditor or a replayer cannot see. The phase's sentence is "do not claim atomic
    /// CEX execution unless the external venue exposes enforceable settlement
    /// semantics", and a claim that does not reach the artifact is exactly the claim
    /// nobody can check. This is the same rule the finality depth follows
    /// (TICKET-059) and the same reason the rebalance portfolio travels in
    /// [`Operation::Rebalance`].
    ///
    /// `guarantee` is `None` for a venue that states none, which is a different fact
    /// from six shapes and is why it is an `Option` rather than a defaulted
    /// [`SettlementGuarantee::Atomic`]: the compiler *requires* a guarantee of an
    /// `orderbook` venue and leaves an on-chain venue that states none alone, so
    /// defaulting here would invent the one claim PHASE 39 introduces this clause to
    /// stop.
    ///
    /// It records; it does not execute. Nothing in the VM acts on a settlement
    /// guarantee, and pretending otherwise would be the defect this variant exists to
    /// prevent.
    VenueSettlement {
        /// The venue's declared name, as written.
        venue: String,
        /// How its leg settles, when the program says.
        guarantee: Option<SettlementGuarantee>,
    },
    /// The execution plan a `parallel` block produced.
    ///
    /// The artifact carries the plan rather than only the legs, because
    /// "these legs may run concurrently" is a claim about their independence,
    /// and a reader who cannot see the waves cannot check that claim. The legs'
    /// operations follow in wave order.
    /// A strategy module's licence and profit split.
    ///
    /// Carried in the artifact because PHASE 25's distribution is an action
    /// somebody has to take after settlement, and a split that only the source
    /// knows cannot be taken. It records; it does not execute, which is what
    /// keeps PHASE 24's "licensing must never compromise deterministic
    /// execution" true by construction rather than by care.
    StrategyLicense {
        creator: String,
        royalty_bps: u32,
        executions: Option<u128>,
        expires_block: Option<u64>,
        /// `(recipient, basis points)`, summing to 10,000.
        split: Vec<(String, u32)>,
    },
    /// An execution mode the program opted into, as `FEATURE_*`.
    FeatureAllow {
        feature: u8,
        /// The name as written, for diagnostics and for the trace.
        name: String,
    },
    ParallelPlan {
        /// Groups of legs that can run concurrently, in execution order.
        waves: Vec<Vec<String>>,
        /// Data dependencies: `from` produces an asset `to` consumes, so `from`
        /// must complete first. This is the "explicit semantics" that resolves
        /// what would otherwise be a race.
        edges: Vec<(String, String)>,
        /// The execution domain of each leg: the VM family its chains run on.
        ///
        /// A chain nothing declares is its own domain — the compiler can only
        /// honestly say "this is chain X" when the program never said two chains
        /// share a VM. The set of these values answers "is this plan multi-VM".
        domains: std::collections::BTreeMap<String, std::collections::BTreeSet<String>>,
        /// What a coordinator owes before each wave may be treated as settled:
        /// the outstanding proof inputs, the domains involved, and whether the
        /// VM alone can undo the wave.
        settlement: Vec<crate::dag::WaveSettlement>,
    },

    // ===== Guard Operations =====
    /// Require a condition to be true
    /// `require nonce unused <id>` — test the nonce and record it.
    ///
    /// Emitted immediately before the guard that reads its result, because the
    /// guard's quantity is `r0` and this is what puts it there.
    NonceUnused {
        nonce: String,
    },
    Require {
        kind: RequireKind,
        /// Optional subject: chain for Finality, invariant name, etc.
        subject: Option<String>,
        condition: Condition,
        error_msg: Option<String>,
        /// The comparison the guard makes, when it makes one. Carried because
        /// `slippage <= 50` and `slippage >= 50` are opposite claims, and a
        /// check that reads one as the other is reading a direction nobody
        /// wrote.
        #[serde(default)]
        comparison: Option<ComparisonOp>,
        /// Whether this guard is judged against a quantity a host **measured** for the
        /// trade, rather than being an assertion about the artifact's configuration.
        ///
        /// The two are different guards and the language distinguishes them here. A
        /// `require slippage <= 50` a program writes is a *constraint*: the compiler
        /// checks it against the venues' and the policy's declared numbers, and the
        /// instruction records it. A guard a **plan generator** emits after the trade it
        /// bounds is a *post-condition*: what that trade actually realised, in basis
        /// points, reported by the host that executed it. The executor refuses a
        /// measured guard that no host reported rather than comparing register residue —
        /// which is what makes "native profit guards: the transaction refuses settlement
        /// below the target" true instead of decorative.
        ///
        /// `false` for every guard a program writes, so no existing program changes
        /// meaning: only the compiler's own plans set it (TICKET-027).
        #[serde(default)]
        measured: bool,
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
        /// The event's payload, keyed by argument name.
        ///
        /// A `BTreeMap` and not a `HashMap`, because this map reaches the artifact's
        /// **bytes**: the emitter renders it with `{:?}`, and a `HashMap`'s `Debug` prints
        /// in iteration order, which for `RandomState` differs between two maps built from
        /// the same entries in the same process. Measured before the fix: twelve
        /// identical compiles of one source produced **six** distinct artifacts, and the
        /// only difference was this field's order
        /// (`{"arg2": …, "arg1": …, "arg0": …}` against `{"arg1": …, "arg2": …, "arg0": …}`).
        /// A `BTreeMap` gives the bytes an order that comes from the program rather than
        /// from a per-instance seed (PHASE 42).
        data: BTreeMap<String, String>,
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
        /// The weights, keyed by the property being scored.
        ///
        /// A `BTreeMap` for the reason `Emit::data` gives: the emitter collects this map
        /// into the payload it writes, so its iteration order is the artifact's byte
        /// order. `verify_route_score` also iterates it to name every overweight key, so
        /// with a `HashMap` the same source could produce a different *diagnostic* as well
        /// (PHASE 42).
        weights: BTreeMap<String, u32>,
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
    /// Asset `minimum_net_profit` is denominated in.
    ///
    /// The same reason `max_gas_asset` exists: without it the floor is an
    /// unenforceable bare number, and the check that consumed it took the
    /// *maximum* delta across every asset — passing a trade that lost on the
    /// settlement asset whenever a side asset gained.
    #[serde(default)]
    pub minimum_net_profit_asset: Option<AssetKey>,
    /// Maximum age, in blocks, of the venue quote a swap is allowed to have
    /// been taken at. `None` means no freshness requirement — the same opt-in
    /// shape as `max_oracle_deviation_bps`.
    ///
    /// Three sibling ceilings were removed here rather than left unenforced:
    /// `max_total_cost` (a bare amount with no denomination asset, hardcoded to
    /// `max_gas`, so it duplicated the gas ceiling and could mean nothing
    /// else), and `max_price_impact_bps` / `max_mev_leakage_bps` (both
    /// hardcoded to `max_slippage_bps`; the host boundary carries no
    /// price-impact or MEV-leakage evidence, so enforcing them would have
    /// required inventing host fields and comparing fabricated numbers). No
    /// source program could set any of the three, and nothing enforced them —
    /// they were read only by `EconomicPolicy::validate_not_weaker_than`,
    /// which compared each one against a copy of itself.
    pub quote_freshness_blocks: Option<u64>,
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
    /// A `finality_policy` declaration: the word a chain must reach, and the depth
    /// the program requires of it.
    ///
    /// Carried as a typed value rather than as `Expression { expr: "strict
    /// finalized" }` so the depth can reach the artifact: the emitter writes it
    /// into the `REQUIRE` operand, which is what lets a replayer re-check the
    /// guard-versus-declaration relationship the compiler decided (TICKET-059). A
    /// string would have to be re-parsed by whoever wants the number, and a value
    /// nothing parses is a claim nothing can check (the shape of TICKET-044).
    FinalityPolicy {
        /// The policy's own name (`strict`, `relaxed`, …), as written.
        name: String,
        /// The word the chain must reach (`finalized`, `safe`, …).
        requirement: String,
        /// The depth the program requires, in blocks, when it states one.
        blocks: Option<u32>,
    },
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
    /// Check the program's computed risk score is within a ceiling.
    ///
    /// Unlike the other kinds this one has no declaration to compare against:
    /// `compute_risk_score` reads the program's own operations, so the guard is
    /// evaluated rather than checked against the source. It is a real variant
    /// rather than a `Custom("risk_score")` string so that the pass which decides
    /// it cannot be reached by a program that happens to write that name as an
    /// unknown guard.
    RiskScore,
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
