//! Token definitions for the X3 lexer.
//!
//! This module defines all token types recognized by the X3 lexer,
//! including keywords, operators, literals, and delimiters.

use serde::{Deserialize, Serialize};
use std::fmt;
pub use x3_lang_common::{BinOp, UnOp};
use x3_lang_common::{Span, Symbol};

/// A token with its kind and span information.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

impl Token {
    pub fn new(kind: TokenKind, span: Span) -> Self {
        Token { kind, span }
    }

    pub fn dummy(kind: TokenKind) -> Self {
        Token {
            kind,
            span: Span::DUMMY,
        }
    }

    pub fn is_eof(&self) -> bool {
        matches!(self.kind, TokenKind::Eof)
    }

    pub fn is_keyword(&self, kw: Keyword) -> bool {
        matches!(&self.kind, TokenKind::Keyword(k) if *k == kw)
    }

    pub fn is_ident(&self) -> bool {
        matches!(self.kind, TokenKind::Ident(_))
    }

    pub fn is_literal(&self) -> bool {
        matches!(self.kind, TokenKind::Literal(_))
    }

    pub fn is_delimiter(&self, delim: Delimiter) -> bool {
        matches!(&self.kind, TokenKind::Delimiter(d) if *d == delim)
    }

    pub fn is_binary_op(&self) -> bool {
        matches!(self.kind, TokenKind::BinOp(_))
    }

    pub fn as_ident(&self) -> Option<Symbol> {
        match &self.kind {
            TokenKind::Ident(sym) => Some(sym.clone()),
            _ => None,
        }
    }
}

impl fmt::Debug for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?} @ {:?}", self.kind, self.span)
    }
}

impl fmt::Display for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.kind)
    }
}

/// The kind of token.
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TokenKind {
    // ===== Identifiers and Literals =====
    /// An identifier (variable name, function name, etc.)
    Ident(Symbol),
    /// A literal value (number, string, etc.)
    Literal(Literal),

    // ===== Keywords =====
    /// A keyword
    Keyword(Keyword),

    // ===== Operators =====
    /// Binary operator
    BinOp(BinOp),
    /// Unary operator
    UnOp(UnOp),
    /// Assignment operator (=)
    Eq,
    /// Compound assignment (+=, -=, etc.)
    BinOpEq(BinOp),
    /// Arrow (->)
    Arrow,
    /// Fat arrow (=>)
    FatArrow,
    /// Double colon (::)
    PathSep,
    /// Question mark (?)
    Question,
    /// At sign (@)
    At,
    /// Hash (#)
    Hash,
    /// Dollar ($)
    Dollar,
    /// Dot (.)
    Dot,
    /// Double dot (..)
    DotDot,
    /// Triple dot (...)
    DotDotDot,
    /// Double dot equals (..=)
    DotDotEq,

    // ===== Delimiters =====
    /// A delimiter (parenthesis, bracket, brace, etc.)
    Delimiter(Delimiter),
    /// Comma (,)
    Comma,
    /// Semicolon (;)
    Semi,
    /// Colon (:)
    Colon,

    // ===== Special =====
    /// Newline (significant in some contexts)
    Newline,
    /// End of file
    Eof,
    /// Unknown/invalid token
    Unknown(char),
}

impl fmt::Debug for TokenKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TokenKind::Ident(s) => write!(f, "Ident({:?})", s.as_str()),
            TokenKind::Literal(lit) => write!(f, "{:?}", lit),
            TokenKind::Keyword(kw) => write!(f, "Keyword({:?})", kw),
            TokenKind::BinOp(op) => write!(f, "BinOp({:?})", op),
            TokenKind::UnOp(op) => write!(f, "UnOp({:?})", op),
            TokenKind::Eq => write!(f, "Eq"),
            TokenKind::BinOpEq(op) => write!(f, "BinOpEq({:?})", op),
            TokenKind::Arrow => write!(f, "Arrow"),
            TokenKind::FatArrow => write!(f, "FatArrow"),
            TokenKind::PathSep => write!(f, "PathSep"),
            TokenKind::Question => write!(f, "Question"),
            TokenKind::At => write!(f, "At"),
            TokenKind::Hash => write!(f, "Hash"),
            TokenKind::Dollar => write!(f, "Dollar"),
            TokenKind::Dot => write!(f, "Dot"),
            TokenKind::DotDot => write!(f, "DotDot"),
            TokenKind::DotDotDot => write!(f, "DotDotDot"),
            TokenKind::DotDotEq => write!(f, "DotDotEq"),
            TokenKind::Delimiter(d) => write!(f, "{:?}", d),
            TokenKind::Comma => write!(f, "Comma"),
            TokenKind::Semi => write!(f, "Semi"),
            TokenKind::Colon => write!(f, "Colon"),
            TokenKind::Newline => write!(f, "Newline"),
            TokenKind::Eof => write!(f, "Eof"),
            TokenKind::Unknown(c) => write!(f, "Unknown({:?})", c),
        }
    }
}

impl fmt::Display for TokenKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TokenKind::Ident(s) => write!(f, "{}", s),
            TokenKind::Literal(lit) => write!(f, "{}", lit),
            TokenKind::Keyword(kw) => write!(f, "{}", kw),
            TokenKind::BinOp(op) => write!(f, "{}", op),
            TokenKind::UnOp(op) => write!(f, "{}", op),
            TokenKind::Eq => write!(f, "="),
            TokenKind::BinOpEq(op) => write!(f, "{}=", op),
            TokenKind::Arrow => write!(f, "->"),
            TokenKind::FatArrow => write!(f, "=>"),
            TokenKind::PathSep => write!(f, "::"),
            TokenKind::Question => write!(f, "?"),
            TokenKind::At => write!(f, "@"),
            TokenKind::Hash => write!(f, "#"),
            TokenKind::Dollar => write!(f, "$"),
            TokenKind::Dot => write!(f, "."),
            TokenKind::DotDot => write!(f, ".."),
            TokenKind::DotDotDot => write!(f, "..."),
            TokenKind::DotDotEq => write!(f, "..="),
            TokenKind::Delimiter(d) => write!(f, "{}", d),
            TokenKind::Comma => write!(f, ","),
            TokenKind::Semi => write!(f, ";"),
            TokenKind::Colon => write!(f, ":"),
            TokenKind::Newline => write!(f, "\\n"),
            TokenKind::Eof => write!(f, "<EOF>"),
            TokenKind::Unknown(c) => write!(f, "{}", c),
        }
    }
}

/// X3 keywords - including standard and domain-specific keywords.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub enum Keyword {
    // ===== Basic Keywords =====
    Fn,
    Let,
    Mut,
    Const,
    Static,
    If,
    Else,
    Match,
    For,
    While,
    Loop,
    Break,
    Continue,
    Return,
    Yield,

    // ===== Type Keywords =====
    Struct,
    Enum,
    Trait,
    Impl,
    Type,
    Where,

    // ===== Visibility =====
    Pub,
    Priv,

    // ===== Module System =====
    Mod,
    Use,
    As,
    SelfLower,
    SelfUpper,
    Super,
    Crate,

    // ===== X3 Agent Keywords =====
    Agent,
    Context,
    State,
    Strategy,
    Spawn,
    Kill,
    Join,

    // ===== X3 Atomic Execution =====
    Atomic,
    Bundle,
    Commit,
    Rollback,
    Checkpoint,

    // ===== X3 MEV/Finance Operations =====
    Flashloan,
    Route,
    Sim,
    Swap,
    Quote,
    Sandwich,
    Arbitrage,

    // ===== X3 Chain Adapters =====
    Evm,
    Svm,
    Bridge,
    Chain,

    // ===== X3 Events and Effects =====
    Emit,
    Log,
    Assert,
    Require,
    Revert,

    // ===== Boolean Literals =====
    True,
    False,

    // ===== Async =====
    Async,
    Await,

    // ===== Error Handling =====
    Try,
    Catch,
    Throw,

    // ===== Memory =====
    Box,
    Ref,
    Move,

    // ===== Unsafe =====
    Unsafe,

    // ===== External =====
    Extern,

    // ===== REAPER Economy =====
    Compute,
    Gas,
    Fee,
    Stake,
    Reward,

    // ===== X3 Cross-Chain Asset Operations =====
    Lock,
    Mint,
    Burn,
    Release,

    // ===== X3 Cross-Chain Guards =====
    Finality,
    Slippage,
    Profit,
    Invariant,
    Nonce,
    Risk,
    Timeout,
    OnFail,
    OnTimeout,

    // ===== X3 Governance =====
    Proposal,
    Quorum,
    Multisig,
    Spend,
    Refund,

    // ===== B-52 Feature Lock =====
    Adapter,
    AtomicBundle,
    BadProof,
    BestPrice,
    BestScore,
    Borrow,
    BridgeExposure,
    ChainHalt,
    CommitReveal,
    Competitive,
    Deny,
    DestFinality,
    Dispute,
    EncryptSolverBids,
    FinalityPolicy,
    Health,
    HideRouteUntilCommit,
    IndependentRpc,
    Intents,
    Lend,
    Liquidate,
    Liquidity,
    MaxFee,
    MinBond,
    MinReputation,
    Mode,
    MultiLeg,
    PauseAndRefund,
    PreventSandwich,
    Privacy,
    PrivateMempool,
    ProofConflict,
    Proofs,
    Rebalance,
    RefundAndSlash,
    RefundTo,
    Reject,
    RelayerFail,
    Relayers,
    Repay,
    Required,
    RevealPreimage,
    RiskPolicy,
    Rotate,
    RouteScore,
    RpcQuorum,
    Slash,
    SolverFail,
    SolverMarket,
    SourceFinality,
    Target,
    TestsFailed,
    Unstake,
    UseBackupRelayer,
    Vm,

    // ===== X3 Capability Keywords =====
    Gpu,
    Simd,
    Simulate,
    SimReceipt,
    Mempool,
    Schedule,
    Period,
    Intent,
    Snapshot,
    Diff,
    Crdt,
    Sandbox,
    OnChain,
    OffChain,
    Migrate,
    SelfDestruct,
    Whitelist,
    Bounty,
    Escrow,
    Pause,
    Resume,
    Subscribe,
    Vrf,
    ZkVerify,
    MpcVerify,
    Version,
    UpgradeFrom,
    NoHeap,
    NoRecursion,
    Hot,
    Audit,
    Role,
    Oracle,
    Namespace,
    StorageRef,
    Pathfind,
    GasAdaptive,
    Sponsor,
    Subscription,
}

impl Keyword {
    /// Parse a string into a keyword, if it matches.
    pub fn from_str(s: &str) -> Option<Keyword> {
        match s {
            // Basic
            "fn" => Some(Keyword::Fn),
            "let" => Some(Keyword::Let),
            "mut" => Some(Keyword::Mut),
            "const" => Some(Keyword::Const),
            "static" => Some(Keyword::Static),
            "if" => Some(Keyword::If),
            "else" => Some(Keyword::Else),
            "match" => Some(Keyword::Match),
            "for" => Some(Keyword::For),
            "while" => Some(Keyword::While),
            "loop" => Some(Keyword::Loop),
            "break" => Some(Keyword::Break),
            "continue" => Some(Keyword::Continue),
            "return" => Some(Keyword::Return),
            "yield" => Some(Keyword::Yield),

            // Types
            "struct" => Some(Keyword::Struct),
            "enum" => Some(Keyword::Enum),
            "trait" => Some(Keyword::Trait),
            "impl" => Some(Keyword::Impl),
            "type" => Some(Keyword::Type),
            "where" => Some(Keyword::Where),

            // Visibility
            "pub" => Some(Keyword::Pub),
            "priv" => Some(Keyword::Priv),

            // Module system
            "mod" => Some(Keyword::Mod),
            "use" => Some(Keyword::Use),
            "as" => Some(Keyword::As),
            "self" => Some(Keyword::SelfLower),
            "Self" => Some(Keyword::SelfUpper),
            "super" => Some(Keyword::Super),
            "crate" => Some(Keyword::Crate),

            // Agent
            "agent" => Some(Keyword::Agent),
            "context" => Some(Keyword::Context),
            "state" => Some(Keyword::State),
            "strategy" => Some(Keyword::Strategy),
            "spawn" => Some(Keyword::Spawn),
            "kill" => Some(Keyword::Kill),
            "join" => Some(Keyword::Join),

            // Atomic
            "atomic" => Some(Keyword::Atomic),
            "bundle" => Some(Keyword::Bundle),
            "commit" => Some(Keyword::Commit),
            "rollback" => Some(Keyword::Rollback),
            "checkpoint" => Some(Keyword::Checkpoint),

            // MEV/Finance
            "flashloan" => Some(Keyword::Flashloan),
            "route" => Some(Keyword::Route),
            "sim" => Some(Keyword::Sim),
            "swap" => Some(Keyword::Swap),
            "quote" => Some(Keyword::Quote),
            "sandwich" => Some(Keyword::Sandwich),
            "arbitrage" => Some(Keyword::Arbitrage),

            // Chain
            "evm" => Some(Keyword::Evm),
            "svm" => Some(Keyword::Svm),
            "bridge" => Some(Keyword::Bridge),
            "chain" => Some(Keyword::Chain),

            // Events
            "emit" => Some(Keyword::Emit),
            "log" => Some(Keyword::Log),
            "assert" => Some(Keyword::Assert),
            "require" => Some(Keyword::Require),
            "revert" => Some(Keyword::Revert),

            // Boolean
            "true" => Some(Keyword::True),
            "false" => Some(Keyword::False),

            // Async
            "async" => Some(Keyword::Async),
            "await" => Some(Keyword::Await),

            // Error handling
            "try" => Some(Keyword::Try),
            "catch" => Some(Keyword::Catch),
            "throw" => Some(Keyword::Throw),

            // Memory
            "box" => Some(Keyword::Box),
            "ref" => Some(Keyword::Ref),
            "move" => Some(Keyword::Move),

            // Unsafe
            "unsafe" => Some(Keyword::Unsafe),

            // External
            "extern" => Some(Keyword::Extern),

            // REAPER
            "compute" => Some(Keyword::Compute),
            "gas" => Some(Keyword::Gas),
            "fee" => Some(Keyword::Fee),
            "stake" => Some(Keyword::Stake),
            "reward" => Some(Keyword::Reward),

            // Cross-chain asset operations
            "lock" => Some(Keyword::Lock),
            "mint" => Some(Keyword::Mint),
            "burn" => Some(Keyword::Burn),
            "release" => Some(Keyword::Release),

            // Cross-chain guards
            "finality" => Some(Keyword::Finality),
            "slippage" => Some(Keyword::Slippage),
            "profit" => Some(Keyword::Profit),
            "invariant" => Some(Keyword::Invariant),
            "nonce" => Some(Keyword::Nonce),
            "risk" => Some(Keyword::Risk),
            "timeout" => Some(Keyword::Timeout),
            "on_fail" => Some(Keyword::OnFail),
            "on_timeout" => Some(Keyword::OnTimeout),

            // Governance
            "proposal" => Some(Keyword::Proposal),
            "quorum" => Some(Keyword::Quorum),
            "multisig" => Some(Keyword::Multisig),
            "spend" => Some(Keyword::Spend),
            "refund" => Some(Keyword::Refund),

            // Capability keywords
            "gpu" => Some(Keyword::Gpu),
            "simd" => Some(Keyword::Simd),
            "simulate" => Some(Keyword::Simulate),
            "sim_receipt" => Some(Keyword::SimReceipt),
            "mempool" => Some(Keyword::Mempool),
            "schedule" => Some(Keyword::Schedule),
            "period" => Some(Keyword::Period),
            "intent" => Some(Keyword::Intent),
            "snapshot" => Some(Keyword::Snapshot),
            "diff" => Some(Keyword::Diff),
            "crdt" => Some(Keyword::Crdt),
            "sandbox" => Some(Keyword::Sandbox),
            "on_chain" => Some(Keyword::OnChain),
            "off_chain" => Some(Keyword::OffChain),
            "migrate" => Some(Keyword::Migrate),
            "self_destruct" => Some(Keyword::SelfDestruct),
            "whitelist" => Some(Keyword::Whitelist),
            "bounty" => Some(Keyword::Bounty),
            "escrow" => Some(Keyword::Escrow),
            "pause" => Some(Keyword::Pause),
            "resume" => Some(Keyword::Resume),
            "subscribe" => Some(Keyword::Subscribe),
            "vrf" => Some(Keyword::Vrf),
            "zk_verify" => Some(Keyword::ZkVerify),
            "mpc_verify" => Some(Keyword::MpcVerify),
            "version" => Some(Keyword::Version),
            "upgrade_from" => Some(Keyword::UpgradeFrom),
            "no_heap" => Some(Keyword::NoHeap),
            "no_recursion" => Some(Keyword::NoRecursion),
            "hot" => Some(Keyword::Hot),
            "audit" => Some(Keyword::Audit),
            "role" => Some(Keyword::Role),
            "oracle" => Some(Keyword::Oracle),
            "namespace" => Some(Keyword::Namespace),
            "storage_ref" => Some(Keyword::StorageRef),
            "pathfind" => Some(Keyword::Pathfind),
            "gas_adaptive" => Some(Keyword::GasAdaptive),
            "sponsor" => Some(Keyword::Sponsor),
            "subscription" => Some(Keyword::Subscription),

            // B-52
            "adapter" => Some(Keyword::Adapter),
            "atomic_bundle" => Some(Keyword::AtomicBundle),
            "bad_proof" => Some(Keyword::BadProof),
            "best_price" => Some(Keyword::BestPrice),
            "best_score" => Some(Keyword::BestScore),
            "borrow" => Some(Keyword::Borrow),
            "bridge_exposure" => Some(Keyword::BridgeExposure),
            "chain_halt" => Some(Keyword::ChainHalt),
            "commit_reveal" => Some(Keyword::CommitReveal),
            "competitive" => Some(Keyword::Competitive),
            "deny" => Some(Keyword::Deny),
            "dest_finality" => Some(Keyword::DestFinality),
            "dispute" => Some(Keyword::Dispute),
            "encrypt_solver_bids" => Some(Keyword::EncryptSolverBids),
            "finality_policy" => Some(Keyword::FinalityPolicy),
            "health" => Some(Keyword::Health),
            "hide_route_until_commit" => Some(Keyword::HideRouteUntilCommit),
            "independent_rpc" => Some(Keyword::IndependentRpc),
            "intents" => Some(Keyword::Intents),
            "lend" => Some(Keyword::Lend),
            "liquidate" => Some(Keyword::Liquidate),
            "liquidity" => Some(Keyword::Liquidity),
            "max_fee" => Some(Keyword::MaxFee),
            "min_bond" => Some(Keyword::MinBond),
            "min_reputation" => Some(Keyword::MinReputation),
            "mode" => Some(Keyword::Mode),
            "multi_leg" => Some(Keyword::MultiLeg),
            "pause_and_refund" => Some(Keyword::PauseAndRefund),
            "prevent_sandwich" => Some(Keyword::PreventSandwich),
            "privacy" => Some(Keyword::Privacy),
            "private_mempool" => Some(Keyword::PrivateMempool),
            "proof_conflict" => Some(Keyword::ProofConflict),
            "proofs" => Some(Keyword::Proofs),
            "rebalance" => Some(Keyword::Rebalance),
            "refund_and_slash" => Some(Keyword::RefundAndSlash),
            "refund_to" => Some(Keyword::RefundTo),
            "reject" => Some(Keyword::Reject),
            "relayer_fail" => Some(Keyword::RelayerFail),
            "relayers" => Some(Keyword::Relayers),
            "repay" => Some(Keyword::Repay),
            "required" => Some(Keyword::Required),
            "reveal_preimage" => Some(Keyword::RevealPreimage),
            "risk_policy" => Some(Keyword::RiskPolicy),
            "rotate" => Some(Keyword::Rotate),
            "route_score" => Some(Keyword::RouteScore),
            "rpc_quorum" => Some(Keyword::RpcQuorum),
            "slash" => Some(Keyword::Slash),
            "solver_fail" => Some(Keyword::SolverFail),
            "solver_market" => Some(Keyword::SolverMarket),
            "source_finality" => Some(Keyword::SourceFinality),
            "target" => Some(Keyword::Target),
            "tests_failed" => Some(Keyword::TestsFailed),
            "unstake" => Some(Keyword::Unstake),
            "use_backup_relayer" => Some(Keyword::UseBackupRelayer),
            "vm" => Some(Keyword::Vm),

            _ => None,
        }
    }

    /// Get the string representation of this keyword.
    pub fn as_str(&self) -> &'static str {
        match self {
            Keyword::Fn => "fn",
            Keyword::Let => "let",
            Keyword::Mut => "mut",
            Keyword::Const => "const",
            Keyword::Static => "static",
            Keyword::If => "if",
            Keyword::Else => "else",
            Keyword::Match => "match",
            Keyword::For => "for",
            Keyword::While => "while",
            Keyword::Loop => "loop",
            Keyword::Break => "break",
            Keyword::Continue => "continue",
            Keyword::Return => "return",
            Keyword::Yield => "yield",
            Keyword::Struct => "struct",
            Keyword::Enum => "enum",
            Keyword::Trait => "trait",
            Keyword::Impl => "impl",
            Keyword::Type => "type",
            Keyword::Where => "where",
            Keyword::Pub => "pub",
            Keyword::Priv => "priv",
            Keyword::Mod => "mod",
            Keyword::Use => "use",
            Keyword::As => "as",
            Keyword::SelfLower => "self",
            Keyword::SelfUpper => "Self",
            Keyword::Super => "super",
            Keyword::Crate => "crate",
            Keyword::Agent => "agent",
            Keyword::Context => "context",
            Keyword::State => "state",
            Keyword::Strategy => "strategy",
            Keyword::Spawn => "spawn",
            Keyword::Kill => "kill",
            Keyword::Join => "join",
            Keyword::Atomic => "atomic",
            Keyword::Bundle => "bundle",
            Keyword::Commit => "commit",
            Keyword::Rollback => "rollback",
            Keyword::Checkpoint => "checkpoint",
            Keyword::Flashloan => "flashloan",
            Keyword::Route => "route",
            Keyword::Sim => "sim",
            Keyword::Swap => "swap",
            Keyword::Quote => "quote",
            Keyword::Sandwich => "sandwich",
            Keyword::Arbitrage => "arbitrage",
            Keyword::Evm => "evm",
            Keyword::Svm => "svm",
            Keyword::Bridge => "bridge",
            Keyword::Chain => "chain",
            Keyword::Emit => "emit",
            Keyword::Log => "log",
            Keyword::Assert => "assert",
            Keyword::Require => "require",
            Keyword::Revert => "revert",
            Keyword::True => "true",
            Keyword::False => "false",
            Keyword::Async => "async",
            Keyword::Await => "await",
            Keyword::Try => "try",
            Keyword::Catch => "catch",
            Keyword::Throw => "throw",
            Keyword::Box => "box",
            Keyword::Ref => "ref",
            Keyword::Move => "move",
            Keyword::Unsafe => "unsafe",
            Keyword::Extern => "extern",
            Keyword::Compute => "compute",
            Keyword::Gas => "gas",
            Keyword::Fee => "fee",
            Keyword::Stake => "stake",
            Keyword::Reward => "reward",
            Keyword::Lock => "lock",
            Keyword::Mint => "mint",
            Keyword::Burn => "burn",
            Keyword::Release => "release",
            Keyword::Finality => "finality",
            Keyword::Slippage => "slippage",
            Keyword::Profit => "profit",
            Keyword::Invariant => "invariant",
            Keyword::Nonce => "nonce",
            Keyword::Risk => "risk",
            Keyword::Timeout => "timeout",
            Keyword::OnFail => "on_fail",
            Keyword::OnTimeout => "on_timeout",
            Keyword::Proposal => "proposal",
            Keyword::Quorum => "quorum",
            Keyword::Multisig => "multisig",
            Keyword::Spend => "spend",
            Keyword::Refund => "refund",
            Keyword::Gpu => "gpu",
            Keyword::Simd => "simd",
            Keyword::Simulate => "simulate",
            Keyword::SimReceipt => "sim_receipt",
            Keyword::Mempool => "mempool",
            Keyword::Schedule => "schedule",
            Keyword::Period => "period",
            Keyword::Intent => "intent",
            Keyword::Snapshot => "snapshot",
            Keyword::Diff => "diff",
            Keyword::Crdt => "crdt",
            Keyword::Sandbox => "sandbox",
            Keyword::OnChain => "on_chain",
            Keyword::OffChain => "off_chain",
            Keyword::Migrate => "migrate",
            Keyword::SelfDestruct => "self_destruct",
            Keyword::Whitelist => "whitelist",
            Keyword::Bounty => "bounty",
            Keyword::Escrow => "escrow",
            Keyword::Pause => "pause",
            Keyword::Resume => "resume",
            Keyword::Subscribe => "subscribe",
            Keyword::Vrf => "vrf",
            Keyword::ZkVerify => "zk_verify",
            Keyword::MpcVerify => "mpc_verify",
            Keyword::Version => "version",
            Keyword::UpgradeFrom => "upgrade_from",
            Keyword::NoHeap => "no_heap",
            Keyword::NoRecursion => "no_recursion",
            Keyword::Hot => "hot",
            Keyword::Audit => "audit",
            Keyword::Role => "role",
            Keyword::Oracle => "oracle",
            Keyword::Namespace => "namespace",
            Keyword::StorageRef => "storage_ref",
            Keyword::Pathfind => "pathfind",
            Keyword::GasAdaptive => "gas_adaptive",
            Keyword::Sponsor => "sponsor",
            Keyword::Subscription => "subscription",
            // B-52
            Keyword::Adapter => "adapter",
            Keyword::AtomicBundle => "atomic_bundle",
            Keyword::BadProof => "bad_proof",
            Keyword::BestPrice => "best_price",
            Keyword::BestScore => "best_score",
            Keyword::Borrow => "borrow",
            Keyword::BridgeExposure => "bridge_exposure",
            Keyword::ChainHalt => "chain_halt",
            Keyword::CommitReveal => "commit_reveal",
            Keyword::Competitive => "competitive",
            Keyword::Deny => "deny",
            Keyword::DestFinality => "dest_finality",
            Keyword::Dispute => "dispute",
            Keyword::EncryptSolverBids => "encrypt_solver_bids",
            Keyword::FinalityPolicy => "finality_policy",
            Keyword::Health => "health",
            Keyword::HideRouteUntilCommit => "hide_route_until_commit",
            Keyword::IndependentRpc => "independent_rpc",
            Keyword::Intents => "intents",
            Keyword::Lend => "lend",
            Keyword::Liquidate => "liquidate",
            Keyword::Liquidity => "liquidity",
            Keyword::MaxFee => "max_fee",
            Keyword::MinBond => "min_bond",
            Keyword::MinReputation => "min_reputation",
            Keyword::Mode => "mode",
            Keyword::MultiLeg => "multi_leg",
            Keyword::PauseAndRefund => "pause_and_refund",
            Keyword::PreventSandwich => "prevent_sandwich",
            Keyword::Privacy => "privacy",
            Keyword::PrivateMempool => "private_mempool",
            Keyword::ProofConflict => "proof_conflict",
            Keyword::Proofs => "proofs",
            Keyword::Rebalance => "rebalance",
            Keyword::RefundAndSlash => "refund_and_slash",
            Keyword::RefundTo => "refund_to",
            Keyword::Reject => "reject",
            Keyword::RelayerFail => "relayer_fail",
            Keyword::Relayers => "relayers",
            Keyword::Repay => "repay",
            Keyword::Required => "required",
            Keyword::RevealPreimage => "reveal_preimage",
            Keyword::RiskPolicy => "risk_policy",
            Keyword::Rotate => "rotate",
            Keyword::RouteScore => "route_score",
            Keyword::RpcQuorum => "rpc_quorum",
            Keyword::Slash => "slash",
            Keyword::SolverFail => "solver_fail",
            Keyword::SolverMarket => "solver_market",
            Keyword::SourceFinality => "source_finality",
            Keyword::Target => "target",
            Keyword::TestsFailed => "tests_failed",
            Keyword::Unstake => "unstake",
            Keyword::UseBackupRelayer => "use_backup_relayer",
            Keyword::Vm => "vm",
        }
    }
}

impl fmt::Display for Keyword {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Literal values in X3.
#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Literal {
    /// Integer literal with optional suffix (e.g., 42, 100u64, 0xFF)
    Int {
        value: u128,
        suffix: Option<IntSuffix>,
        base: IntBase,
    },
    /// Floating-point literal (e.g., 3.14, 1e-10)
    Float { value: Symbol, suffix: Option<FloatSuffix> },
    /// String literal (e.g., "hello")
    String(Symbol),
    /// Raw string literal (e.g., r#"hello"#)
    RawString { value: Symbol, hash_count: u8 },
    /// Byte string (e.g., b"hello")
    ByteString(Vec<u8>),
    /// Character literal (e.g., 'a')
    Char(char),
    /// Byte literal (e.g., b'a')
    Byte(u8),
    /// Address literal (e.g., 0x742d35cc6634c0532925a3b844bc9e7595f..., prefixed)
    Address(Symbol),
    /// Hash/Bytes32 literal (e.g., 0x... 32 bytes)
    Hash(Symbol),
    /// Percentage literal (e.g., 0.5%, 100%)
    Percentage { value: Symbol },
    /// Duration literal (e.g., 100ms, 5s, 1h)
    Duration { value: u64, unit: DurationUnit },
    /// Size literal (e.g., 1KB, 1MB, 1GB)
    Size { value: u64, unit: SizeUnit },
}

impl fmt::Debug for Literal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Literal::Int { value, suffix, base } => {
                write!(f, "Int({:?}, {:?}, {:?})", value, suffix, base)
            }
            Literal::Float { value, suffix } => write!(f, "Float({}, {:?})", value, suffix),
            Literal::String(s) => write!(f, "String({:?})", s.as_str()),
            Literal::RawString { value, hash_count } => {
                write!(f, "RawString({:?}, {})", value.as_str(), hash_count)
            }
            Literal::ByteString(bytes) => write!(f, "ByteString({:?})", bytes),
            Literal::Char(c) => write!(f, "Char({:?})", c),
            Literal::Byte(b) => write!(f, "Byte({:?})", b),
            Literal::Address(a) => write!(f, "Address({})", a),
            Literal::Hash(h) => write!(f, "Hash({})", h),
            Literal::Percentage { value } => write!(f, "Percentage({}%)", value),
            Literal::Duration { value, unit } => write!(f, "Duration({}{:?})", value, unit),
            Literal::Size { value, unit } => write!(f, "Size({}{:?})", value, unit),
        }
    }
}

impl fmt::Display for Literal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Literal::Int { value, suffix, base } => {
                match base {
                    IntBase::Decimal => write!(f, "{}", value)?,
                    IntBase::Hex => write!(f, "0x{:x}", value)?,
                    IntBase::Octal => write!(f, "0o{:o}", value)?,
                    IntBase::Binary => write!(f, "0b{:b}", value)?,
                }
                if let Some(s) = suffix {
                    write!(f, "{}", s)?;
                }
                Ok(())
            }
            Literal::Float { value, suffix } => {
                write!(f, "{}", value)?;
                if let Some(s) = suffix {
                    write!(f, "{}", s)?;
                }
                Ok(())
            }
            Literal::String(s) => write!(f, "\"{}\"", s),
            Literal::RawString { value, hash_count } => {
                let hashes = "#".repeat(*hash_count as usize);
                write!(f, "r{}\"{}\"{}", hashes, value, hashes)
            }
            Literal::ByteString(bytes) => write!(f, "b\"{:?}\"", bytes),
            Literal::Char(c) => write!(f, "'{}'", c),
            Literal::Byte(b) => write!(f, "b'{}'", *b as char),
            Literal::Address(a) => write!(f, "{}", a),
            Literal::Hash(h) => write!(f, "{}", h),
            Literal::Percentage { value } => write!(f, "{}%", value),
            Literal::Duration { value, unit } => write!(f, "{}{}", value, unit),
            Literal::Size { value, unit } => write!(f, "{}{}", value, unit),
        }
    }
}

/// Integer literal suffix.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub enum IntSuffix {
    U8,
    U16,
    U32,
    U64,
    U128,
    U256,
    Usize,
    I8,
    I16,
    I32,
    I64,
    I128,
    I256,
    Isize,
}

impl fmt::Display for IntSuffix {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IntSuffix::U8 => write!(f, "u8"),
            IntSuffix::U16 => write!(f, "u16"),
            IntSuffix::U32 => write!(f, "u32"),
            IntSuffix::U64 => write!(f, "u64"),
            IntSuffix::U128 => write!(f, "u128"),
            IntSuffix::U256 => write!(f, "U256"),
            IntSuffix::Usize => write!(f, "usize"),
            IntSuffix::I8 => write!(f, "i8"),
            IntSuffix::I16 => write!(f, "i16"),
            IntSuffix::I32 => write!(f, "i32"),
            IntSuffix::I64 => write!(f, "i64"),
            IntSuffix::I128 => write!(f, "i128"),
            IntSuffix::I256 => write!(f, "I256"),
            IntSuffix::Isize => write!(f, "isize"),
        }
    }
}

impl IntSuffix {
    pub fn from_str(s: &str) -> Option<IntSuffix> {
        match s {
            "u8" => Some(IntSuffix::U8),
            "u16" => Some(IntSuffix::U16),
            "u32" => Some(IntSuffix::U32),
            "u64" => Some(IntSuffix::U64),
            "u128" => Some(IntSuffix::U128),
            "U256" => Some(IntSuffix::U256),
            "usize" => Some(IntSuffix::Usize),
            "i8" => Some(IntSuffix::I8),
            "i16" => Some(IntSuffix::I16),
            "i32" => Some(IntSuffix::I32),
            "i64" => Some(IntSuffix::I64),
            "i128" => Some(IntSuffix::I128),
            "I256" => Some(IntSuffix::I256),
            "isize" => Some(IntSuffix::Isize),
            _ => None,
        }
    }
}

/// Integer literal base.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub enum IntBase {
    Decimal,
    Hex,
    Octal,
    Binary,
}

/// Float literal suffix.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub enum FloatSuffix {
    F32,
    F64,
}

impl fmt::Display for FloatSuffix {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FloatSuffix::F32 => write!(f, "f32"),
            FloatSuffix::F64 => write!(f, "f64"),
        }
    }
}

/// Duration unit for duration literals.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub enum DurationUnit {
    Nanoseconds,  // ns
    Microseconds, // us
    Milliseconds, // ms
    Seconds,      // s
    Minutes,      // m
    Hours,        // h
    Days,         // d
}

impl fmt::Display for DurationUnit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DurationUnit::Nanoseconds => write!(f, "ns"),
            DurationUnit::Microseconds => write!(f, "us"),
            DurationUnit::Milliseconds => write!(f, "ms"),
            DurationUnit::Seconds => write!(f, "s"),
            DurationUnit::Minutes => write!(f, "m"),
            DurationUnit::Hours => write!(f, "h"),
            DurationUnit::Days => write!(f, "d"),
        }
    }
}

/// Size unit for size literals.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub enum SizeUnit {
    Bytes,     // B
    Kilobytes, // KB
    Megabytes, // MB
    Gigabytes, // GB
    Terabytes, // TB
}

impl fmt::Display for SizeUnit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SizeUnit::Bytes => write!(f, "B"),
            SizeUnit::Kilobytes => write!(f, "KB"),
            SizeUnit::Megabytes => write!(f, "MB"),
            SizeUnit::Gigabytes => write!(f, "GB"),
            SizeUnit::Terabytes => write!(f, "TB"),
        }
    }
}

// BinOp and UnOp live in `x3-common::token` and are imported above. They are shared across crate boundaries.

// UnOp and display are also implemented in x3-common::token.

/// Delimiters (brackets, braces, parentheses).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub enum Delimiter {
    /// `(`
    OpenParen,
    /// `)`
    CloseParen,
    /// `[`
    OpenBracket,
    /// `]`
    CloseBracket,
    /// `{`
    OpenBrace,
    /// `}`
    CloseBrace,
    /// `<` (when used as delimiter, not comparison)
    OpenAngle,
    /// `>` (when used as delimiter, not comparison)
    CloseAngle,
}

impl fmt::Display for Delimiter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Delimiter::OpenParen => write!(f, "("),
            Delimiter::CloseParen => write!(f, ")"),
            Delimiter::OpenBracket => write!(f, "["),
            Delimiter::CloseBracket => write!(f, "]"),
            Delimiter::OpenBrace => write!(f, "{{"),
            Delimiter::CloseBrace => write!(f, "}}"),
            Delimiter::OpenAngle => write!(f, "<"),
            Delimiter::CloseAngle => write!(f, ">"),
        }
    }
}

impl Delimiter {
    /// Get the matching delimiter.
    pub fn matching(&self) -> Delimiter {
        match self {
            Delimiter::OpenParen => Delimiter::CloseParen,
            Delimiter::CloseParen => Delimiter::OpenParen,
            Delimiter::OpenBracket => Delimiter::CloseBracket,
            Delimiter::CloseBracket => Delimiter::OpenBracket,
            Delimiter::OpenBrace => Delimiter::CloseBrace,
            Delimiter::CloseBrace => Delimiter::OpenBrace,
            Delimiter::OpenAngle => Delimiter::CloseAngle,
            Delimiter::CloseAngle => Delimiter::OpenAngle,
        }
    }

    /// Check if this is an opening delimiter.
    pub fn is_open(&self) -> bool {
        matches!(
            self,
            Delimiter::OpenParen | Delimiter::OpenBracket | Delimiter::OpenBrace | Delimiter::OpenAngle
        )
    }

    /// Check if this is a closing delimiter.
    pub fn is_close(&self) -> bool {
        !self.is_open()
    }
}
