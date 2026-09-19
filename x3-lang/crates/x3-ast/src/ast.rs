use serde::{Deserialize, Serialize};
use x3_lang_common::{BinOp, DurationUnit, FloatSuffix, IntBase, IntSuffix, SizeUnit, Spanned, Symbol, UnOp};

use crate::trading::{AssetDecl, AtomicTradeDecl, TradeRiskPolicy};

/// Node ID - deterministic, 0-based index assigned during parsing/lowering when required.
/// Internally is a simple u32 wrapper for compactness and reproducibility.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct NodeId(pub u32);

/// The root of an X3 program AST.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Program {
    /// Top-level module items (functions, agents, types, etc.).
    pub items: Vec<Spanned<Item>>,
}

impl Program {
    pub fn new(items: Vec<Spanned<Item>>) -> Self {
        Program { items }
    }
}

/// Top-level items (declarations)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Item {
    Function(Function),
    Agent(Agent),
    Struct(StructDecl),
    Enum(EnumDecl),
    Use(UseDecl),
    Mod(ModDecl),
    Import(ImportDecl),
    Const(ConstDecl),
    // Cross-chain top-level declarations
    Bridge(BridgeDecl),
    AtomicSwap(AtomicSwapDecl),
    AtomicChoice(AtomicChoiceDecl),
    Strategy(CrossChainStrategy),
    Proposal(ProposalDecl),
    GpuBlock(GpuBlock),
    SimulateDecl(SimulateDecl),
    ScheduledTask(ScheduledTask),
    IntentDecl(IntentDecl),
    SubscriptionDecl(SubscriptionDecl),
    // B-52 feature lock
    VmDecl(VmDecl),
    SolverMarket(SolverMarket),
    RelayerSwarm(RelayerSwarm),
    RpcQuorum(RpcQuorum),
    RiskPolicy(RiskPolicy),
    PrivacyBlock(PrivacyBlock),
    InvariantDecl(InvariantDecl),
    ErrorDecl(ErrorDecl),
    FinalityPolicy(FinalityPolicy),
    ProofsRequired(ProofsRequired),
    VmTarget(VmTarget),
    // ===== Trading Core v1 top-level declarations =====
    AssetDecl(AssetDecl),
    TradeRiskPolicy(TradeRiskPolicy),
    VenueDecl(VenueDecl),
    /// `atomic_hedge { … }` — spec PHASE 9.
    AtomicHedge(AtomicHedgeDecl),
    /// `atomic_liquidation { … }` — spec PHASE 10.
    AtomicLiquidation(AtomicLiquidationDecl),
    /// `rebalance <name> { … }` — spec PHASE 11.
    Rebalance(RebalanceDecl),
    /// `netting <name> { … }` — spec PHASE 22.
    Netting(NettingDecl),
    /// `arb <name> { discover { … } capital { … } execution { … } risk { … } }` —
    /// spec PHASE 37.
    Arb(ArbDecl),
    ParallelDecl(ParallelDecl),
    ObjectiveDecl(ObjectiveDecl),
    AtomicTrade(AtomicTradeDecl),
}

/// A `use` declaration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UseDecl {
    pub path: Vec<Symbol>,
    pub alias: Option<Symbol>,
}

/// An `import` declaration for FFI or runtime adapters
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportDecl {
    pub module: Vec<Symbol>,
    pub as_alias: Option<Symbol>,
}

/// A `mod` declaration - for modules
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModDecl {
    pub name: Symbol,
    pub items: Vec<Spanned<Item>>,
}

/// Constant declaration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConstDecl {
    pub name: Symbol,
    pub ty: Option<TypeExpr>,
    pub value: Expression,
}

/// Function declaration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Function {
    pub name: Symbol,
    pub id: Option<NodeId>,
    pub params: Vec<Parameter>,
    pub ret: Option<TypeExpr>,
    pub generics: Vec<GenericParam>,
    pub body: Block,
    pub visibility: Visibility,
    pub is_async: bool,
    pub annotations: Vec<Annotation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Parameter {
    pub name: Option<Symbol>,
    pub ty: Option<TypeExpr>,
    pub is_mut: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Visibility {
    Pub,
    Priv,
}

/// Generic parameter declaration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenericParam {
    pub name: Symbol,
    pub bounds: Vec<TypeExpr>,
}

/// Struct declaration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StructDecl {
    pub name: Symbol,
    pub fields: Vec<StructField>,
    pub generics: Vec<GenericParam>,
    pub visibility: Visibility,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StructField {
    pub name: Symbol,
    pub ty: TypeExpr,
    pub visibility: Visibility,
}

/// Enum declaration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnumDecl {
    pub name: Symbol,
    pub variants: Vec<EnumVariant>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnumVariant {
    pub name: Symbol,
    pub payload: Option<TypeExpr>,
}

/// Agent declaration - core X3 construct
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Agent {
    pub name: Symbol,
    pub id: Option<NodeId>,
    /// Optional context block - runtime configuration inherited by strategies
    pub context: Option<ContextBlock>,
    /// State variables for the agent
    pub state: Vec<StructField>,
    /// Methods and strategies (functions)
    pub methods: Vec<Spanned<Function>>,
    /// Strategies (named entry points)
    pub strategies: Vec<Spanned<StrategyDecl>>,
    pub visibility: Visibility,
    pub annotations: Vec<Annotation>,
}

/// Context block defines configuration for an agent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextBlock {
    pub entries: Vec<(Symbol, Expression)>,
}

/// Strategy declaration inside agents
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyDecl {
    pub name: Symbol,
    pub id: Option<NodeId>,
    pub params: Vec<Parameter>,
    pub body: Block,
    pub is_async: bool,
}

/// Block - a sequence of statements
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Block {
    pub stmts: Vec<Statement>,
}

impl Block {
    pub fn new(stmts: Vec<Statement>) -> Self {
        Block { stmts }
    }
}

/// Statements
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Statement {
    Let {
        name: Symbol,
        ty: Option<TypeExpr>,
        expr: Option<Expression>,
        is_mut: bool,
    },
    Expr(Expression),
    Return(Option<Expression>),
    Break,
    Continue,
    If {
        cond: Expression,
        then_block: Block,
        else_block: Option<Block>,
    },
    While {
        cond: Expression,
        body: Block,
    },
    For {
        pattern: Pattern,
        iterable: Expression,
        body: Block,
    },
    Loop(Block),
    Atomic(AtomicBlock),
    Emit(EventEmit),

    // ===== Cross-chain asset operations =====
    /// `lock chain.ASSET amount <expr> from <expr>`
    Lock {
        chain: ChainRef,
        asset: AssetRef,
        amount: Expression,
        from: Expression,
    },
    /// `mint chain.ASSET amount <expr> to <expr>`
    Mint {
        asset: AssetRef,
        amount: Expression,
        to: Expression,
    },
    /// `burn chain.ASSET amount <expr> from <expr>`
    Burn {
        asset: AssetRef,
        amount: Expression,
        from: Expression,
    },
    /// `release chain.ASSET to <expr>`
    Release {
        chain: ChainRef,
        asset: AssetRef,
        to: Expression,
    },
    /// `swap <venue> from.ASSET -> to.ASSET [amount <expr>] [min_output <expr>]`
    Swap {
        from: AssetRef,
        to: AssetRef,
        /// The step's input amount, from `amount <expr>`.
        ///
        /// It used to be called `route`, which is what the parser wrote the amount
        /// *into*: three readers (the lowering, the formatter, the profitability
        /// check) treated it as the amount, and the name said otherwise — a reader
        /// of the field had to check which of the two meanings the producers used.
        /// The old name is still accepted when an AST is deserialized, so a stored
        /// artifact does not stop loading (TICKET-067).
        #[serde(default, alias = "route")]
        amount: Option<Expression>,
        min_output: Option<Expression>,
        dex: Option<Expression>,
    },
    /// `bridge via from.ASSET -> to.ASSET amount <expr> receiver <expr>
    ///   [finality_proof <expr>] [transfer_proof <expr>]`
    Bridge {
        via: Symbol,
        from: AssetRef,
        to: AssetRef,
        amount: Expression,
        receiver: Expression,
        source_finality_proof: Option<Expression>,
        transfer_proof: Option<Expression>,
    },

    // ===== Cross-chain guards =====
    /// `require <kind> [subject] <value_expr>`
    Require(RequireGuard),
    /// `on_fail <action>`
    OnFail(FailureAction),
    /// `on_timeout <duration> <action>`
    OnTimeout {
        duration: Expression,
        action: FailureAction,
    },
    /// `allow <feature>` — opt in to an execution mode the compiler may apply.
    ///
    /// A closed set rather than free text: "allow intent_fusion" is the compiler
    /// agreeing to net this intent against others, which changes who settles
    /// with whom. An unknown feature is refused, so a misspelling cannot read as
    /// consent.
    Allow {
        feature: Symbol,
    },
    /// `fallback { replace with <venue> ... require <bound> ... }` — the
    /// approved substitutions for a route's failing legs.
    ///
    /// The list *is* the approval: the runtime may pick a replacement only from
    /// these venues, and every one of them is verified as a route in its own
    /// right before it is admitted. A substitution the compiler did not check
    /// cannot be expressed here, which is what stops a failing leg from being
    /// replaced by arbitrary code.
    RouteFallback {
        replacements: Vec<FallbackReplacement>,
        /// Bounds a substitution must satisfy, written as ordinary guards.
        requires: Vec<RequireGuard>,
    },

    // ===== Capability statements =====
    Snapshot,
    Diff {
        before: Expression,
        after: Expression,
    },
    CrdtOp(CrdtOp),
    SelfDestruct,
    Migrate {
        new_contract: Expression,
    },
    ZkVerify {
        proof: Expression,
        public_input: Expression,
        key: Expression,
    },
    MpcVerify {
        result: Expression,
        signatures: Expression,
        threshold: Expression,
    },
    StorageRef {
        op: StorageRefOp,
        data: Expression,
    },
    Pathfind {
        from: Expression,
        to: Expression,
        max_depth: Expression,
    },
    MempoolScan {
        max_results: Expression,
    },
    OracleRequest {
        token: Expression,
        reward: Expression,
    },
    Pause,
    Resume,
}

/// Pattern is used in `let`, `for`, `match` (keep simple for now)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Pattern {
    Wildcard,
    Ident(Symbol),
    Tuple(Vec<Pattern>),
    Literal(LiteralExpr),
}

/// Atomic block
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AtomicBlock {
    /// Optional 'prepare' or metadata to enforce prepare_root check
    pub meta: Option<Expression>,
    pub body: Block,
}

/// Emitted event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventEmit {
    pub name: Symbol,
    pub payload: Vec<Expression>,
}

/// Function, agent, and strategy annotations parsed from `@name(...)` attributes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Annotation {
    NoHeap,
    NoRecursion(u32),
    Hot,
    Audit,
    Role(Symbol),
    Multisig(u32, u32),
    Version(Symbol),
    UpgradeFrom(Symbol),
    OnChain,
    OffChain,
    Sandbox,
    Whitelist(Vec<Symbol>),
    Concurrent,
    Scheduled(u64),
    Subscription(u128, u64),
    Extern,
    Payable,
    Simd,
    Subscribe(Symbol),
    Sponsor,
    GasAdaptive,
}

/// `gpu { ... }` or SIMD-capable GPU block.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GpuBlock {
    pub body: Block,
    pub is_simd: bool,
}

/// Off-chain simulation declaration that returns a simulation receipt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimulateDecl {
    pub name: Symbol,
    pub body: Block,
    pub receipt: Option<Symbol>,
}

/// Periodic on-chain task declaration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduledTask {
    pub name: Symbol,
    pub period_blocks: u64,
    pub body: Block,
}

/// Intent declaration with constraints and resolver body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntentDecl {
    pub name: Symbol,
    pub constraints: Vec<Expression>,
    pub body: Block,
}

/// Subscription declaration with amount and charging period.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubscriptionDecl {
    pub name: Symbol,
    pub amount: u128,
    pub period_blocks: u64,
    pub body: Block,
}

/// CRDT operation statements.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrdtOp {
    pub kind: CrdtOpKind,
    pub key: Expression,
    pub value: Option<Expression>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CrdtOpKind {
    Get,
    Set,
    Append,
    Merge,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StorageRefOp {
    Store,
    Load,
}

// ============================================================
// Cross-chain types
// ============================================================

/// Reference to a blockchain network (e.g., "eth", "sol", "btc", "x3").
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ChainRef(pub Symbol);

impl ChainRef {
    pub fn new(name: Symbol) -> Self {
        ChainRef(name)
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

/// Qualified asset reference on a specific chain (e.g., `eth.USDC`, `x3.USDC_e`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AssetRef {
    pub chain: ChainRef,
    pub name: Symbol,
}

impl AssetRef {
    pub fn new(chain: ChainRef, name: Symbol) -> Self {
        AssetRef { chain, name }
    }
}

/// Kind of runtime requirement guard.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RequireKind {
    /// `require finality <chain> >= <confirmations>`
    Finality,
    /// `require slippage <= <pct>`
    Slippage,
    /// `require profit > <amount>`
    Profit,
    /// `require invariant <name> == <expected>`
    InvariantCheck,
    /// `require risk < <score>`
    RiskScore,
    /// `require nonce unused`
    Nonce,
    /// `require audit_gate <name>`
    AuditGate,
    /// `require bridge_liquidity >= <amount>`
    BridgeLiquidity,
    /// `require canonical_supply <asset>`
    CanonicalSupply,
    /// `require relayer_quorum >= <count>` — minimum number of relayers
    /// that must attest to the cross-chain operation before settlement.
    RelayerQuorum,
    /// `require route_score >= <score>` — minimum route score for solver routing
    RouteScore,
    /// `require solver_bond >= <amount>` — minimum solver bond required
    SolverBond,
    /// `require proof_complete <proof_type>` — assertion that a proof was completed
    ProofComplete,
    /// `require refund_path <target>` — ensures a refund path exists
    RefundPath,
    /// `require finality_explicit <chain> == <status>` — explicit finality requirement
    FinalityExplicit,
    /// `require vm_supported <vm>` — requires a specific VM is supported
    VmSupported,
    /// `require mainnet_safe` — requires mainnet safety checks pass
    MainnetSafe,
    /// Custom / catch-all require
    Custom(Symbol),
}

impl RequireKind {
    /// Whether a guard of this kind *asserts a property* rather than stating a
    /// bound.
    ///
    /// A bound is read as a number by something — the linter, the risk profile,
    /// the mainnet gates, the nonce metadata — so it cannot be written without a
    /// value: a reader that found none would take its "nothing to check" branch,
    /// which is the one answer that makes a check disappear.
    ///
    /// A property guard names what must hold and needs no number:
    /// `require proof_complete`, `require canonical_supply USDC`.
    pub fn asserts_a_property(&self) -> bool {
        matches!(
            self,
            RequireKind::CanonicalSupply
                | RequireKind::ProofComplete
                | RequireKind::RefundPath
                | RequireKind::VmSupported
                | RequireKind::MainnetSafe
                | RequireKind::AuditGate
                | RequireKind::InvariantCheck
                | RequireKind::Custom(_)
        )
    }

    /// The name this kind is written and reported as.
    pub fn as_str(&self) -> &str {
        match self {
            RequireKind::Finality => "finality",
            RequireKind::Slippage => "slippage",
            RequireKind::Profit => "profit",
            RequireKind::InvariantCheck => "invariant",
            RequireKind::RiskScore => "risk",
            RequireKind::Nonce => "nonce",
            RequireKind::AuditGate => "audit_gate",
            RequireKind::BridgeLiquidity => "bridge_liquidity",
            RequireKind::CanonicalSupply => "canonical_supply",
            RequireKind::RelayerQuorum => "relayer_quorum",
            RequireKind::RouteScore => "route_score",
            RequireKind::SolverBond => "solver_bond",
            RequireKind::ProofComplete => "proof_complete",
            RequireKind::RefundPath => "refund_path",
            RequireKind::FinalityExplicit => "finality_explicit",
            RequireKind::VmSupported => "vm_supported",
            RequireKind::MainnetSafe => "mainnet_safe",
            RequireKind::Custom(name) => name.as_str(),
        }
    }
}

/// A `require` guard inside a cross-chain declaration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequireGuard {
    /// What property is being asserted.
    pub kind: RequireKind,
    /// Optional subject: chain name for Finality, invariant name for InvariantCheck, etc.
    pub subject: Option<Symbol>,
    /// The comparison the guard makes, when it makes one.
    ///
    /// This used to be parsed and thrown away, so `require slippage <= 50` and
    /// `require slippage >= 50` were the same program: the direction of every
    /// guard in the language was discarded at the parser, and the number that
    /// survived was only a number. Every check that reads a guard as a ceiling
    /// or a floor was therefore reading a direction nobody had stated.
    ///
    /// `None` is a guard with no comparison at all, such as
    /// `require nonce unused <id>`, which asserts a property rather than a
    /// threshold.
    #[serde(default)]
    pub comparison: Option<ComparisonOp>,
    /// The threshold or target expression (the RHS of the comparison).
    ///
    /// `None` is a guard that names a property rather than comparing against a
    /// number: `require canonical_supply USDC` says the canonical supply of
    /// USDC must hold, and there is no right-hand side to compare it to. What it
    /// names is its `subject`, so a guard with neither a value nor a subject is
    /// refused by the parser — a `require` that names nothing asserts nothing.
    #[serde(default)]
    pub value: Option<Expression>,
}

/// The comparison a guard makes between the quantity it names and its value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ComparisonOp {
    Less,
    LessOrEqual,
    Greater,
    GreaterOrEqual,
    Equal,
    NotEqual,
}

impl ComparisonOp {
    pub fn as_str(self) -> &'static str {
        match self {
            ComparisonOp::Less => "<",
            ComparisonOp::LessOrEqual => "<=",
            ComparisonOp::Greater => ">",
            ComparisonOp::GreaterOrEqual => ">=",
            ComparisonOp::Equal => "==",
            ComparisonOp::NotEqual => "!=",
        }
    }

    /// Whether the guard names an upper bound on the quantity.
    pub fn is_upper_bound(self) -> bool {
        matches!(self, ComparisonOp::Less | ComparisonOp::LessOrEqual)
    }

    /// Whether the guard names a lower bound on the quantity.
    pub fn is_lower_bound(self) -> bool {
        matches!(self, ComparisonOp::Greater | ComparisonOp::GreaterOrEqual)
    }
}

/// One leg of a `parallel` block.
///
/// A leg is a body, not an expression: the compiler decides whether legs are
/// independent from what their operations actually read and write, and an
/// opaque call would give it nothing to decide with.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParallelLeg {
    pub name: Symbol,
    pub body: Vec<Statement>,
}

/// `parallel <name> { leg <name> { ... } ... }` — legs that may run
/// concurrently where the compiler can prove they are independent.
///
/// Declaring legs here does not make them concurrent: the compiler builds the
/// dependency DAG and the artifact carries the plan it produced. A leg that
/// depends on another is ordered by an edge, and two legs that would race are
/// refused rather than sequenced silently.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParallelDecl {
    pub name: Symbol,
    pub legs: Vec<ParallelLeg>,
}

/// `objective { maximize net_profit; constraints { … } }` — what a program is
/// asking the planner to do, declared rather than passed on a command line.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObjectiveDecl {
    pub name: Symbol,
    pub metric: ObjectiveMetric,
    pub constraints: ObjectiveConstraints,
}

/// What is being maximised or minimised.
///
/// The spec lists eight; this carries all eight so the compiler can say *why*
/// one of them is not usable rather than failing to parse it. Whether the
/// optimizer can rank it is a separate question, answered in the compiler.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ObjectiveMetric {
    MaximizeProfit,
    MaximizeOutput,
    MinimizeFees,
    MinimizeSlippage,
    MinimizeExecutionTime,
    MinimizeExternalLiquidity,
    MinimizeRisk,
    MaximizeCapitalEfficiency,
    /// Not in the spec's list, but a venue declares it and the optimizer ranks
    /// it, so it would be odd to leave it out of the surface.
    MinimizeFinality,
}

impl ObjectiveMetric {
    /// Whether the metric is maximised or minimised.
    ///
    /// The direction is part of the metric rather than a modifier on it: there
    /// is no such thing as maximising fees, and a program that says so has a
    /// mistake the parser can name instead of a keyword it never heard of.
    pub fn direction(self) -> &'static str {
        match self {
            ObjectiveMetric::MaximizeProfit
            | ObjectiveMetric::MaximizeOutput
            | ObjectiveMetric::MaximizeCapitalEfficiency => "maximize",
            ObjectiveMetric::MinimizeFees
            | ObjectiveMetric::MinimizeSlippage
            | ObjectiveMetric::MinimizeExecutionTime
            | ObjectiveMetric::MinimizeExternalLiquidity
            | ObjectiveMetric::MinimizeRisk
            | ObjectiveMetric::MinimizeFinality => "minimize",
        }
    }

    /// The metric itself, without its direction.
    ///
    /// This, not the phrase, is what the parser matches: the direction is read
    /// separately so `maximize fees` is a mismatch with an explanation rather
    /// than an unknown word.
    pub fn name(self) -> &'static str {
        match self {
            ObjectiveMetric::MaximizeProfit => "net_profit",
            ObjectiveMetric::MaximizeOutput => "output",
            ObjectiveMetric::MinimizeFees => "fees",
            ObjectiveMetric::MinimizeSlippage => "slippage",
            ObjectiveMetric::MinimizeExecutionTime => "execution_time",
            ObjectiveMetric::MinimizeExternalLiquidity => "external_liquidity",
            ObjectiveMetric::MinimizeRisk => "risk",
            ObjectiveMetric::MaximizeCapitalEfficiency => "capital_efficiency",
            ObjectiveMetric::MinimizeFinality => "finality",
        }
    }

    /// The phrase the spec and the diagnostics use.
    pub fn as_str(self) -> &'static str {
        match self {
            ObjectiveMetric::MaximizeProfit => "maximize net_profit",
            ObjectiveMetric::MaximizeOutput => "maximize output",
            ObjectiveMetric::MinimizeFees => "minimize fees",
            ObjectiveMetric::MinimizeSlippage => "minimize slippage",
            ObjectiveMetric::MinimizeExecutionTime => "minimize execution time",
            ObjectiveMetric::MinimizeExternalLiquidity => "minimize external liquidity",
            ObjectiveMetric::MinimizeRisk => "minimize risk",
            ObjectiveMetric::MaximizeCapitalEfficiency => "maximize capital efficiency",
            ObjectiveMetric::MinimizeFinality => "minimize finality",
        }
    }

    pub const ALL: [ObjectiveMetric; 9] = [
        ObjectiveMetric::MaximizeProfit,
        ObjectiveMetric::MaximizeOutput,
        ObjectiveMetric::MinimizeFees,
        ObjectiveMetric::MinimizeSlippage,
        ObjectiveMetric::MinimizeExecutionTime,
        ObjectiveMetric::MinimizeExternalLiquidity,
        ObjectiveMetric::MinimizeRisk,
        ObjectiveMetric::MaximizeCapitalEfficiency,
        ObjectiveMetric::MinimizeFinality,
    ];

    /// The metric a bare name denotes, ignoring whether it is maximised or
    /// minimised. The caller checks the direction, so it can say *why* a name
    /// and a direction do not go together.
    ///
    /// `profit` and `net_profit` are one metric: PHASE 15's example writes
    /// `maximize net_profit` in the declaration and lists `maximize profit`
    /// among the objectives, and a program following either spelling is asking
    /// for the same thing. Accepting the second spelling matters because the
    /// metric is one the graph cannot rank — without it the spec's own example
    /// would report an unknown word instead of the reason it cannot be ranked.
    pub fn by_name(name: &str) -> Option<ObjectiveMetric> {
        if name == "profit" {
            return Some(ObjectiveMetric::MaximizeProfit);
        }
        ObjectiveMetric::ALL
            .iter()
            .copied()
            .find(|metric| metric.name() == name)
    }
}

/// A ceiling the planner must respect.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ObjectiveConstraints {
    pub max_hops: Option<u32>,
    /// Distinct chains a route may touch.
    pub max_chains: Option<u32>,
    pub max_risk: Option<RiskBound>,
    pub max_execution_time_ms: Option<u32>,
    pub max_fees_bps: Option<u32>,
    pub max_slippage_bps: Option<u32>,
    pub max_finality_blocks: Option<u32>,
    /// The size the route must be able to absorb, from `capital <= <N> <ASSET>`.
    pub capital: Option<crate::trading::AmountExpr>,
    /// `private` — the submission has to travel privately.
    pub private: bool,
    /// `atomic` — the execution has to be all-or-nothing. Recorded rather than
    /// checked: everything this language lowers is atomic already.
    pub atomic: bool,
}

/// Either a number or the enclosing strategy module's declared profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RiskBound {
    Score(u32),
    /// `risk <= strategy.policy` — bound risk by what the module declared,
    /// rather than restating a number that could drift from it.
    StrategyPolicy,
}

/// What kind of node a `venue` declaration introduces.
///
/// One closed set rather than a declaration shape per kind: a pool, an
/// orderbook, a lending market, a perp market, a flash-liquidity source, a
/// bridge adapter and a settlement path all appear in the opportunity graph as
/// nodes with the same edge attributes, and the compiler can only reason about
/// them together if they are one kind of thing with a discriminant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VenueKind {
    /// A constant-product/AMM style pool.
    Pool,
    /// A central-limit orderbook.
    Orderbook,
    /// A lending market (a source or sink of debt).
    Lending,
    /// A perpetuals venue.
    Perp,
    /// A flash-liquidity source: capital that must be returned within the
    /// transaction.
    Flash,
    /// A bridge or cross-domain adapter.
    Bridge,
    /// A settlement path: how a position is finally closed.
    Settlement,
}

impl VenueKind {
    pub fn as_str(self) -> &'static str {
        match self {
            VenueKind::Pool => "pool",
            VenueKind::Orderbook => "orderbook",
            VenueKind::Lending => "lending",
            VenueKind::Perp => "perp",
            VenueKind::Flash => "flash",
            VenueKind::Bridge => "bridge",
            VenueKind::Settlement => "settlement",
        }
    }

    /// The kinds a program may declare. `parse` and the unknown-kind error
    /// message both read this, so the two cannot list different sets.
    pub const ALL: &'static [VenueKind] = &[
        VenueKind::Pool,
        VenueKind::Orderbook,
        VenueKind::Lending,
        VenueKind::Perp,
        VenueKind::Flash,
        VenueKind::Bridge,
        VenueKind::Settlement,
    ];

    pub fn parse(name: &str) -> Option<VenueKind> {
        VenueKind::ALL.iter().copied().find(|kind| kind.as_str() == name)
    }

    /// Whether a venue of this kind can be a graph edge's intermediate stop,
    /// as opposed to only funding (`flash`) or only terminating (`settlement`)
    /// a path. A flash source is returned within the transaction, so it never
    /// takes the path anywhere; a settlement path is where the path ends.
    pub fn is_traversable(self) -> bool {
        matches!(
            self,
            VenueKind::Pool | VenueKind::Orderbook | VenueKind::Lending | VenueKind::Perp | VenueKind::Bridge
        )
    }
}

/// `venue <name> { kind pool chain ethereum ... }` — one node of the
/// opportunity graph, with the attributes its edges carry.
///
/// The declared attributes are what make the graph searchable and what let a
/// `fallback`'s bounds be checked against something: before this, a venue was
/// only a name, so `require slippage <= 7` had nothing to compare against.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VenueDecl {
    pub name: Symbol,
    pub kind: VenueKind,
    /// Chain the venue settles on.
    pub chain: ChainRef,
    /// VM family hosting the venue (for example `evm`, `svm`).
    pub domain: Symbol,
    /// Asset the venue takes in.
    pub asset_in: AssetRef,
    /// Asset the venue gives out. Equal to `asset_in` for a pure lending or
    /// flash venue, which moves one asset and charges a fee for it.
    pub asset_out: AssetRef,
    /// Venue fee in basis points.
    pub fee_bps: u32,
    /// Declared depth, as an amount of `asset_in`.
    pub liquidity: u128,
    /// Slippage in basis points at the declared liquidity. A function would be
    /// more expressive; a bound is what the compiler can check today, and an
    /// unchecked function would be a promise nothing enforces.
    pub slippage_bps: u32,
    /// Expected latency to settlement, in milliseconds.
    pub latency_ms: u32,
    /// Blocks of finality the venue's settlement requires.
    pub finality_blocks: u32,
    /// Declared risk score, 0 (safest) to 100.
    pub risk: u32,
    /// Proof a claim against this venue must carry, if any.
    pub proof: Option<Symbol>,
}

/// One approved substitution in a `fallback` block.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FallbackReplacement {
    /// The venue the failing leg may be re-routed through.
    pub venue: Symbol,
    /// Replacement `min_output`, when the substitute is expected to fill less
    /// than the leg it replaces. `None` keeps the leg's own bound.
    pub min_output: Option<Expression>,
}

/// What to do when a cross-chain operation fails or times out.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FailureAction {
    /// Roll back all state changes in the current atomic scope.
    Rollback,
    /// Refund a specific asset/amount back to an account.
    Refund(Expression),
    /// Halt the bridge / freeze settlement.
    Halt,
    /// Quarantine the operation for manual review.
    Quarantine,
}

/// Top-level `bridge` declaration.
/// Declares a named bridge route with its invariant guards and failure policy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BridgeDecl {
    pub name: Symbol,
    pub from_asset: AssetRef,
    pub to_asset: AssetRef,
    /// The ordered list of operations in the bridge body.
    pub body: Vec<Statement>,
    /// Compiled list of `require` guards (also present inline in `body`).
    pub requires: Vec<RequireGuard>,
    /// What to do on failure.
    pub on_fail: Option<FailureAction>,
    /// Maximum execution time before timeout fires.
    pub timeout: Option<Expression>,
}

/// Specification of a hashlock guard.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HashlockSpec {
    /// Hash function name: "sha256" or "blake2b"
    pub hash_fn: Symbol,
    /// The secret expression that will be hashed.
    pub secret: Box<Expression>,
}

/// `atomic swap <from> -> <to> { ... }` — a named atomic multi-step cross-chain swap.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AtomicSwapDecl {
    pub name: Symbol,
    pub from_asset: AssetRef,
    pub to_asset: AssetRef,
    /// VM family for the source chain (e.g. "Evm", "Svm", "MoveVm").
    pub source_vm: Option<String>,
    /// VM family for the destination chain (e.g. "Evm", "Svm", "MoveVm").
    pub dest_vm: Option<String>,
    pub amount: Option<Expression>,
    pub receiver: Option<Expression>,
    pub hashlock: Option<HashlockSpec>,
    pub body: Vec<Statement>,
    pub requires: Vec<RequireGuard>,
    pub on_fail: Option<FailureAction>,
    pub timeout_source: Option<Expression>,
    pub timeout_destination: Option<Expression>,
}

/// How an `atomic_choice` picks one of its paths.
///
/// A closed set on purpose. The compiler has to "prohibit arbitrary runtime
/// code mutation", so the choice is a criterion the compiler understands and
/// can evaluate over the paths it verified — never an expression whose value
/// decides at run time which code runs. Adding a criterion means teaching the
/// compiler how to rank paths by it, which is exactly the review point that
/// keeps the set closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChoiceCriterion {
    /// Take the path whose declared `net_output` is largest.
    HighestNetOutput,
    /// Take the path with the fewest hops.
    FewestHops,
}

impl ChoiceCriterion {
    pub fn as_str(self) -> &'static str {
        match self {
            ChoiceCriterion::HighestNetOutput => "highest_net_output",
            ChoiceCriterion::FewestHops => "fewest_hops",
        }
    }

    /// The criteria the language accepts. Used both by the parser and by the
    /// error message for an unknown one, so the two cannot list different sets.
    pub const ALL: &'static [ChoiceCriterion] = &[ChoiceCriterion::HighestNetOutput, ChoiceCriterion::FewestHops];

    pub fn parse(name: &str) -> Option<ChoiceCriterion> {
        ChoiceCriterion::ALL.iter().copied().find(|c| c.as_str() == name)
    }
}

/// One `path <name> { ... }` arm of an `atomic_choice`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChoicePath {
    pub name: Symbol,
    /// Executable body, in the same statement grammar every other block uses.
    pub body: Vec<Statement>,
    /// `A -> B -> C`, when the path is written as a hop chain. Kept so the
    /// route's shape survives parsing; resolving hops to venues is the
    /// opportunity graph's job, not this node's.
    pub hops: Vec<AssetRef>,
    /// `net_output <amount>` — what this path claims to produce. Required by
    /// `choose highest_net_output`, because a criterion the compiler cannot
    /// evaluate over every path is not a criterion.
    pub net_output: Option<crate::AmountExpr>,
}

/// `atomic_choice { path A { ... } path B { ... } choose <criterion> }` —
/// bounded branch execution.
///
/// The paths are the permitted branches: every one is parsed, lowered and
/// verified, the compiler picks one by the declared criterion, and the
/// artifact records both the full branch set and which one was taken. Nothing
/// at run time can choose a branch the compiler did not verify.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AtomicChoiceDecl {
    pub name: Symbol,
    pub paths: Vec<ChoicePath>,
    pub criterion: ChoiceCriterion,
}

/// `strategy <name> { ... }` — a constrained execution strategy (arb, liquidation, etc.).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrossChainStrategy {
    pub name: Symbol,
    /// Maximum number of execution steps.
    pub max_steps: Option<Expression>,
    /// Maximum gas the strategy may consume.
    pub max_gas: Option<Expression>,
    pub body: Vec<Statement>,
    pub requires: Vec<RequireGuard>,
    pub on_fail: Option<FailureAction>,
    /// `input <ASSET> amount <N>` — what the module is handed. One or more.
    pub inputs: Vec<StrategyInput>,
    /// `output <ASSET>` — what it produces.
    pub outputs: Vec<AssetRef>,
    /// `effects [ ... ]` — the economic effects the body is expected to have.
    pub effects: Vec<crate::trading::TradeEffect>,
    /// `guarantees [ ... ]` — what the module promises holds afterwards.
    pub guarantees: Vec<crate::trading::TradeGuarantee>,
    /// `permissions [ ... ]` — what the body is allowed to do beyond its
    /// declared effects.
    pub permissions: Vec<StrategyPermission>,
    /// `domains [ ... ]` — the chains the module needs.
    pub domains: Vec<Symbol>,
    /// `risk { ... }` — the bounds the module accepts.
    pub risk: Option<StrategyRisk>,
    /// `license { ... }` — who wrote the module and what its use earns them.
    ///
    /// Optional: a module need not be licensed. When it is, the royalty is
    /// checked against the profit split, because a royalty that the split does
    /// not pay is a promise the artifact does not keep.
    pub license: Option<StrategyLicense>,
    /// `split profit { ... }` — how net profit is distributed.
    pub split: Option<ProfitSplit>,
    /// `submission { private = ... }` — the submission policy the module is
    /// compiled with.
    ///
    /// PHASE 28: the runtime has to reject an accidental public submission when
    /// the compiled policy requires privacy, so this is a *requirement the
    /// artifact states* rather than a description of the source.
    pub submission: Option<SubmissionPolicy>,
}

/// How a module requires its submission to travel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PrivateSubmissionMode {
    /// A public submission is acceptable.
    Allowed,
    /// A private channel is preferred but not required.
    Preferred,
    /// A private channel is required; a runtime without one must refuse.
    Required,
}

impl PrivateSubmissionMode {
    pub fn as_str(self) -> &'static str {
        match self {
            PrivateSubmissionMode::Allowed => "allowed",
            PrivateSubmissionMode::Preferred => "preferred",
            PrivateSubmissionMode::Required => "required",
        }
    }

    pub const ALL: [PrivateSubmissionMode; 3] = [
        PrivateSubmissionMode::Allowed,
        PrivateSubmissionMode::Preferred,
        PrivateSubmissionMode::Required,
    ];

    pub fn parse(name: &str) -> Option<PrivateSubmissionMode> {
        PrivateSubmissionMode::ALL
            .iter()
            .copied()
            .find(|mode| mode.as_str() == name)
    }
}

/// `submission { private = <mode> }`
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct SubmissionPolicy {
    pub private: PrivateSubmissionMode,
}

/// `license { creator <who> profit_share <N>% [executions <N>] [expires_block <N>] }`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyLicense {
    /// Who the licence is granted to and by — an opaque handle, as written.
    pub creator: Symbol,
    /// The author's share of net profit, in basis points.
    ///
    /// Basis points rather than a float: PHASE 42 forbids floating-point
    /// ambiguity in anything that moves money, and a share is money.
    pub profit_share_bps: u32,
    /// Per-execution entitlement, if the licence is sold by execution count.
    pub executions: Option<u128>,
    /// Block at which the licence expires, if it is time-limited.
    pub expires_block: Option<u64>,
}

/// `split profit { 70% -> trader ... }` — the distribution of net profit.
///
/// Every share is in basis points and the shares must total exactly 10,000: a
/// split that does not add up is distributing something it does not have, or
/// quietly leaving part of the profit unassigned.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfitSplit {
    pub shares: Vec<(SplitRecipient, u32)>,
}

/// Who a profit share is paid to.
///
/// A closed set. The alternative — free-form recipient handles — would make the
/// royalty check impossible, because "does the author get paid" would depend on
/// whether a name in the split happens to match a name in the licence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum SplitRecipient {
    Trader,
    LiquidityProvider,
    ValidatorPool,
    StrategyAuthor,
}

impl SplitRecipient {
    pub fn as_str(self) -> &'static str {
        match self {
            SplitRecipient::Trader => "trader",
            SplitRecipient::LiquidityProvider => "liquidity_provider",
            SplitRecipient::ValidatorPool => "validator_pool",
            SplitRecipient::StrategyAuthor => "strategy_author",
        }
    }

    pub const ALL: [SplitRecipient; 4] = [
        SplitRecipient::Trader,
        SplitRecipient::LiquidityProvider,
        SplitRecipient::ValidatorPool,
        SplitRecipient::StrategyAuthor,
    ];

    pub fn parse(name: &str) -> Option<SplitRecipient> {
        SplitRecipient::ALL
            .iter()
            .copied()
            .find(|recipient| recipient.as_str() == name)
    }
}

/// One `input` of a strategy module.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyInput {
    pub asset: AssetRef,
    /// How much the module may take. `None` means it does not say, which the
    /// verifier refuses: a capital figure nobody bounded is a capital figure
    /// nobody agreed to.
    pub amount: Option<Expression>,
    /// The most the module will take of this asset, when it says.
    ///
    /// Optional because a module may genuinely be indifferent above its
    /// minimum; absent means "unstated", which the metadata reports as a null
    /// maximum rather than inventing one.
    pub max_amount: Option<Expression>,
}

/// A module's declared risk profile.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct StrategyRisk {
    pub max_slippage_bps: u32,
    pub max_total_fee_bps: u32,
}

/// What a strategy module's body is allowed to do beyond its declared effects.
///
/// A closed set, for the same reason every other permission here is closed: a
/// permission the compiler does not understand is a permission it cannot check,
/// and "permissions" that are not checked are decoration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StrategyPermission {
    /// Submit through a private channel.
    PrivateSubmission,
    /// Take flash liquidity that must be returned within the transaction.
    FlashCapital,
    /// Allow the intent to be netted against others.
    IntentFusion,
    /// Touch more than one chain.
    CrossDomain,
}

impl StrategyPermission {
    pub fn as_str(self) -> &'static str {
        match self {
            StrategyPermission::PrivateSubmission => "private_submission",
            StrategyPermission::FlashCapital => "flash_capital",
            StrategyPermission::IntentFusion => "intent_fusion",
            StrategyPermission::CrossDomain => "cross_domain",
        }
    }

    /// The permissions the language accepts. The parser and the unknown-name
    /// error message both read this, so they cannot list different sets.
    pub const ALL: [StrategyPermission; 4] = [
        StrategyPermission::PrivateSubmission,
        StrategyPermission::FlashCapital,
        StrategyPermission::IntentFusion,
        StrategyPermission::CrossDomain,
    ];

    pub fn parse(name: &str) -> Option<StrategyPermission> {
        StrategyPermission::ALL
            .iter()
            .copied()
            .find(|permission| permission.as_str() == name)
    }
}

/// `proposal { ... }` — an on-chain governance proposal.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProposalDecl {
    pub name: Symbol,
    /// Human-readable title expression (usually a string literal).
    pub title: Option<Expression>,
    pub body: Vec<Statement>,
    pub requires: Vec<RequireGuard>,
}

// ============================================================
// B-52 Feature Lock
// ============================================================

/// `vm { chain ..., adapter ..., finality ... }` — declares a VM target with adapter and finality config.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VmDecl {
    pub chain: Symbol,
    pub adapter: Symbol,
    pub finality: Option<Symbol>,
}

/// `solver_market { mode ..., min_reputation ... }` — configures solver marketplace parameters.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SolverMarket {
    pub mode: Symbol,
    pub min_reputation: u64,
    /// The bond a solver must post, from `bond <amount> <ASSET>`.
    ///
    /// Optional in the grammar so existing programs keep parsing, but a program
    /// that writes `require solver_bond >= N` without one is rejected: the guard
    /// has nothing to compare against. A bond is an amount, so it carries the
    /// asset it is denominated in — the same rule that made `max_gas` need
    /// `max_gas_asset` before it could be enforced.
    #[serde(default)]
    pub bond: Option<crate::AmountExpr>,
}

/// `relayers { quorum 3_of_5 ... }` — declares a relayer swarm with quorum configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelayerSwarm {
    pub quorum_numerator: u32,
    pub quorum_denominator: u32,
    pub relayers: Vec<Symbol>,
}

/// `rpc_quorum { source require 2_of_3 ... }` — declares RPC consensus requirements.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcQuorum {
    pub source: Symbol,
    pub require_numerator: u32,
    pub require_denominator: u32,
    pub reject_on: Vec<Symbol>,
}

/// `risk_policy { max_slippage ... }` — risk management policy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskPolicy {
    pub max_slippage: u64,
    pub max_position: Option<u128>,
    /// `min_route_score <n>` — the score a route must reach for this program's
    /// `require route_score >= n` guard to have something to compare against.
    ///
    /// The guard is a claim about the route; this is where the program states the
    /// score it accepts. Without it the guard was a claim nothing backed: no
    /// declaration, no run-time quantity, and an executor instruction that treats
    /// it as true.
    #[serde(default)]
    pub min_route_score: Option<u32>,
}

/// `privacy { hide_route_until_commit ... }` — privacy configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrivacyBlock {
    pub hide_route_until_commit: bool,
    pub reveal_on: Symbol,
    pub encrypted: bool,
}

/// `invariant no_double_claim` — named invariant assertion declaration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvariantDecl {
    pub name: Symbol,
    pub assert_expr: Symbol,
}

/// `error SlippageExceeded` — user-defined error declaration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorDecl {
    pub name: Symbol,
}

/// `finality_policy strict { chain evm requirement finalized blocks 32 }` —
/// finality configuration per chain.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FinalityPolicy {
    pub mode: Symbol,
    pub chain: Symbol,
    pub requirement: Symbol,
    /// The depth the program requires of this chain, in blocks.
    ///
    /// A mode (`requirement finalized`) says *what* the chain must reach; it
    /// cannot answer `require finality.arbitrum >= 32`, which is a claim about
    /// how many blocks deep the chain must be. Nine corpus programs wrote such a
    /// guard and nothing declared a depth anywhere, so every one of them lowered
    /// to a `REQUIRE` the executor treats as true. The depth is optional because
    /// a program that only makes mode claims does not have one to state, and a
    /// guard that needs it is refused by name when it is absent.
    #[serde(default)]
    pub blocks: Option<u32>,
}

/// `proofs required { source_lock_proof ... }` — required proof declarations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProofsRequired {
    pub proofs: Vec<Symbol>,
}

/// `target evm { adapter ..., contract ... }` — VM target binding.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VmTarget {
    pub vm: Symbol,
    pub adapter: Symbol,
    pub contract: Option<Symbol>,
}

/// Types used in the AST
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TypeExpr {
    Path(Vec<Symbol>),
    Array(Box<TypeExpr>, Option<usize>),
    Tuple(Vec<TypeExpr>),
    Primitive(Symbol),
    Generic { base: Box<TypeExpr>, args: Vec<TypeExpr> },
    Func { params: Vec<TypeExpr>, ret: Box<TypeExpr> },
    Option(Box<TypeExpr>),
}

/// Expressions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Expression {
    Literal(LiteralExpr),
    Ident(Symbol),
    Binary {
        op: BinOp,
        lhs: Box<Expression>,
        rhs: Box<Expression>,
    },
    Unary {
        op: UnOp,
        expr: Box<Expression>,
    },
    Call {
        callee: Box<Expression>,
        args: Vec<Expression>,
    },
    MethodCall {
        receiver: Box<Expression>,
        method: Symbol,
        args: Vec<Expression>,
    },
    FieldAccess {
        target: Box<Expression>,
        field: Symbol,
    },
    Index {
        target: Box<Expression>,
        index: Box<Expression>,
    },
    IfExpr {
        cond: Box<Expression>,
        then_block: Box<Block>,
        else_block: Option<Box<Block>>,
    },
    BlockExpr(Block),
    Closure {
        params: Vec<Parameter>,
        body: Box<Expression>,
        is_async: bool,
    },
    Await(Box<Expression>),
    Async(Box<Expression>),
    Match {
        expr: Box<Expression>,
        arms: Vec<(Pattern, Expression)>,
    },
    Try(Box<Expression>),
    Atomic(Box<AtomicBlock>),
}

/// Literal expression (matching token Literal)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LiteralExpr {
    Int {
        value: u128,
        base: IntBase,
        suffix: Option<IntSuffix>,
    },
    Float {
        raw: Symbol,
        suffix: Option<FloatSuffix>,
    },
    String(Symbol),
    RawString {
        value: Symbol,
        hash_count: u8,
    },
    ByteString(Vec<u8>),
    Char(char),
    Byte(u8),
    Address(Symbol),
    Hash(Symbol),
    Percentage {
        value: Symbol,
    },
    Duration {
        value: u64,
        unit: DurationUnit,
    },
    Size {
        value: u64,
        unit: SizeUnit,
    },
    Bool(bool),
    Unit,
}

impl LiteralExpr {
    pub fn is_truthy(&self) -> bool {
        match self {
            LiteralExpr::Bool(b) => *b,
            LiteralExpr::Int { value, .. } => *value != 0,
            LiteralExpr::Float { raw, .. } => raw.as_str() != "0.0",
            LiteralExpr::String(s) => !s.as_str().is_empty(),
            _ => true,
        }
    }
}

// === Helpers ===

/// Walk the program AST using the provided visitor.
impl Program {
    pub fn walk(&self, v: &mut dyn crate::visitor::AstVisitor) {
        v.enter_program(self);
        for item in &self.items {
            v.enter_item(item);
            match &item.node {
                Item::Agent(a) => v.visit_agent(a),
                Item::Function(f) => v.visit_function(f),
                Item::Struct(s) => v.visit_struct(s),
                Item::Enum(e) => v.visit_enum(e),
                Item::Bridge(b) => v.visit_bridge(b),
                Item::AtomicSwap(a) => v.visit_atomic_swap(a),
                Item::AtomicChoice(c) => v.visit_atomic_choice(c),
                Item::Strategy(s) => v.visit_cross_chain_strategy(s),
                Item::Proposal(p) => v.visit_proposal(p),
                Item::VmDecl(d) => v.visit_vm_decl(d),
                Item::SolverMarket(m) => v.visit_solver_market(m),
                Item::RelayerSwarm(r) => v.visit_relayer_swarm(r),
                Item::RpcQuorum(q) => v.visit_rpc_quorum(q),
                Item::RiskPolicy(p) => v.visit_risk_policy(p),
                Item::PrivacyBlock(p) => v.visit_privacy_block(p),
                Item::InvariantDecl(i) => v.visit_invariant_decl(i),
                Item::ErrorDecl(e) => v.visit_error_decl(e),
                Item::FinalityPolicy(f) => v.visit_finality_policy(f),
                Item::ProofsRequired(p) => v.visit_proofs_required(p),
                Item::VmTarget(t) => v.visit_vm_target(t),
                Item::TradeRiskPolicy(p) => v.visit_trade_risk_policy(p),
                _ => (),
            }
            v.exit_item(item);
        }
        v.exit_program(self);
    }
}

// Keep the AST minimal, deterministic, and deterministic-friendly for serialization.
/// `atomic_hedge { buy … spot; short … perp; require delta <= 0.01%; }` — spec
/// PHASE 9.
///
/// A hedge is two directional legs on one asset inside one atomic plan, and the
/// guard is a claim about the *net* of them: `require delta <= 0.01%` says the
/// position left open after both legs is at most that fraction of the notional being
/// hedged. The verifier computes that net (`compiler/src/hedge.rs`); the type names
/// the phase suggests (`Exposure`, `Delta`, `HedgeRatio`) are what it computes
/// rather than syntax the language has to carry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AtomicHedgeDecl {
    pub legs: Vec<HedgeLeg>,
    /// The bound from `require delta <= <pct>`, in basis points: `0.01%` is 1.
    pub delta_bound_bps: Option<u32>,
}

/// One side of a hedge.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HedgeLeg {
    pub side: HedgeSide,
    pub quantity: HedgeQuantity,
    pub asset: AssetRef,
    pub venue: HedgeVenue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HedgeSide {
    /// `buy … spot` / `buy … perp` — a long position.
    Long,
    /// `short … spot` / `short … perp` — a short position.
    Short,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HedgeQuantity {
    /// A written size.
    Amount(u128),
    /// `equivalent` — the size of the leg on the other side, which is how a hedge
    /// says "short the same notional I am long" without repeating the number.
    Equivalent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HedgeVenue {
    Spot,
    Perp,
}
/// `atomic_liquidation { … }` — spec PHASE 10.
///
/// The step's five clauses are an accounting: capital advanced to repay a
/// borrower's debt, collateral seized, the swap that turns collateral into what is
/// repaid, the repayment itself, and the profit floor the caller claims. The
/// verifier (`compiler/src/liquidation.rs`) decides that the swap's minimum covers
/// the repayment, that the floor is met, and that no collateral is left
/// unaccounted for — which is the phase's "repayment and valid final position".
///
/// Amounts are required rather than optional. The spec's sketch writes
/// `liquidate borrower.position;` with no figure, and a verifier asked to check
/// repayment without knowing what was advanced has nothing to check — the same
/// reason a hedge leg states its size.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AtomicLiquidationDecl {
    /// `of <ident>(.<ident>)*` — whose position this is, recorded verbatim so the
    /// artifact says what was liquidated rather than only that something was.
    pub position: Symbol,
    /// The capital advanced to repay the debt: `liquidate <n> <ASSET> of …`.
    pub capital: (u128, AssetRef),
    /// The collateral seized: `receive <n> <ASSET> collateral`.
    pub collateral: (u128, AssetRef),
    /// `swap <n> <ASSET> -> <ASSET> min_output <n>`.
    pub swap: LiquidationSwap,
    /// `repay <n> <ASSET>`.
    pub repaid: (u128, AssetRef),
    /// `require net_profit >= <n> <ASSET>`.
    pub profit_floor: Option<(u128, AssetRef)>,
}

/// The conversion a liquidation performs on the collateral it seized.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiquidationSwap {
    pub amount: u128,
    pub from: AssetRef,
    pub to: AssetRef,
    pub min_output: u128,
}
/// `rebalance <name> { <ASSET> = <pct>; … minimize { <metric>; … } atomic; }` —
/// spec PHASE 11.
///
/// The declaration is a target portfolio and the things the plan would minimise.
/// Two things are decided from it (`compiler/src/rebalance.rs`): the weights are a
/// whole portfolio (at least two assets, none of them zero, summing to 100%), and
/// every `minimize` target is one the optimizer can actually rank — a target it
/// cannot rank is refused with the reason rather than recorded as a label nothing
/// acts on.
///
/// `atomic;` is required and not carried: a rebalance that is not atomic leaves the
/// portfolio off-target after a partial execution, so there is no non-atomic form to
/// represent. The phase's "compiler should *eventually* be able to generate the
/// transaction graph automatically" is not implemented, so no artifact is emitted
/// for a declaration today (TICKET-070).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RebalanceDecl {
    pub name: Symbol,
    /// `BTC = 40%` — the target weight of each asset, in percent.
    pub weights: Vec<(AssetRef, u32)>,
    /// `minimize { fees; slippage; }` — in the order written. The first is the
    /// criterion the compiler would rank a plan by; the rest are recorded targets,
    /// because the optimizer ranks one metric (PHASE 15).
    pub minimize: Vec<ObjectiveMetric>,
}

/// `netting <name> { consent <party>; <debtor> owes <amount> <chain.ASSET> to
/// <creditor>; … }` — spec PHASE 22.
///
/// A *book* of obligations between named parties. PHASE 22's premise is that
/// several obligations can be discharged by the transfers that remain after they
/// are offset, so less value moves than the sum of what was promised: "net
/// obligations before settlement where cryptographically valid."
///
/// The declaration is the obligation set; the analysis over it lives in
/// `compiler/src/netting.rs`, which decides each of the phase's listed benefits as
/// a measured quantity (gross movement and transfer count before and after) rather
/// than as a claim. Two things are deliberately *not* in the syntax:
///
/// - **no netting of unlike assets.** `alice owes 5 ethereum.ETH to bob; bob owes
///   5 ethereum.USDC to alice;` are not offsettable: saying 5 ETH discharges 5
///   USDC is a claim about a price, and this compiler does not have one
///   (`compiler/src/hedge.rs` refuses the same claim in a hedge for the same
///   reason). The analysis refuses it with that reason instead of picking a rate.
/// - **no netting across domains.** An obligation to deliver on Ethereum is not a
///   payment on X3, so the two are not offset against each other — that is a
///   bridge's job, and the bridge carries its own trust assumptions. Groups are
///   netted one `(domain, asset)` at a time and the report says which obligations
///   it declined to combine and why.
///
/// `consent` is not decoration: netting changes *who* pays *whom*, so a party whose
/// obligation is rewritten has to have agreed. A book that nets a party which did
/// not consent is refused, the same way a non-consenting intent is never
/// internalized into a fusion ring (`compiler/src/fusion.rs`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NettingDecl {
    pub name: Symbol,
    /// `consent <party>;` — the parties that agreed to have their obligations
    /// offset against each other, in the order written.
    pub consent: Vec<Symbol>,
    /// The obligations, in the order written.
    pub obligations: Vec<ObligationDecl>,
}

/// `<debtor> owes <amount> <chain.ASSET> to <creditor>` — one obligation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObligationDecl {
    pub debtor: Symbol,
    pub creditor: Symbol,
    /// A whole number of base units of `asset`. Amounts crossing this surface are
    /// integers for the same reason every other amount in the language is: a
    /// fractional unit would have to name its rounding.
    pub amount: u128,
    pub asset: AssetRef,
}

/// `arb <name> { discover { … } capital { … } execution { … } risk { … } }` — spec
/// PHASE 37.
///
/// The declaration is a *scope and a policy* for arbitrage: which chains may be
/// searched, how far, how much capital may be committed, what execution
/// guarantees are claimed, and the risk bounds the trade must satisfy. Nothing
/// here executes. The phase's own lowering pipeline is
/// "Opportunity Graph → Candidate Routes → Filter → Dependency DAG → Risk
/// Verification → Execution Plan → Atomic Settlement", and most of those stages
/// already exist in this compiler for other constructs — `compiler/src/arb.rs`
/// names which module implements which, and which one does not exist at all.
/// The IR verifier and the emitter refuse the operation while any stage is
/// missing, so `check` and `build` agree that this program cannot run yet.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArbDecl {
    pub name: Symbol,
    pub discover: ArbDiscover,
    pub capital: ArbCapital,
    pub execution: ArbExecution,
    pub risk: ArbRisk,
}

/// `discover { chains = […]; max_hops = <n>; liquidity_min = <n> <ASSET>; }`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArbDiscover {
    /// The chains the search may look at, in the order written. An asset outside
    /// this set is outside the scope, and the analysis refuses it rather than
    /// silently widening the search.
    pub chains: Vec<ChainRef>,
    /// The longest path the search may take. A hop count of zero is not a search.
    pub max_hops: u32,
    /// The least liquidity a candidate pool must carry to be considered.
    pub liquidity_min: Option<(u128, AssetRef)>,
}

/// `capital { flash = <bool>; max = <n> <ASSET>; }`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArbCapital {
    /// Whether the trade intends to borrow its principal. `true` is refused: spec
    /// PHASE 20 forbids shipping flash collateral before a formal safety proof,
    /// and a declaration that says `enabled` would be a claim the runtime cannot
    /// honour.
    pub flash: bool,
    /// The ceiling on committed capital. Required, because a strategy with no
    /// capital bound is not bounded (`compiler/src/arb.rs`).
    pub max: Option<(u128, AssetRef)>,
}

/// `execution { atomic = <bool>; parallel = <bool>; private = <bool>; }`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArbExecution {
    /// Required to be `true`: an arbitrage whose legs may settle separately is a
    /// set of positions, not a trade.
    pub atomic: bool,
    /// Whether independent legs may run concurrently (PHASE 16's plan).
    pub parallel: bool,
    /// Whether the trade claims private submission. `true` is refused: no private
    /// submission path exists in this compiler or VM, so the claim would be false.
    pub private: bool,
}

/// `risk { min_profit = <n>bps; max_slippage = <n>bps; max_total_fee = <n>bps;
/// deadline = <n><unit>; }`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArbRisk {
    /// The floor the trade must clear. Required: a risk block with no profit floor
    /// is a strategy with no floor.
    pub min_profit_bps: Option<u16>,
    pub max_slippage_bps: Option<u16>,
    pub max_total_fee_bps: Option<u16>,
    /// A duration expression, converted to blocks by the same reader every other
    /// duration in the language uses.
    pub deadline: Option<Expression>,
}
