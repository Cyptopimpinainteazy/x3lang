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
    ParallelDecl(ParallelDecl),
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
    /// `swap from.ASSET -> to.ASSET [route <expr>] [min_output <expr>] [dex <expr>]`
    Swap {
        from: AssetRef,
        to: AssetRef,
        route: Option<Expression>,
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
    pub value: Expression,
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

/// `finality_policy strict { evm require finalized }` — finality configuration per chain.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FinalityPolicy {
    pub mode: Symbol,
    pub chain: Symbol,
    pub requirement: Symbol,
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
