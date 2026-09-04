use serde::{Deserialize, Serialize};
use x3_lang_common::{BinOp, DurationUnit, FloatSuffix, IntBase, IntSuffix, SizeUnit, Spanned, Symbol, UnOp};

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
    /// The threshold or target expression (the RHS of the comparison).
    pub value: Expression,
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
                _ => (),
            }
            v.exit_item(item);
        }
        v.exit_program(self);
    }
}

// Keep the AST minimal, deterministic, and deterministic-friendly for serialization.
