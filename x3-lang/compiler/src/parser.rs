//! Token-based parser for the X3 intent language.
//!
//! Parses X3 source text into the full AST, including:
//!   - agents with context blocks and strategies
//!   - bridge / atomic_swap / strategy / proposal declarations
//!   - capability calls (`capability(args)`)
//!   - cross-chain asset operations and guards
//!   - expressions with operators, if-exprs, closures
//!
//! Architecture: hand-rolled recursive descent with explicit cursor.
//! Tokenization is delegated to the x3-lang-lexer crate; the parser
//! converts lexer `TokenKind` items into its internal `Tok` enum.

use x3_lang_ast::ast::*;
use x3_lang_ast::{
    AmountExpr, AssetDecl, AssetId, AtomicChoiceDecl, AtomicTradeDecl, ChoiceCriterion, ChoicePath, DebtId,
    FallbackReplacement, InvariantKind, TradeEffect, TradeGuarantee, TradeRiskPolicy, TradeStmt, VenueDecl, VenueKind,
};
use x3_lang_common::{BinOp as CBinOp, IntBase, Span, Spanned, Symbol, UnOp as CUnOp, X3Error};
use x3_lang_lexer::token::{Keyword, Token, TokenKind};

/// Map a chain prefix string to its VM family name (as used by the x3-atomic-swap VmType enum).
///
/// Returns `None` for unrecognised prefixes – the caller should fall back to
/// the raw chain name.
pub fn parse_vm_family(prefix: &str) -> Option<&'static str> {
    Some(match prefix {
        // evm family
        "evm" | "eth" | "polygon" | "arb" | "optimism" | "base" | "bsc" | "avax" => "Evm",
        // svm family
        "svm" | "sol" | "solana" => "Svm",
        // substrate family
        "substrate" | "dot" | "ksm" | "polkadot" | "kusama" => "Substrate",
        // bitcoin script
        "btc" | "bitcoin" => "BitcoinScript",
        // x3vm
        "x3" | "x3vm" => "X3Vm",
        // move family
        "move" | "sui" | "aptos" => "MoveVm",
        // cosmwasm family
        "cosmwasm" | "cosmos" | "atom" | "osmo" => "CosmWasm",
        // cairo / starknet
        "cairo" | "starknet" => "CairoVm",
        // cardano / plutus
        "ada" | "cardano" | "plutus" => "PlutusEutxo",
        // ton
        "ton" => "TonTvm",
        // fuel
        "fuel" => "FuelVm",
        // near
        "near" => "NearWasm",
        // stellar / soroban
        "xlm" | "stellar" | "soroban" => "SorobanWasm",
        // ink! / polkadot pvm
        "ink" | "pvm" => "InkWasm",
        // zk
        "zk" | "zkvm" | "risc0" | "sp1" => "ZkVm",
        _ => return None,
    })
}

// ---------------------------------------------------------------------------
// Public entry
// ---------------------------------------------------------------------------

pub fn parse_source(source: &str) -> Result<Program, X3Error> {
    let tokens = tokenize(source);
    let mut p = Parser::new(&tokens);
    let items = p.parse_program()?;
    Ok(Program::new(items))
}

// ===========================================================================
// Tokenizer (inline, no dependency on x3-lexer for now)
// ===========================================================================

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Ident(String),
    Int(u128),
    Float(Symbol),
    String_(String),
    // Punctuation / delimiters
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Comma,
    Colon,
    Semicolon,
    Eq,
    Arrow,    // ->
    FatArrow, // =>
    Dot,
    At,
    // Keywords
    KwFn,
    KwLet,
    KwMut,
    KwReturn,
    KwIf,
    KwElse,
    KwWhile,
    KwFor,
    KwIn,
    KwLoop,
    KwBreak,
    KwContinue,
    KwAgent,
    KwStruct,
    KwEnum,
    KwUse,
    KwMod,
    KwImport,
    KwConst,
    KwBridge,
    KwAtomicSwap,
    KwStrategy,
    KwProposal,
    KwGpu,
    KwSimulate,
    KwScheduled,
    KwIntent,
    KwSubscription,
    KwPub,
    KwAsync,
    KwAs,
    KwTrue,
    KwFalse,
    KwRequire,
    KwOnFail,
    KwOnTimeout,
    KwLock,
    KwMint,
    KwBurn,
    KwRelease,
    KwSwap,
    KwMatch,
    KwAtomic,
    KwEmit,
    KwTry,
    KwAwait,
    // Operators
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    AmpAmp,
    PipePipe,
    EqEq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
    Bang,
    Eof,
}

#[derive(Debug, Clone, PartialEq)]
struct ParserToken {
    kind: Tok,
    span: Span,
}

struct Parser<'a> {
    tokens: &'a [ParserToken],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn new(tokens: &'a [ParserToken]) -> Self {
        Parser { tokens, pos: 0 }
    }

    fn peek(&self) -> Tok {
        self.tokens
            .get(self.pos)
            .map(|token| token.kind.clone())
            .unwrap_or(Tok::Eof)
    }

    fn peek_n(&self, n: usize) -> Tok {
        self.tokens
            .get(self.pos + n)
            .map(|token| token.kind.clone())
            .unwrap_or(Tok::Eof)
    }

    fn advance(&mut self) -> Tok {
        let tok = self.peek();
        if tok != Tok::Eof {
            self.pos += 1;
        }
        tok
    }

    fn expect_ident(&mut self, context: &str) -> Result<String, X3Error> {
        match self.advance() {
            Tok::Ident(name) => Ok(name),
            found => Err(parse_err(format!("{context}: expected identifier"), found)),
        }
    }

    // ------------------------------------------------------------------
    // parse_program
    // ------------------------------------------------------------------

    fn parse_program(&mut self) -> Result<Vec<Spanned<Item>>, X3Error> {
        let mut items = Vec::new();
        loop {
            let item_start = self.pos;
            match self.peek() {
                Tok::Eof => break,
                Tok::At => {
                    // Annotations are collected eagerly and attached to the
                    // next top-level item.
                    let annots = self.parse_annotations()?;
                    let item = self.parse_top_item()?;
                    let with_annots = annotate_item(item, annots);
                    items.push(Spanned::new(with_annots, self.consumed_span(item_start)));
                }
                _ => {
                    let item = self.parse_top_item()?;
                    items.push(Spanned::new(item, self.consumed_span(item_start)));
                }
            }
        }
        Ok(items)
    }

    fn consumed_span(&self, start: usize) -> Span {
        let Some(first) = self.tokens.get(start) else {
            return Span::DUMMY;
        };
        let Some(last) = self.tokens.get(self.pos.saturating_sub(1)) else {
            return Span::DUMMY;
        };
        Span::new(first.span.start, last.span.end, first.span.file_id)
    }

    fn parse_top_item(&mut self) -> Result<Item, X3Error> {
        match self.peek() {
            Tok::KwAsync => self.parse_function_item(),
            Tok::KwFn => self.parse_function_item(),
            Tok::KwAgent => self.parse_agent_item(),
            Tok::KwStruct => self.parse_struct_item(),
            Tok::KwEnum => self.parse_enum_item(),
            Tok::KwUse => self.parse_use_item(),
            Tok::KwMod => self.parse_mod_item(),
            Tok::KwImport => self.parse_import_item(),
            Tok::KwConst => self.parse_const_item(),
            Tok::KwBridge => self.parse_bridge_item(),
            Tok::KwAtomicSwap => self.parse_atomic_swap_item(),
            Tok::KwAtomic => {
                self.advance(); // consume 'atomic'
                if self.check(Tok::KwSwap) {
                    self.parse_atomic_swap_item_new()
                } else if self.check(Tok::Ident("choice".to_string())) {
                    // `atomic choice { ... }` — the spaced spelling of the
                    // same declaration `atomic_choice { ... }` reaches above.
                    self.parse_atomic_choice_body(Symbol::new("atomic_choice"))
                } else if self.check(Tok::Ident("trade".to_string())) {
                    self.parse_atomic_trade_decl().map(Item::AtomicTrade)
                } else {
                    // Parser saw 'atomic' at top level without 'swap' —
                    // not a valid top-level item.
                    Err(parse_err(
                        "expected 'swap', 'trade' or 'choice' after 'atomic' at top level".into(),
                        self.peek(),
                    ))
                }
            }
            Tok::KwStrategy => self.parse_strategy_item(),
            Tok::KwProposal => self.parse_proposal_item(),
            Tok::KwGpu => self.parse_gpu_item(),
            Tok::KwSimulate => self.parse_simulate_item(),
            Tok::KwScheduled => self.parse_scheduled_item(),
            Tok::KwIntent => self.parse_intent_item(),
            Tok::KwSubscription => self.parse_subscription_item(),
            Tok::Ident(ref s) if s == "asset" => self.parse_asset_decl().map(Item::AssetDecl),
            Tok::Ident(ref s) if s == "atomic_choice" => {
                self.advance(); // consume 'atomic_choice'
                self.parse_atomic_choice_body(Symbol::new("atomic_choice"))
            }
            Tok::Ident(ref s) if s == "risk" && matches!(self.peek_n(1), Tok::Ident(ref n) if n == "policy") => {
                self.parse_trade_risk_policy().map(Item::TradeRiskPolicy)
            }
            // B-52 feature lock items
            Tok::Ident(ref s) if s == "solver_market" => self.parse_solver_market_item(),
            Tok::Ident(ref s) if s == "relayers" => self.parse_relayer_swarm_item(),
            Tok::Ident(ref s) if s == "rpc_quorum" => self.parse_rpc_quorum_item(),
            Tok::Ident(ref s) if s == "risk_policy" => self.parse_risk_policy_item(),
            Tok::Ident(ref s) if s == "privacy" => self.parse_privacy_block_item(),
            Tok::Ident(ref s) if s == "invariant" => self.parse_invariant_decl_item(),
            Tok::Ident(ref s) if s == "proofs" => self.parse_proofs_required_item(),
            Tok::Ident(ref s) if s == "vm" => self.parse_vm_decl_item(),
            Tok::Ident(ref s) if s == "target" => self.parse_vm_target_item(),
            Tok::Ident(ref s) if s == "finality_policy" => self.parse_finality_policy_item(),
            Tok::Ident(ref s) if s == "atomic_hedge" => self.parse_atomic_hedge_item(),
            Tok::Ident(ref s) if s == "atomic_liquidation" => self.parse_atomic_liquidation_item(),
            Tok::Ident(ref s) if s == "rebalance" => self.parse_rebalance_item(),
            Tok::Ident(ref s) if s == "netting" => self.parse_netting_item(),
            Tok::Ident(ref s) if s == "arb" => self.parse_arb_item(),
            Tok::Ident(ref s) if s == "hyperarb" => self.parse_hyperarb_item(),
            Tok::Ident(ref s) if s == "venue" => self.parse_venue_decl().map(Item::VenueDecl),
            Tok::Ident(ref s) if s == "parallel" => self.parse_parallel_decl().map(Item::ParallelDecl),
            Tok::Ident(ref s) if s == "objective" => self.parse_objective_decl().map(Item::ObjectiveDecl),
            Tok::Ident(ref s) if s == "error" => self.parse_error_decl_item(),
            _ => Err(parse_err("expected top-level item".into(), self.peek())),
        }
    }

    // ------------------------------------------------------------------
    // Items
    // ------------------------------------------------------------------

    fn parse_function_item(&mut self) -> Result<Item, X3Error> {
        let is_async = if self.peek() == Tok::KwAsync {
            self.advance();
            self.expect(Tok::KwFn, "async function: expected fn")?;
            true
        } else {
            self.advance(); // 'fn'
            false
        };
        let name = self.expect_ident("function name")?;
        let generics = self.parse_optional_generics()?;
        let params = self.parse_param_list()?;
        let ret = self.parse_optional_ret_type()?;
        let body = self.parse_block()?;
        Ok(Item::Function(Function {
            name: Symbol::new(&name),
            id: None,
            params,
            ret,
            generics,
            body,
            visibility: Visibility::Pub,
            is_async,
            annotations: vec![],
        }))
    }

    fn parse_agent_item(&mut self) -> Result<Item, X3Error> {
        self.advance(); // 'agent'
        let name = self.expect_ident("agent name")?;
        let mut context = None;
        let mut state = Vec::new();
        let mut methods: Vec<Spanned<Function>> = Vec::new();
        let mut strategies: Vec<Spanned<StrategyDecl>> = Vec::new();
        let annotations = Vec::new();

        // Optional context block { k: v, ... }
        if self.peek() == Tok::LBrace {
            self.advance();
            let mut entries = Vec::new();
            while self.peek() != Tok::RBrace && self.peek() != Tok::Eof {
                let key = self.expect_ident("context key")?;
                self.expect(Tok::Colon, "':' after context key")?;
                let val = self.parse_primary_expr()?;
                entries.push((Symbol::new(&key), val));
                if self.peek() == Tok::Comma {
                    self.advance();
                }
            }
            self.expect(Tok::RBrace, "expected '}' after context block")?;
            context = Some(ContextBlock { entries });
        }

        // State block { field: type, ... }
        if self.peek() == Tok::LBrace {
            self.advance();
            while self.peek() != Tok::RBrace && self.peek() != Tok::Eof {
                let field_name = self.expect_ident("field name")?;
                self.expect(Tok::Colon, "':' after field")?;
                let ty = self.parse_type()?;
                state.push(StructField {
                    name: Symbol::new(&field_name),
                    ty,
                    visibility: Visibility::Pub,
                });
                if self.peek() == Tok::Comma {
                    self.advance();
                }
            }
            self.expect(Tok::RBrace, "expected '}' after state")?;
        }

        // Main body { methods & strategies }
        if self.peek() == Tok::LBrace {
            self.advance();
            while self.peek() != Tok::RBrace && self.peek() != Tok::Eof {
                let item_annots = self.parse_annotations()?;
                match self.peek() {
                    Tok::KwFn => {
                        let func = self.parse_fn_into_struct()?;
                        methods.push(Spanned::new(func, Span::DUMMY));
                    }
                    Tok::KwStrategy => {
                        let s = self.parse_strategy_decl()?;
                        strategies.push(Spanned::new(s, Span::DUMMY));
                    }
                    Tok::At => {
                        // annotations on the next fn/strategy
                        let inner = self.parse_annotations()?;
                        let mut all_annots = item_annots;
                        all_annots.extend(inner);
                        match self.peek() {
                            Tok::KwFn => {
                                let mut func = self.parse_fn_into_struct()?;
                                func.annotations = all_annots;
                                methods.push(Spanned::new(func, Span::DUMMY));
                            }
                            Tok::KwStrategy => {
                                let s = self.parse_strategy_decl()?;
                                strategies.push(Spanned::new(s, Span::DUMMY));
                            }
                            _ => {
                                return Err(parse_err(
                                    "expected fn or strategy after annotations".into(),
                                    self.peek(),
                                ));
                            }
                        }
                    }
                    _ => {
                        return Err(parse_err("expected fn or strategy in agent body".into(), self.peek()));
                    }
                }
            }
            self.expect(Tok::RBrace, "expected '}' after agent body")?;
        }

        Ok(Item::Agent(Agent {
            name: Symbol::new(&name),
            id: None,
            context,
            state,
            methods,
            strategies,
            visibility: Visibility::Pub,
            annotations,
        }))
    }

    fn parse_struct_item(&mut self) -> Result<Item, X3Error> {
        self.advance(); // 'struct'
        let name = self.expect_ident("struct name")?;
        let generics = self.parse_optional_generics()?;
        self.expect(Tok::LBrace, "expected '{' for struct body")?;
        let mut fields = Vec::new();
        while self.peek() != Tok::RBrace && self.peek() != Tok::Eof {
            let field_name = self.expect_ident("field name")?;
            self.expect(Tok::Colon, "':' after field")?;
            let ty = self.parse_type()?;
            fields.push(StructField {
                name: Symbol::new(&field_name),
                ty,
                visibility: Visibility::Pub,
            });
            if self.peek() == Tok::Comma {
                self.advance();
            }
        }
        self.expect(Tok::RBrace, "expected '}' after struct body")?;
        Ok(Item::Struct(StructDecl {
            name: Symbol::new(&name),
            fields,
            generics,
            visibility: Visibility::Pub,
        }))
    }

    fn parse_enum_item(&mut self) -> Result<Item, X3Error> {
        self.advance();
        let name = self.expect_ident("enum name")?;
        self.expect(Tok::LBrace, "expected '{' for enum body")?;
        let mut variants = Vec::new();
        while self.peek() != Tok::RBrace && self.peek() != Tok::Eof {
            let vname = self.expect_ident("variant name")?;
            let payload = if self.peek() == Tok::LParen {
                self.advance();
                let ty = self.parse_type()?;
                self.expect(Tok::RParen, "expected ')' after variant payload")?;
                Some(ty)
            } else {
                None
            };
            variants.push(EnumVariant {
                name: Symbol::new(&vname),
                payload,
            });
            if self.peek() == Tok::Comma {
                self.advance();
            }
        }
        self.expect(Tok::RBrace, "expected '}'")?;
        Ok(Item::Enum(EnumDecl {
            name: Symbol::new(&name),
            variants,
        }))
    }

    fn parse_use_item(&mut self) -> Result<Item, X3Error> {
        self.advance();
        let mut path = Vec::new();
        loop {
            path.push(Symbol::new(&self.expect_ident("use path segment")?));
            if self.peek() == Tok::Colon {
                // :: separator — we treat two colons as a separator
                self.advance();
                if self.peek() == Tok::Colon {
                    self.advance();
                }
                continue;
            }
            break;
        }
        let alias = if self.peek() == Tok::KwAs {
            self.advance();
            Some(Symbol::new(&self.expect_ident("alias")?))
        } else {
            None
        };
        self.expect(Tok::Semicolon, "expected ';' after use")?;
        Ok(Item::Use(UseDecl { path, alias }))
    }

    fn parse_mod_item(&mut self) -> Result<Item, X3Error> {
        self.advance();
        let name = self.expect_ident("module name")?;
        self.expect(Tok::LBrace, "expected '{'")?;
        let mut parser = Parser::new(self.tokens);
        parser.pos = self.pos;
        let items = parser.parse_program()?;
        self.pos = parser.pos;
        self.expect(Tok::RBrace, "expected '}'")?;
        Ok(Item::Mod(ModDecl {
            name: Symbol::new(&name),
            items,
        }))
    }

    fn parse_import_item(&mut self) -> Result<Item, X3Error> {
        self.advance();
        let mut module = Vec::new();
        loop {
            module.push(Symbol::new(&self.expect_ident("import path")?));
            if self.peek() == Tok::Colon {
                self.advance();
                if self.peek() == Tok::Colon {
                    self.advance();
                }
                continue;
            }
            break;
        }
        let as_alias = if self.peek() == Tok::KwAs {
            self.advance();
            Some(Symbol::new(&self.expect_ident("alias")?))
        } else {
            None
        };
        self.expect(Tok::Semicolon, "expected ';'")?;
        Ok(Item::Import(ImportDecl { module, as_alias }))
    }

    fn parse_const_item(&mut self) -> Result<Item, X3Error> {
        self.advance();
        let name = self.expect_ident("const name")?;
        let ty = if self.peek() == Tok::Colon {
            self.advance();
            Some(self.parse_type()?)
        } else {
            None
        };
        self.expect(Tok::Eq, "expected '=' in const declaration")?;
        let value = self.parse_expr()?;
        self.expect(Tok::Semicolon, "expected ';' after const")?;
        Ok(Item::Const(ConstDecl {
            name: Symbol::new(&name),
            ty,
            value,
        }))
    }

    fn parse_bridge_item(&mut self) -> Result<Item, X3Error> {
        self.advance();
        let name = self.expect_ident("bridge name")?;
        let from_asset = self.parse_asset_ref()?;
        self.expect_ident("to")?; // skip 'to'
        let to_asset = self.parse_asset_ref()?;
        self.expect(Tok::LBrace, "expected '{'")?;
        let mut body = Vec::new();
        let mut requires = Vec::new();
        let mut on_fail = None;
        let mut timeout = None;
        while self.peek() != Tok::RBrace && self.peek() != Tok::Eof {
            match self.peek() {
                Tok::KwRequire => {
                    requires.push(self.parse_require_guard()?);
                }
                Tok::KwOnFail => {
                    self.advance();
                    let action = self.parse_failure_action()?;
                    on_fail = Some(self.merge_failure_action(on_fail.take(), action)?);
                }
                Tok::KwOnTimeout => {
                    self.advance();
                    let dur = self.parse_expr()?;
                    let action = self.parse_failure_action()?;
                    timeout = Some(dur);
                    on_fail = Some(self.merge_failure_action(on_fail.take(), action)?);
                }
                _ => {
                    body.push(self.parse_statement()?);
                }
            }
        }
        self.expect(Tok::RBrace, "expected '}'")?;
        Ok(Item::Bridge(BridgeDecl {
            name: Symbol::new(&name),
            from_asset,
            to_asset,
            body,
            requires,
            on_fail,
            timeout,
        }))
    }

    fn parse_atomic_swap_item(&mut self) -> Result<Item, X3Error> {
        self.advance();
        let name = self.expect_ident("atomic_swap name")?;
        self.expect(Tok::LBrace, "expected '{'")?;
        let mut body = Vec::new();
        let mut on_fail = None;
        let mut timeout_source = None;
        let mut timeout_destination = None;
        while self.peek() != Tok::RBrace && self.peek() != Tok::Eof {
            match self.peek() {
                Tok::KwOnFail => {
                    self.advance();
                    let action = self.parse_failure_action()?;
                    on_fail = Some(self.merge_failure_action(on_fail.take(), action)?);
                }
                Tok::KwOnTimeout => {
                    self.advance();
                    let duration = self.parse_duration_expr("on_timeout")?;
                    let action = self.parse_failure_action()?;
                    // Store timeout on destination by default for backward compat
                    timeout_destination = Some(duration);
                    on_fail = Some(self.merge_failure_action(on_fail.take(), action)?);
                }
                Tok::Ident(ref s) if s == "min_output" => {
                    return Err(parse_err(
                        "`min_output` is not a clause of `atomic_swap`; the declaration has no \
                         price floor. Write the floor on a route `swap` instead (TICKET-122)"
                            .into(),
                        self.peek(),
                    ));
                }
                _ => body.push(self.parse_statement()?),
            }
        }
        self.expect(Tok::RBrace, "expected '}'")?;
        Ok(Item::AtomicSwap(AtomicSwapDecl {
            name: Symbol::new(&name),
            from_asset: AssetRef::new(ChainRef(Symbol::new("unknown")), Symbol::new("unknown")),
            to_asset: AssetRef::new(ChainRef(Symbol::new("unknown")), Symbol::new("unknown")),
            source_vm: None,
            dest_vm: None,
            amount: None,
            receiver: None,
            hashlock: None,
            body,
            requires: vec![],
            on_fail,
            timeout_source,
            timeout_destination,
        }))
    }

    /// Parse the new `atomic swap <from> -> <to> { ... }` syntax.
    /// The caller has already consumed `atomic` and `swap` tokens.
    fn parse_atomic_swap_item_new(&mut self) -> Result<Item, X3Error> {
        let from_asset = self.parse_asset_ref()?;
        self.expect(Tok::Arrow, "expected '->' after source asset")?;
        let to_asset = self.parse_asset_ref()?;
        self.expect(Tok::LBrace, "expected '{'")?;

        // Extract VM families from chain prefixes.
        let source_vm = parse_vm_family(from_asset.chain.as_str()).map(String::from);
        let dest_vm = parse_vm_family(to_asset.chain.as_str()).map(String::from);

        let mut body = Vec::new();
        let mut amount = None;
        let mut receiver = None;
        let mut hashlock = None;
        let mut requires = Vec::new();
        let mut on_fail = None;
        let mut timeout_source = None;
        let mut timeout_destination = None;

        while self.peek() != Tok::RBrace && self.peek() != Tok::Eof {
            match self.peek() {
                Tok::Ident(ref s) if s == "amount" => {
                    self.advance();
                    amount = Some(self.parse_expr()?);
                }
                Tok::Ident(ref s) if s == "receiver" => {
                    self.advance();
                    receiver = Some(self.parse_expr()?);
                }
                Tok::Ident(ref s) if s == "hashlock" => {
                    self.advance();
                    // hashlock <hash_fn>(<secret_expr>)
                    let hash_fn_name = self.expect_ident("hash function name")?;
                    self.expect(Tok::LParen, "expected '(' after hash function")?;
                    let secret = self.parse_expr()?;
                    self.expect(Tok::RParen, "expected ')' after hashlock secret")?;
                    hashlock = Some(HashlockSpec {
                        hash_fn: Symbol::new(&hash_fn_name),
                        secret: Box::new(secret),
                    });
                }
                Tok::Ident(ref s) if s == "timeout" => {
                    self.advance();
                    // timeout source <expr>  |  timeout destination <expr>
                    let kind = self.expect_ident("timeout kind (source/destination)")?;
                    let duration = self.parse_duration_expr("timeout")?;
                    match kind.as_str() {
                        "source" => timeout_source = Some(duration),
                        "destination" => timeout_destination = Some(duration),
                        other => {
                            return Err(parse_err(
                                format!("expected 'source' or 'destination' after 'timeout', got '{other}'"),
                                self.peek(),
                            ));
                        }
                    }
                }
                Tok::KwRequire => {
                    requires.push(self.parse_require_guard()?);
                }
                Tok::KwOnFail => {
                    self.advance();
                    on_fail = Some(self.parse_failure_action()?);
                }
                Tok::Ident(ref s) if s == "min_output" => {
                    return Err(parse_err(
                        "`min_output` is not a clause of `atomic swap`; the declaration has no \
                         price floor. Write the floor on a route `swap` instead (TICKET-122)"
                            .into(),
                        self.peek(),
                    ));
                }
                _ => {
                    body.push(self.parse_statement()?);
                }
            }
        }
        self.expect(Tok::RBrace, "expected '}'")?;

        // Auto-generate a name from the source/destination chains.
        let name = Symbol::from_string(format!(
            "atomic_swap_{}_{}",
            from_asset.chain.as_str(),
            to_asset.chain.as_str()
        ));

        Ok(Item::AtomicSwap(AtomicSwapDecl {
            name,
            from_asset,
            to_asset,
            source_vm,
            dest_vm,
            amount,
            receiver,
            hashlock,
            body,
            requires,
            on_fail,
            timeout_source,
            timeout_destination,
        }))
    }

    // ------------------------------------------------------------------
    // Trading Core v1: asset declarations, risk policies, atomic trades
    // ------------------------------------------------------------------

    /// `asset NAME = VM_FAMILY.CHAIN.CANONICAL_ID { decimals: N }`
    fn parse_asset_decl(&mut self) -> Result<AssetDecl, X3Error> {
        self.advance(); // 'asset'
        let name = self.expect_ident("asset declaration name")?;
        self.expect(Tok::Eq, "expected '=' after asset declaration name")?;

        let vm_family = self.expect_ident("asset vm family")?;
        self.expect(Tok::Dot, "expected '.' after asset vm family")?;
        let chain = self.expect_ident("asset chain")?;
        self.expect(Tok::Dot, "expected '.' after asset chain")?;
        let canonical_id = self.expect_ident("asset canonical id")?;
        self.expect(Tok::LBrace, "expected '{' to open the asset declaration body")?;

        let mut decimals = None;
        while self.peek() != Tok::RBrace && self.peek() != Tok::Eof {
            match self.peek() {
                Tok::Ident(ref s) if s == "decimals" => {
                    self.advance();
                    self.expect(Tok::Colon, "expected ':' after decimals")?;
                    let value = self.parse_u8_literal("asset decimals")?;
                    if decimals.replace(value).is_some() {
                        return Err(parse_err(
                            "duplicate 'decimals' field in asset declaration".into(),
                            self.peek(),
                        ));
                    }
                }
                _ => {
                    return Err(parse_err(
                        "expected 'decimals' field in asset declaration".into(),
                        self.peek(),
                    ));
                }
            }
        }
        self.expect(Tok::RBrace, "expected '}' to close the asset declaration")?;
        let decimals =
            decimals.ok_or_else(|| parse_err("asset declaration is missing the 'decimals' field".into(), Tok::Eof))?;

        Ok(AssetDecl {
            name: Symbol::new(&name),
            asset: AssetId {
                vm_family: Symbol::new(&vm_family),
                chain: ChainRef(Symbol::new(&chain)),
                canonical_id: Symbol::new(&canonical_id),
                symbol: Symbol::new(&name),
                decimals,
            },
        })
    }

    /// `risk policy NAME { ... }` — Trading Core v1 policy (not the legacy
    /// single-token `risk_policy { ... }` B-52 declaration).
    fn parse_trade_risk_policy(&mut self) -> Result<TradeRiskPolicy, X3Error> {
        self.advance(); // 'risk'
        self.advance(); // 'policy'
        let name = self.expect_ident("risk policy name")?;
        self.expect(Tok::LBrace, "expected '{' after risk policy name")?;

        let mut max_slippage_bps = None;
        let mut max_gas = None;
        let mut max_flash_fee_bps = None;
        let mut deadline = None;
        let mut require_private_submission = None;
        let mut min_profit = None;
        let mut max_oracle_deviation_bps = None;
        let mut max_cumulative_loss = None;
        let mut quote_freshness = None;
        let mut max_price_impact_bps = None;
        let mut max_mev_leakage_bps = None;

        while self.peek() != Tok::RBrace && self.peek() != Tok::Eof {
            match self.peek() {
                Tok::Ident(ref s) if s == "max_slippage" => {
                    self.advance();
                    self.expect(Tok::Colon, "expected ':' after max_slippage")?;
                    let value = self.parse_bps_value("max_slippage")?;
                    if max_slippage_bps.replace(value).is_some() {
                        return Err(parse_err("duplicate 'max_slippage' in risk policy".into(), self.peek()));
                    }
                }
                Tok::Ident(ref s) if s == "max_gas" => {
                    self.advance();
                    self.expect(Tok::Colon, "expected ':' after max_gas")?;
                    let value = self.parse_amount_expr("max_gas")?;
                    if max_gas.replace(value).is_some() {
                        return Err(parse_err("duplicate 'max_gas' in risk policy".into(), self.peek()));
                    }
                }
                Tok::Ident(ref s) if s == "max_flash_fee" => {
                    self.advance();
                    self.expect(Tok::Colon, "expected ':' after max_flash_fee")?;
                    let value = self.parse_bps_value("max_flash_fee")?;
                    if max_flash_fee_bps.replace(value).is_some() {
                        return Err(parse_err(
                            "duplicate 'max_flash_fee' in risk policy".into(),
                            self.peek(),
                        ));
                    }
                }
                Tok::Ident(ref s) if s == "deadline" => {
                    self.advance();
                    self.expect(Tok::Colon, "expected ':' after deadline")?;
                    let value = self.parse_deadline_expr()?;
                    if deadline.replace(value).is_some() {
                        return Err(parse_err("duplicate 'deadline' in risk policy".into(), self.peek()));
                    }
                }
                Tok::Ident(ref s) if s == "require_private_submission" => {
                    self.advance();
                    self.expect(Tok::Colon, "expected ':' after require_private_submission")?;
                    let value = self.parse_bool_literal("require_private_submission")?;
                    if require_private_submission.replace(value).is_some() {
                        return Err(parse_err(
                            "duplicate 'require_private_submission' in risk policy".into(),
                            self.peek(),
                        ));
                    }
                }
                Tok::Ident(ref s) if s == "min_profit" => {
                    self.advance();
                    self.expect(Tok::Colon, "expected ':' after min_profit")?;
                    let value = self.parse_amount_expr("min_profit")?;
                    if min_profit.replace(value).is_some() {
                        return Err(parse_err("duplicate 'min_profit' in risk policy".into(), self.peek()));
                    }
                }
                Tok::Ident(ref s) if s == "max_oracle_deviation" => {
                    self.advance();
                    self.expect(Tok::Colon, "expected ':' after max_oracle_deviation")?;
                    let value = self.parse_bps_value("max_oracle_deviation")?;
                    if max_oracle_deviation_bps.replace(value).is_some() {
                        return Err(parse_err(
                            "duplicate 'max_oracle_deviation' in risk policy".into(),
                            self.peek(),
                        ));
                    }
                }
                Tok::Ident(ref s) if s == "max_cumulative_loss" => {
                    self.advance();
                    self.expect(Tok::Colon, "expected ':' after max_cumulative_loss")?;
                    let value = self.parse_amount_expr("max_cumulative_loss")?;
                    if max_cumulative_loss.replace(value).is_some() {
                        return Err(parse_err(
                            "duplicate 'max_cumulative_loss' in risk policy".into(),
                            self.peek(),
                        ));
                    }
                }
                Tok::Ident(ref s) if s == "quote_freshness" => {
                    self.advance();
                    self.expect(Tok::Colon, "expected ':' after quote_freshness")?;
                    let value = self.parse_block_count("quote_freshness")?;
                    if value == 0 {
                        return Err(parse_err(
                            "quote_freshness must be greater than zero blocks; omit the field instead of \
                             declaring a zero ceiling"
                                .into(),
                            self.peek(),
                        ));
                    }
                    if quote_freshness.replace(value).is_some() {
                        return Err(parse_err(
                            "duplicate 'quote_freshness' in risk policy".into(),
                            self.peek(),
                        ));
                    }
                }
                Tok::Ident(ref s) if s == "max_price_impact" => {
                    self.advance();
                    self.expect(Tok::Colon, "expected ':' after max_price_impact")?;
                    let value = self.parse_bps_value("max_price_impact")?;
                    if max_price_impact_bps.replace(value).is_some() {
                        return Err(parse_err(
                            "duplicate 'max_price_impact' in risk policy".into(),
                            self.peek(),
                        ));
                    }
                }
                Tok::Ident(ref s) if s == "max_mev_leakage" => {
                    self.advance();
                    self.expect(Tok::Colon, "expected ':' after max_mev_leakage")?;
                    let value = self.parse_bps_value("max_mev_leakage")?;
                    if max_mev_leakage_bps.replace(value).is_some() {
                        return Err(parse_err(
                            "duplicate 'max_mev_leakage' in risk policy".into(),
                            self.peek(),
                        ));
                    }
                }
                _ => {
                    return Err(parse_err("unknown field in trading risk policy".into(), self.peek()));
                }
            }
        }
        self.expect(Tok::RBrace, "expected '}' to close the risk policy")?;

        Ok(TradeRiskPolicy {
            name: Symbol::new(&name),
            max_slippage_bps: max_slippage_bps
                .ok_or_else(|| parse_err("risk policy missing 'max_slippage'".into(), Tok::RBrace))?,
            max_gas: max_gas.ok_or_else(|| parse_err("risk policy missing 'max_gas'".into(), Tok::RBrace))?,
            max_flash_fee_bps: max_flash_fee_bps
                .ok_or_else(|| parse_err("risk policy missing 'max_flash_fee'".into(), Tok::RBrace))?,
            deadline: deadline.ok_or_else(|| parse_err("risk policy missing 'deadline'".into(), Tok::RBrace))?,
            require_private_submission: require_private_submission
                .ok_or_else(|| parse_err("risk policy missing 'require_private_submission'".into(), Tok::RBrace))?,
            min_profit,
            max_oracle_deviation_bps,
            max_cumulative_loss,
            quote_freshness,
            max_price_impact_bps,
            max_mev_leakage_bps,
        })
    }

    /// `atomic trade NAME using POLICY { statements }` — caller has consumed
    /// both `atomic` and `trade` (the latter through `check`).
    fn parse_atomic_trade_decl(&mut self) -> Result<AtomicTradeDecl, X3Error> {
        let name = self.expect_ident("atomic trade name")?;
        self.expect(Tok::Ident("using".into()), "expected 'using' after trade name")?;
        let risk_policy = self.expect_ident("atomic trade risk policy")?;

        // Optional `effects [...]` / `guarantees [...]` clauses. Both use a
        // closed vocabulary, so an unrecognized name is a parse error rather
        // than a label the compiler silently accepts and never checks.
        let mut effects: Vec<TradeEffect> = Vec::new();
        let mut guarantees: Vec<TradeGuarantee> = Vec::new();
        loop {
            match self.peek() {
                Tok::Ident(ref s) if s == "effects" => {
                    self.advance();
                    for name in self.parse_bracketed_ident_list("effects")? {
                        let effect = TradeEffect::from_name(&name).ok_or_else(|| {
                            parse_err(
                                format!(
                                    "unknown trade effect '{name}'; known effects are: {}",
                                    known_names(TradeEffect::ALL.iter().map(|effect| effect.as_str()))
                                ),
                                self.peek(),
                            )
                        })?;
                        if effects.contains(&effect) {
                            return Err(parse_err(
                                format!("duplicate trade effect '{}'", effect.as_str()),
                                self.peek(),
                            ));
                        }
                        effects.push(effect);
                    }
                }
                Tok::Ident(ref s) if s == "guarantees" => {
                    self.advance();
                    for name in self.parse_bracketed_ident_list("guarantees")? {
                        let guarantee = TradeGuarantee::from_name(&name).ok_or_else(|| {
                            parse_err(
                                format!(
                                    "unknown trade guarantee '{name}'; known guarantees are: {}",
                                    known_names(TradeGuarantee::ALL.iter().map(|g| g.as_str()))
                                ),
                                self.peek(),
                            )
                        })?;
                        if guarantees.contains(&guarantee) {
                            return Err(parse_err(
                                format!("duplicate trade guarantee '{}'", guarantee.as_str()),
                                self.peek(),
                            ));
                        }
                        guarantees.push(guarantee);
                    }
                }
                _ => break,
            }
        }
        self.expect(Tok::LBrace, "expected '{' to open the atomic trade body")?;

        let mut body = Vec::new();
        let mut seen_debts = Vec::new();
        while self.peek() != Tok::RBrace && self.peek() != Tok::Eof {
            let stmt = self.parse_trade_stmt()?;
            if let TradeStmt::Borrow { debt, .. } = &stmt {
                if seen_debts.contains(&debt.0) {
                    return Err(parse_err(
                        format!(
                            "duplicate debt '{}' in atomic trade; each debt must be declared once",
                            debt.0.as_str()
                        ),
                        self.peek(),
                    ));
                }
                seen_debts.push(debt.0.clone());
            }
            body.push(stmt);
        }
        self.expect(Tok::RBrace, "expected '}' to close the atomic trade")?;

        Ok(AtomicTradeDecl {
            name: Symbol::new(&name),
            risk_policy: Symbol::new(&risk_policy),
            effects,
            guarantees,
            body,
        })
    }

    fn parse_trade_stmt(&mut self) -> Result<TradeStmt, X3Error> {
        match self.peek() {
            Tok::Ident(ref s) if s == "borrow" => {
                self.advance();
                let amount = self.parse_amount_expr("borrow amount")?;
                self.expect(
                    Tok::Ident("from".into()),
                    "expected 'from <provider>' after borrow amount",
                )?;
                let provider = self.expect_ident("borrow provider")?;
                self.expect(Tok::Ident("as".into()), "expected 'as <debt>' after provider")?;
                let debt = self.expect_ident("borrow debt name")?;
                Ok(TradeStmt::Borrow {
                    amount,
                    provider: Symbol::new(&provider),
                    debt: DebtId(Symbol::new(&debt)),
                })
            }
            Tok::KwLet => {
                self.advance();
                let binding = self.expect_ident("swap binding")?;
                self.expect(Tok::Eq, "expected '=' after swap binding")?;
                self.expect(Tok::KwSwap, "expected 'swap' after '='")?;
                let input = self.parse_amount_expr("swap input")?;
                self.expect(Tok::Arrow, "expected '->' after swap input")?;
                let to_asset = self.expect_ident("swap destination asset")?;
                self.expect(
                    Tok::Ident("via".into()),
                    "expected 'via <venue>' after destination asset",
                )?;
                let venue = self.expect_ident("swap venue")?;
                self.expect(Tok::Ident("min_out".into()), "expected 'min_out <amount>' after venue")?;
                let min_output = self.parse_amount_expr("min_out amount")?;
                let from_asset = input.asset.clone();
                Ok(TradeStmt::Swap {
                    binding: Symbol::new(&binding),
                    input,
                    from_asset,
                    to_asset: Symbol::new(&to_asset),
                    venue: Symbol::new(&venue),
                    min_output,
                })
            }
            Tok::Ident(ref s) if s == "repay" => {
                self.advance();
                let debt = self.expect_ident("debt to repay")?;
                Ok(TradeStmt::Repay {
                    debt: DebtId(Symbol::new(&debt)),
                })
            }
            Tok::KwBridge => self.parse_bridge_trade_stmt(),
            Tok::KwRequire => {
                self.advance();
                match self.peek() {
                    Tok::Ident(ref s) if s == "net_profit" => {
                        self.advance();
                        self.expect(Tok::Ge, "expected '>=' after net_profit requirement")?;
                        let amount = self.parse_amount_expr("net_profit amount")?;
                        Ok(TradeStmt::RequireMinNetProfit { amount })
                    }
                    Tok::Ident(ref s) if s == "all_debts_repaid" => {
                        self.advance();
                        Ok(TradeStmt::RequireAllDebtsRepaid)
                    }
                    _ => Err(parse_err(
                        "expected 'net_profit >= <amount>' or 'all_debts_repaid' after require".into(),
                        self.peek(),
                    )),
                }
            }
            Tok::Ident(ref s) if s == "invariant" => {
                self.advance();
                let kind = self.parse_invariant_kind()?;
                Ok(TradeStmt::AssertInvariant { kind })
            }
            Tok::KwEmit => {
                self.advance();
                self.expect(Tok::Ident("receipt".into()), "expected 'receipt' after emit")?;
                Ok(TradeStmt::EmitReceipt)
            }
            _ => Err(parse_err(
                "expected a trading statement (borrow, swap, repay, bridge, require, invariant, or emit receipt)"
                    .into(),
                self.peek(),
            )),
        }
    }

    /// `bridge <amount_expr> -> <asset> via <bridge> to <receiver>` — caller
    /// has consumed neither `Tok::KwBridge` nor the bare-identifier form;
    /// both dispatch here (see the general-VM `bridge { ... }` item parser
    /// for why `bridge` can tokenize as either).
    fn parse_bridge_trade_stmt(&mut self) -> Result<TradeStmt, X3Error> {
        self.advance();
        let input = self.parse_amount_expr("bridge amount")?;
        self.expect(Tok::Arrow, "expected '->' after bridge amount")?;
        let to_asset = self.expect_ident("bridge destination asset")?;
        self.expect(
            Tok::Ident("via".into()),
            "expected 'via <bridge>' after destination asset",
        )?;
        let via = self.expect_ident("bridge name")?;
        self.expect(Tok::Ident("to".into()), "expected 'to <receiver>' after bridge name")?;
        let receiver = self.parse_expr()?;
        let from_asset = input.asset.clone();
        Ok(TradeStmt::Bridge {
            input,
            from_asset,
            to_asset: Symbol::new(&to_asset),
            via: Symbol::new(&via),
            receiver,
        })
    }

    /// Parse the name after `invariant`. Deliberately closed: an
    /// unrecognized name is a parse error rather than a silently-accepted
    /// no-op, matching the plan's "unknown values ... fail closed" rule.
    fn parse_invariant_kind(&mut self) -> Result<InvariantKind, X3Error> {
        match self.peek() {
            Tok::Ident(ref s) if s == "solvent" => {
                self.advance();
                Ok(InvariantKind::Solvent)
            }
            _ => Err(parse_err(
                "expected a known invariant name ('solvent') after 'invariant'".into(),
                self.peek(),
            )),
        }
    }

    /// Parse `<expression> <ASSET>` into a typed amount. The asset identifier
    /// terminates the expression, so `debt.amount USDC` stays a field access.
    fn parse_amount_expr(&mut self, field: &str) -> Result<AmountExpr, X3Error> {
        let value = self.parse_expr()?;
        let asset = Symbol::new(&self.expect_ident(&format!("{field} asset"))?);
        Ok(AmountExpr { value, asset })
    }

    fn parse_bps_value(&mut self, field: &str) -> Result<u16, X3Error> {
        let found = self.advance();
        let value = match found {
            Tok::Int(value) => value,
            other => {
                return Err(parse_err(
                    format!("{field}: basis points must be an unsigned integer"),
                    other,
                ));
            }
        };
        if value > u16::MAX as u128 {
            return Err(parse_err(
                format!("{field}: basis points exceed u16::MAX"),
                Tok::Int(value),
            ));
        }
        self.expect(
            Tok::Ident("bps".into()),
            &format!("{field}: expected 'bps' after the integer value"),
        )?;
        Ok(value as u16)
    }

    fn parse_bool_literal(&mut self, field: &str) -> Result<bool, X3Error> {
        match self.advance() {
            Tok::KwTrue => Ok(true),
            Tok::KwFalse => Ok(false),
            other => Err(parse_err(format!("{field}: expected true or false"), other)),
        }
    }

    fn parse_u8_literal(&mut self, field: &str) -> Result<u8, X3Error> {
        match self.advance() {
            Tok::Int(value) if value <= u8::MAX as u128 => Ok(value as u8),
            Tok::Int(_) => Err(parse_err(format!("{field}: value exceeds u8"), self.peek())),
            other => Err(parse_err(format!("{field}: expected an unsigned integer"), other)),
        }
    }

    /// A count of blocks. Used for ceilings where the unit is blocks and a
    /// fractional value would be meaningless.
    /// `[<name>] { path <name> { ... } ... choose <criterion> }`
    ///
    /// The caller has consumed `atomic_choice` / `atomic choice`. The paths are
    /// all parsed, however many there are — bounding them is a semantic
    /// decision, and a parser that silently stopped after N paths would hide
    /// the program's real shape from the verifier.
    fn parse_atomic_choice_body(&mut self, name: Symbol) -> Result<Item, X3Error> {
        // Both `atomic_choice { ... }` and `atomic_choice <name> { ... }`. The
        // name is optional because the declaration is anonymous in the spec's
        // form, but it is what error messages need to point at, so allowing one
        // is worth the two lines.
        let name = if let Tok::Ident(candidate) = self.peek() {
            self.advance();
            Symbol::new(&candidate)
        } else {
            name
        };
        self.expect(Tok::LBrace, "expected '{' after atomic_choice")?;
        let mut paths: Vec<ChoicePath> = Vec::new();
        let mut criterion: Option<ChoiceCriterion> = None;
        while self.peek() != Tok::RBrace && self.peek() != Tok::Eof {
            match self.peek() {
                Tok::Ident(ref s) if s == "path" => {
                    self.advance();
                    let path_name = self.expect_ident("path name")?;
                    paths.push(self.parse_choice_path(Symbol::new(&path_name))?);
                }
                Tok::Ident(ref s) if s == "choose" => {
                    self.advance();
                    let wanted = self.expect_ident("choice criterion")?;
                    criterion = Some(ChoiceCriterion::parse(&wanted).ok_or_else(|| {
                        let allowed: Vec<&str> = ChoiceCriterion::ALL.iter().map(|c| c.as_str()).collect();
                        parse_err(
                            format!(
                                "unknown choice criterion '{wanted}'; the compiler can only choose by a \
                                 criterion it can evaluate over every path, so the set is closed: {}",
                                allowed.join(", ")
                            ),
                            self.peek(),
                        )
                    })?);
                    self.opt_semi();
                }
                other => {
                    return Err(parse_err(
                        "expected `path <name> { ... }` or `choose <criterion>` inside atomic_choice".into(),
                        other,
                    ))
                }
            }
        }
        self.expect(Tok::RBrace, "expected '}' to close atomic_choice")?;
        let criterion = criterion.ok_or_else(|| {
            parse_err(
                "atomic_choice has no `choose` clause; a branch set with no criterion is not a choice".into(),
                self.peek(),
            )
        })?;
        Ok(Item::AtomicChoice(AtomicChoiceDecl { name, paths, criterion }))
    }

    /// `{ <statements> }`, or `{ <chain.ASSET> -> <chain.ASSET> -> ... }`.
    fn parse_choice_path(&mut self, name: Symbol) -> Result<ChoicePath, X3Error> {
        self.expect(Tok::LBrace, "expected '{' after the path name")?;
        let mut body: Vec<Statement> = Vec::new();
        let mut hops: Vec<AssetRef> = Vec::new();
        let mut net_output: Option<AmountExpr> = None;
        while self.peek() != Tok::RBrace && self.peek() != Tok::Eof {
            match self.peek() {
                Tok::Ident(ref s) if s == "net_output" => {
                    self.advance();
                    let value = self.parse_expr()?;
                    let asset = self.expect_ident("asset for net_output")?;
                    net_output = Some(AmountExpr {
                        value,
                        asset: Symbol::new(&asset),
                    });
                    self.opt_semi();
                }
                // A hop chain is `chain.ASSET` followed by `->`, which no
                // statement begins with: statements start with a keyword or
                // with `emit`/`route`/`require`, none of which is dotted.
                Tok::Ident(_) if self.peek_n(1) == Tok::Dot && self.peek_n(3) == Tok::Arrow => {
                    if !body.is_empty() {
                        return Err(parse_err(
                            "a path is either a hop chain or a block of statements, not both".into(),
                            self.peek(),
                        ));
                    }
                    hops = self.parse_hop_chain()?;
                }
                // A path's real content is a route — the swaps and bridges the
                // branch would take — so the route-step grammar is the one that
                // applies here. It is used *without* the `Statement::Atomic`
                // wrapper that `route { ... }` adds: each path is already
                // lowered inside its own `AtomicBegin`/`AtomicEnd`, and
                // wrapping again would be a nested atomic scope, which the IR
                // verifier rejects.
                Tok::KwSwap | Tok::KwBridge | Tok::KwLock | Tok::KwMint | Tok::KwBurn | Tok::KwRelease => {
                    body.push(self.parse_route_step()?);
                }
                _ => body.push(self.parse_statement()?),
            }
        }
        self.expect(Tok::RBrace, "expected '}' to close the path")?;
        Ok(ChoicePath {
            name,
            body,
            hops,
            net_output,
        })
    }

    /// `<chain.ASSET> -> <chain.ASSET> [-> ...]`
    fn parse_hop_chain(&mut self) -> Result<Vec<AssetRef>, X3Error> {
        let mut hops = vec![self.parse_asset_ref()?];
        while self.peek() == Tok::Arrow {
            self.advance();
            hops.push(self.parse_asset_ref()?);
        }
        self.opt_semi();
        Ok(hops)
    }

    fn parse_block_count(&mut self, field: &str) -> Result<u64, X3Error> {
        match self.advance() {
            Tok::Int(value) if value <= u64::MAX as u128 => Ok(value as u64),
            Tok::Int(_) => Err(parse_err(format!("{field}: value exceeds u64"), self.peek())),
            other => Err(parse_err(format!("{field}: expected an unsigned integer"), other)),
        }
    }

    /// `[a, b, c]` — a comma-separated identifier list in square brackets.
    fn parse_bracketed_ident_list(&mut self, clause: &str) -> Result<Vec<String>, X3Error> {
        self.expect(Tok::LBracket, &format!("expected '[' after '{clause}'"))?;
        let mut names = Vec::new();
        while self.peek() != Tok::RBracket && self.peek() != Tok::Eof {
            names.push(self.parse_clause_name(clause)?);
            if self.peek() == Tok::Comma {
                self.advance();
            }
        }
        self.expect(Tok::RBracket, &format!("expected ']' to close the '{clause}' list"))?;
        if names.is_empty() {
            return Err(parse_err(
                format!(
                    "'{clause}' must list at least one name; omit the clause rather than declaring an \
                     empty one"
                ),
                self.peek(),
            ));
        }
        Ok(names)
    }

    /// Accept a name that may also be a language keyword.
    ///
    /// `swap` and `bridge` are keywords, so `effects [borrow, swap, repay]` has
    /// to accept them. Rejecting a keyword here would make the vocabulary
    /// unreachable for two of the four effects — the two that matter most.
    fn parse_clause_name(&mut self, clause: &str) -> Result<String, X3Error> {
        let found = self.advance();
        match found {
            Tok::Ident(name) => Ok(name),
            Tok::KwSwap => Ok("swap".to_string()),
            Tok::KwBridge => Ok("bridge".to_string()),
            other => Err(parse_err(format!("{clause}: expected a name, found {other:?}"), other)),
        }
    }

    /// A trading deadline: `deadline: 2 blocks`, `deadline: 30s`, `deadline: 2h`.
    ///
    /// Trading Core v1's canonical syntax is `N blocks` and that is still what a
    /// bare number means. It used to *reject* any other unit ("'seconds' is not
    /// supported") while the timeout clauses accepted `180s` — one language, two
    /// answers about what a duration is, and a deadline in seconds was the
    /// program that could not be written. It goes through the same parser and the
    /// same conversion as everything else now, so `deadline: 2h` and
    /// `timeout 2h` are the same 1200 blocks (TICKET-048).
    fn parse_deadline_expr(&mut self) -> Result<Expression, X3Error> {
        self.parse_duration_expr("deadline")
    }

    fn parse_strategy_item(&mut self) -> Result<Item, X3Error> {
        self.advance();
        let name = self.expect_ident("strategy name")?;
        if self.peek() != Tok::LBrace {
            return Err(parse_err(
                "expected '{' after the strategy name; a strategy module declares input, output, \
                 effects, guarantees, permissions, domains, risk, bounds and execute"
                    .into(),
                self.peek(),
            ));
        }
        self.advance(); // consume '{'
        let mut inputs: Vec<StrategyInput> = Vec::new();
        let mut outputs: Vec<AssetRef> = Vec::new();
        let mut effects: Vec<x3_lang_ast::trading::TradeEffect> = Vec::new();
        let mut guarantees: Vec<x3_lang_ast::trading::TradeGuarantee> = Vec::new();
        let mut permissions: Vec<StrategyPermission> = Vec::new();
        let mut domains: Vec<Symbol> = Vec::new();
        let mut risk: Option<StrategyRisk> = None;
        let mut license: Option<StrategyLicense> = None;
        let mut submission: Option<SubmissionPolicy> = None;
        let mut resources: Option<StrategyResources> = None;
        let mut split: Option<ProfitSplit> = None;
        let mut max_steps: Option<Expression> = None;
        let mut max_gas: Option<Expression> = None;
        let mut body: Vec<Statement> = Vec::new();

        while self.peek() != Tok::RBrace && self.peek() != Tok::Eof {
            let section = self.expect_ident("strategy section")?;
            match section.as_str() {
                "input" => {
                    let asset = self.parse_asset_ref()?;
                    let mut amount = None;
                    let mut max_amount = None;
                    loop {
                        let Tok::Ident(ref keyword) = self.peek() else {
                            break;
                        };
                        match keyword.as_str() {
                            "amount" => {
                                self.advance();
                                amount = Some(self.parse_expr()?);
                            }
                            "max" => {
                                self.advance();
                                max_amount = Some(self.parse_expr()?);
                            }
                            _ => break,
                        }
                    }
                    self.opt_semi();
                    inputs.push(StrategyInput {
                        asset,
                        amount,
                        max_amount,
                    });
                }
                "output" => {
                    outputs.push(self.parse_asset_ref()?);
                    self.opt_semi();
                }
                "effects" => {
                    for effect in self.parse_bracketed_ident_list("effects")? {
                        effects.push(TradeEffect::from_name(&effect).ok_or_else(|| {
                            parse_err(
                                format!(
                                    "unknown effect '{effect}' in a strategy module; known: {}",
                                    known_names(TradeEffect::ALL.iter().map(|effect| effect.as_str()))
                                ),
                                self.peek(),
                            )
                        })?);
                    }
                    self.opt_semi();
                }
                "guarantees" => {
                    for guarantee in self.parse_bracketed_ident_list("guarantees")? {
                        guarantees.push(TradeGuarantee::from_name(&guarantee).ok_or_else(|| {
                            parse_err(
                                format!(
                                    "unknown guarantee '{guarantee}' in a strategy module; known: {}",
                                    known_names(TradeGuarantee::ALL.iter().map(|g| g.as_str()))
                                ),
                                self.peek(),
                            )
                        })?);
                    }
                    self.opt_semi();
                }
                "permissions" => {
                    for permission in self.parse_bracketed_ident_list("permissions")? {
                        permissions.push(StrategyPermission::parse(&permission).ok_or_else(|| {
                            let allowed: Vec<&str> = StrategyPermission::ALL.iter().map(|p| p.as_str()).collect();
                            parse_err(
                                format!(
                                    "unknown permission '{permission}'; the set is closed: {}",
                                    allowed.join(", ")
                                ),
                                self.peek(),
                            )
                        })?);
                    }
                    self.opt_semi();
                }
                "domains" => {
                    for domain in self.parse_bracketed_ident_list("domains")? {
                        domains.push(Symbol::new(&domain));
                    }
                    self.opt_semi();
                }
                "risk" => {
                    risk = Some(self.parse_strategy_risk()?);
                }
                "bounds" => {
                    self.expect(Tok::LBrace, "expected '{' after bounds")?;
                    while self.peek() != Tok::RBrace && self.peek() != Tok::Eof {
                        let key = self.expect_ident("bounds field")?;
                        match key.as_str() {
                            "max_steps" => max_steps = Some(self.parse_expr()?),
                            "max_gas" => max_gas = Some(self.parse_expr()?),
                            other => {
                                return Err(parse_err(
                                    format!("unknown bounds field '{other}'; expected max_steps or max_gas"),
                                    self.peek(),
                                ))
                            }
                        }
                        self.opt_semi();
                    }
                    self.expect(Tok::RBrace, "expected '}' to close bounds")?;
                }
                "resources" => {
                    // PHASE 41's own spelling for the caps `bounds` states two of. Parsed as the
                    // phase writes it (`max_compute = …;`) with the `=` optional, because the
                    // implementation's `bounds` block writes the same shape without one.
                    resources = Some(self.parse_strategy_resources()?);
                    self.opt_semi();
                }
                "submission" => {
                    self.expect(Tok::LBrace, "expected '{' after submission")?;
                    let mut private: Option<PrivateSubmissionMode> = None;
                    while self.peek() != Tok::RBrace && self.peek() != Tok::Eof {
                        let key = self.expect_ident("submission field")?;
                        if key != "private" {
                            return Err(parse_err(
                                format!("unknown submission field '{key}'; expected private"),
                                self.peek(),
                            ));
                        }
                        self.expect(Tok::Eq, "expected '=' after `private`")?;
                        let mode = self.expect_ident("submission privacy mode")?;
                        private = Some(PrivateSubmissionMode::parse(&mode).ok_or_else(|| {
                            let allowed: Vec<&str> =
                                PrivateSubmissionMode::ALL.iter().map(|mode| mode.as_str()).collect();
                            parse_err(
                                format!(
                                    "unknown submission mode '{mode}'; the set is closed: {}",
                                    allowed.join(", ")
                                ),
                                self.peek(),
                            )
                        })?);
                        self.opt_semi();
                    }
                    self.expect(Tok::RBrace, "expected '}' to close submission")?;
                    submission = Some(SubmissionPolicy {
                        private: private
                            .ok_or_else(|| parse_err("submission is missing `private = <mode>`".into(), self.peek()))?,
                    });
                }
                "license" => {
                    license = Some(self.parse_strategy_license()?);
                }
                "split" => {
                    // `split profit { ... }` — the noun is required so a future
                    // `split` of something else cannot be read as a profit split.
                    let noun = self.expect_ident("split target")?;
                    if noun != "profit" {
                        return Err(parse_err(
                            format!(
                                "unknown split target '{noun}'; the only split the language defines is `split profit`"
                            ),
                            self.peek(),
                        ));
                    }
                    split = Some(self.parse_profit_split()?);
                }
                "execute" => {
                    self.expect(Tok::LBrace, "expected '{' after execute")?;
                    while self.peek() != Tok::RBrace && self.peek() != Tok::Eof {
                        match self.peek() {
                            Tok::KwSwap | Tok::KwBridge | Tok::KwLock | Tok::KwMint | Tok::KwBurn | Tok::KwRelease => {
                                body.push(self.parse_route_step()?)
                            }
                            Tok::Ident(ref s)
                                if matches!(s.as_str(), "swap" | "bridge" | "lock" | "mint" | "burn" | "release") =>
                            {
                                body.push(self.parse_route_step()?)
                            }
                            Tok::Ident(ref s) if s == "timeout" => body.push(self.parse_intent_timeout()?),
                            // A module's body can opt into a mode, and the
                            // permission it must declare for doing so is part of
                            // PHASE 23's list. Without this arm the line parses
                            // as an expression statement and the opt-in vanishes
                            // — the same way a leg's timeout did.
                            Tok::Ident(ref s) if s == "allow" => {
                                self.advance();
                                let feature = self.expect_ident("allowed feature")?;
                                self.opt_semi();
                                body.push(Statement::Allow {
                                    feature: Symbol::new(&feature),
                                });
                            }
                            _ => body.push(self.parse_statement()?),
                        }
                    }
                    self.expect(Tok::RBrace, "expected '}' to close execute")?;
                }
                other => {
                    return Err(parse_err(
                        format!(
                            "unknown strategy section '{other}'; expected input, output, effects, \
                             guarantees, permissions, domains, risk, bounds or execute"
                        ),
                        self.peek(),
                    ))
                }
            }
        }
        self.expect(Tok::RBrace, "expected '}' to close the strategy")?;
        Ok(Item::Strategy(CrossChainStrategy {
            name: Symbol::new(&name),
            max_steps,
            max_gas,
            body,
            requires: Vec::new(),
            on_fail: None,
            inputs,
            outputs,
            effects,
            guarantees,
            permissions,
            domains,
            risk,
            license,
            split,
            submission,
            resources,
        }))
    }

    /// `license { creator <who> profit_share <N>% [executions <N>] [expires_block <N>] }`
    ///
    /// `resources { max_compute = N; … }` — PHASE 41's block, with the `=` the phase writes optional
    /// so the same five names read the same way as `bounds` does. Every name is checked against the
    /// vocabulary rather than stored as written: a cap nothing measures is the defect TICKET-116's
    /// neighbour, one construct over.
    fn parse_strategy_resources(&mut self) -> Result<StrategyResources, X3Error> {
        self.expect(Tok::LBrace, "expected '{' after resources")?;
        let mut resources = StrategyResources::default();
        while self.peek() != Tok::RBrace && self.peek() != Tok::Eof {
            let key = self.expect_ident("resources field")?;
            if self.peek() == Tok::Eq {
                self.advance();
            }
            let value = self.parse_expr()?;
            let slot = match key.as_str() {
                "max_compute" => &mut resources.max_compute,
                "max_memory" => &mut resources.max_memory,
                "max_network_calls" => &mut resources.max_network_calls,
                "max_routes" => &mut resources.max_routes,
                "max_branches" => &mut resources.max_branches,
                other => {
                    return Err(parse_err(
                        format!(
                            "unknown resources field '{other}'; PHASE 41's caps are max_compute, \
                             max_memory, max_network_calls, max_routes and max_branches"
                        ),
                        self.peek(),
                    ))
                }
            };
            if slot.is_some() {
                return Err(parse_err(
                    format!("the resources block declares '{key}' twice"),
                    self.peek(),
                ));
            }
            *slot = Some(value);
            self.opt_semi();
        }
        self.expect(Tok::RBrace, "expected '}' to close resources")?;
        Ok(resources)
    }

    ///
    /// `creator` and `profit_share` are required: a licence that does not say who
    /// holds it, or what it earns them, is a heading rather than a licence.
    fn parse_strategy_license(&mut self) -> Result<StrategyLicense, X3Error> {
        self.expect(Tok::LBrace, "expected '{' after license")?;
        let mut creator: Option<Symbol> = None;
        let mut profit_share_bps: Option<u32> = None;
        let mut executions: Option<u128> = None;
        let mut expires_block: Option<u64> = None;
        while self.peek() != Tok::RBrace && self.peek() != Tok::Eof {
            let key = self.expect_ident("license field")?;
            match key.as_str() {
                "creator" => {
                    // A handle, written either bare or quoted: an identity is
                    // opaque to the compiler, so it does not get to insist on a
                    // spelling the outside world must match.
                    let who = match self.advance() {
                        Tok::Ident(name) => name,
                        Tok::String_(name) => name,
                        other => {
                            return Err(parse_err(
                                "license creator must be a name or a string".into(),
                                other,
                            ))
                        }
                    };
                    creator = Some(Symbol::new(&who));
                }
                "profit_share" => profit_share_bps = Some(self.parse_whole_percent_bps("profit_share")?),
                "executions" => {
                    let value = self.parse_expr()?;
                    executions = Some(expr_to_u128(&value).map_err(|_| {
                        parse_err("license executions must be an integer".into(), self.peek())
                    })?);
                }
                "expires_block" => {
                    let value = self.parse_expr()?;
                    let block = expr_to_u128(&value)
                        .map_err(|_| parse_err("license expires_block must be an integer".into(), self.peek()))?;
                    if block > u64::MAX as u128 {
                        return Err(parse_err(format!("license expires_block {block} exceeds u64"), self.peek()));
                    }
                    expires_block = Some(block as u64);
                }
                other => {
                    return Err(parse_err(
                        format!(
                            "unknown license field '{other}'; expected creator, profit_share,                              executions or expires_block"
                        ),
                        self.peek(),
                    ))
                }
            }
            self.opt_semi();
        }
        self.expect(Tok::RBrace, "expected '}' to close license")?;
        Ok(StrategyLicense {
            creator: creator.ok_or_else(|| parse_err("license is missing creator".into(), self.peek()))?,
            profit_share_bps: profit_share_bps
                .ok_or_else(|| parse_err("license is missing profit_share".into(), self.peek()))?,
            executions,
            expires_block,
        })
    }

    /// `split profit { <N>% -> <recipient> ... }`
    fn parse_profit_split(&mut self) -> Result<ProfitSplit, X3Error> {
        self.expect(Tok::LBrace, "expected '{' after `split profit`")?;
        let mut shares: Vec<(SplitRecipient, u32)> = Vec::new();
        while self.peek() != Tok::RBrace && self.peek() != Tok::Eof {
            let share = self.parse_whole_percent_bps("profit split share")?;
            self.expect(Tok::Arrow, "expected '->' after a profit share")?;
            let recipient = self.expect_ident("split recipient")?;
            let recipient = SplitRecipient::parse(&recipient).ok_or_else(|| {
                let allowed: Vec<&str> = SplitRecipient::ALL.iter().map(|r| r.as_str()).collect();
                parse_err(
                    format!(
                        "unknown split recipient '{recipient}'; the set is closed: {}",
                        allowed.join(", ")
                    ),
                    self.peek(),
                )
            })?;
            self.opt_semi();
            shares.push((recipient, share));
        }
        self.expect(Tok::RBrace, "expected '}' to close the profit split")?;
        Ok(ProfitSplit { shares })
    }

    /// A share written as a whole percentage, returned in basis points.
    ///
    /// Basis points internally, because a share is money and PHASE 42 forbids
    /// floating-point ambiguity in anything that moves money. The surface takes
    /// whole percentages, which is the granularity it can express without a
    /// fractional literal; finer shares would need one.
    fn parse_whole_percent_bps(&mut self, field: &str) -> Result<u32, X3Error> {
        // A whole percentage and a fractional one reach here as different
        // tokens: `70%` is an integer followed by `%`, while `0.5%` is a single
        // percentage literal whose text still carries the sign. Both are the
        // same construct to an author, so both are handled here rather than
        // making the spelling depend on whether the number has a point.
        let percent: u32 = if let Tok::Int(value) = self.peek() {
            if self.peek_n(1) == Tok::Percent {
                self.advance(); // the integer
                self.advance(); // the '%'
                if value > u32::MAX as u128 {
                    return Err(parse_err(format!("{field} {value}% is out of range"), self.peek()));
                }
                value as u32
            } else {
                return Err(parse_err(
                    format!("{field} must be a percentage, written as a number followed by '%'"),
                    self.peek(),
                ));
            }
        } else {
            let expr = self.parse_expr()?;
            let Expression::Literal(LiteralExpr::Percentage { value }) = &expr else {
                return Err(parse_err(
                    format!("{field} must be a percentage, written as a number followed by '%'"),
                    self.peek(),
                ));
            };
            let text = value.as_str().trim_end_matches('%');
            let whole: u32 = text.parse().map_err(|_| {
                parse_err(
                    format!(
                        "{field} '{text}%' is not a whole number of percent; a share finer than one percent is not expressible yet"
                    ),
                    self.peek(),
                )
            })?;
            whole
        };
        percent
            .checked_mul(100)
            .ok_or_else(|| parse_err(format!("{field} {percent}% exceeds the basis-point range"), self.peek()))
    }

    /// `risk { max_slippage_bps <n> max_total_fee_bps <n> }` — a module's
    /// declared risk profile. Both fields are required: a module that bounds one
    /// cost and not the other has not declared a profile, it has declared half
    /// of one.
    fn parse_strategy_risk(&mut self) -> Result<StrategyRisk, X3Error> {
        self.expect(Tok::LBrace, "expected '{' after risk")?;
        let mut max_slippage_bps = None;
        let mut max_total_fee_bps = None;
        while self.peek() != Tok::RBrace && self.peek() != Tok::Eof {
            let key = self.expect_ident("risk field")?;
            match key.as_str() {
                "max_slippage_bps" => max_slippage_bps = Some(self.parse_venue_u32("max_slippage_bps")?),
                "max_total_fee_bps" => max_total_fee_bps = Some(self.parse_venue_u32("max_total_fee_bps")?),
                other => {
                    return Err(parse_err(
                        format!("unknown risk field '{other}'; expected max_slippage_bps or max_total_fee_bps"),
                        self.peek(),
                    ))
                }
            }
            self.opt_semi();
        }
        self.expect(Tok::RBrace, "expected '}' to close risk")?;
        Ok(StrategyRisk {
            max_slippage_bps: max_slippage_bps
                .ok_or_else(|| parse_err("strategy risk is missing max_slippage_bps".into(), self.peek()))?,
            max_total_fee_bps: max_total_fee_bps
                .ok_or_else(|| parse_err("strategy risk is missing max_total_fee_bps".into(), self.peek()))?,
        })
    }

    fn parse_proposal_item(&mut self) -> Result<Item, X3Error> {
        self.advance();
        let name = self.expect_ident("proposal name")?;
        let title = if self.peek() == Tok::Colon {
            self.advance();
            Some(self.parse_expr()?)
        } else {
            None
        };
        self.expect(Tok::LBrace, "expected '{'")?;
        let mut body = Vec::new();
        let mut requires = Vec::new();
        while self.peek() != Tok::RBrace && self.peek() != Tok::Eof {
            if self.peek() == Tok::KwRequire {
                requires.push(self.parse_require_guard()?);
            } else {
                body.push(self.parse_statement()?);
            }
        }
        self.expect(Tok::RBrace, "expected '}'")?;
        Ok(Item::Proposal(ProposalDecl {
            name: Symbol::new(&name),
            title,
            body,
            requires,
        }))
    }

    fn parse_gpu_item(&mut self) -> Result<Item, X3Error> {
        self.advance();
        let is_simd = if matches!(self.peek(), Tok::Ident(ref s) if s == "simd") {
            self.advance();
            true
        } else {
            false
        };
        let body = self.parse_block()?;
        Ok(Item::GpuBlock(GpuBlock { body, is_simd }))
    }

    fn parse_simulate_item(&mut self) -> Result<Item, X3Error> {
        self.advance();
        let name = self.expect_ident("simulate name")?;
        let body = self.parse_block()?;
        let receipt = if matches!(self.peek(), Tok::Ident(ref s) if s == "receipt") {
            self.advance();
            self.expect(Tok::Colon, "expected ':'")?;
            Some(Symbol::new(&self.expect_ident("receipt name")?))
        } else {
            None
        };
        Ok(Item::SimulateDecl(SimulateDecl {
            name: Symbol::new(&name),
            body,
            receipt,
        }))
    }

    fn parse_scheduled_item(&mut self) -> Result<Item, X3Error> {
        self.advance();
        let name = self.expect_ident("scheduled task name")?;
        self.expect(Tok::Colon, "expected ':'")?;
        let period = self.parse_expr()?;
        let period_blocks = expr_to_u64(&period);
        let body = self.parse_block()?;
        Ok(Item::ScheduledTask(ScheduledTask {
            name: Symbol::new(&name),
            period_blocks,
            body,
        }))
    }

    fn parse_intent_item(&mut self) -> Result<Item, X3Error> {
        self.advance();
        let name = self.expect_ident("intent name")?;
        // Production shape: `intent <name> { from ... to ... route { ... }
        // require ... timeout ... on_fail ... }`. The legacy shape
        // (`intent <name> [constraints] { stmt* }`) is preserved.
        if self.peek() == Tok::LBrace {
            return self.parse_intent_body(name);
        }
        let mut constraints = Vec::new();
        if self.peek() == Tok::LBracket {
            self.advance();
            while self.peek() != Tok::RBracket && self.peek() != Tok::Eof {
                constraints.push(self.parse_expr()?);
                if self.peek() == Tok::Comma {
                    self.advance();
                }
            }
            self.expect(Tok::RBracket, "expected ']'")?;
        }
        let body = self.parse_block()?;
        Ok(Item::IntentDecl(IntentDecl {
            name: Symbol::new(&name),
            constraints,
            body,
        }))
    }

    /// Parse the body of a production-shape intent: from/to endpoints,
    /// route operations, require guards, and timeout/on_fail policies.
    fn parse_intent_body(&mut self, name: String) -> Result<Item, X3Error> {
        self.expect(Tok::LBrace, "expected '{' after intent name")?;
        let mut stmts: Vec<Statement> = Vec::new();
        while self.peek() != Tok::RBrace && self.peek() != Tok::Eof {
            stmts.push(self.parse_intent_clause()?);
        }
        self.expect(Tok::RBrace, "expected '}' to close intent body")?;
        fill_route_step_amounts(&mut stmts);
        Ok(Item::IntentDecl(IntentDecl {
            name: Symbol::new(&name),
            constraints: Vec::new(),
            body: Block::new(stmts),
        }))
    }

    /// One line of a production intent body.
    fn parse_intent_clause(&mut self) -> Result<Statement, X3Error> {
        match self.peek() {
            Tok::Ident(ref s) if s == "from" => self.parse_intent_endpoint(true),
            Tok::Ident(ref s) if s == "to" => self.parse_intent_endpoint(false),
            Tok::Ident(ref s) if s == "route" => self.parse_intent_route(),
            Tok::KwRequire => {
                let g = self.parse_require_guard()?;
                self.opt_semi();
                Ok(Statement::Require(g))
            }
            Tok::Ident(ref s) if s == "timeout" => self.parse_intent_timeout(),
            Tok::Ident(ref s) if s == "allow" => {
                self.advance();
                let feature = self.expect_ident("allowed feature")?;
                self.opt_semi();
                Ok(Statement::Allow {
                    feature: Symbol::new(&feature),
                })
            }
            Tok::KwOnFail => self.parse_intent_onfail(),
            // `use` is a **keyword** to the lexer (`Keyword::Use`, the top-level import), so this
            // arm has to match the keyword token. It matched `Tok::Ident(ref s) if s == "use"` —
            // the shape every other clause uses — which no program could reach, because a lexer
            // keyword never arrives as an identifier. `use uniswap 1` inside an intent body was
            // refused while the arm below this one advertised `use` in the list of clauses it
            // accepts, and the formatter wrote the clause back out (TICKET-046).
            Tok::KwUse => self.parse_intent_use(),
            Tok::Ident(ref s) if s == "on" => self.parse_intent_on_event(),
            Tok::Ident(ref s) if s == "proofs" => Err(parse_err(
                "`proofs required { ... }` must be declared at file scope, not inside an intent body: a \
                 nested declaration parsed successfully and was then never applied to the intent"
                    .into(),
                self.peek().clone(),
            )),
            other => Err(parse_err(
                format!(
                    "unexpected clause in intent body: {other:?}; expected one of `from`, `to`, `route`, \
                     `require`, `timeout`, `on_fail`, `use` or `on`"
                ),
                other.clone(),
            )),
        }
    }

    /// `from <chain.ASSET> [amount <expr>] [receiver <expr>]` — lowers to a
    /// `Statement::Lock` on the source asset.
    /// `to   <chain.ASSET> [receiver <expr>]` — lowers to a
    /// `Statement::Release` on the destination asset.
    fn parse_intent_endpoint(&mut self, is_from: bool) -> Result<Statement, X3Error> {
        self.advance(); // consume `from` / `to`
        let asset_ref = self.parse_asset_ref()?;
        let chain = asset_ref.chain.clone();
        let asset = asset_ref.name.clone();

        let mut amount_expr: Option<Expression> = None;
        let mut receiver_expr: Option<Expression> = None;
        loop {
            match self.peek() {
                Tok::Ident(ref s) if s == "amount" => {
                    self.advance();
                    amount_expr = Some(self.parse_expr()?);
                }
                Tok::Ident(ref s) if s == "receiver" => {
                    self.advance();
                    receiver_expr = Some(self.parse_expr()?);
                }
                _ => break,
            }
        }
        self.opt_semi();

        let zero = || {
            Expression::Literal(LiteralExpr::Int {
                value: 0,
                base: IntBase::Decimal,
                suffix: None,
            })
        };
        let sender = || Expression::Literal(LiteralExpr::String(Symbol::new("sender")));

        if is_from {
            Ok(Statement::Lock {
                chain: chain.clone(),
                asset: AssetRef::new(chain, asset),
                amount: amount_expr.unwrap_or_else(zero),
                from: receiver_expr.unwrap_or_else(sender),
            })
        } else {
            Ok(Statement::Release {
                chain: chain.clone(),
                asset: AssetRef::new(chain, asset),
                to: receiver_expr.unwrap_or_else(sender),
            })
        }
    }

    /// `route { swap <dex> <chain.ASSET> -> <chain.ASSET> [amount N] [min_output N] ;
    ///         bridge <via> <chain.ASSET> -> <chain.ASSET> [receiver <addr>] ;
    ///         lock|mint|burn|release <chain.ASSET> [amount N] [from|to <expr>] }`
    ///
    /// Wraps the route operations in an `Atomic` block so cross-VM
    /// safety guarantees are preserved end-to-end.
    fn parse_intent_route(&mut self) -> Result<Statement, X3Error> {
        self.advance(); // consume `route`
        self.expect(Tok::LBrace, "expected '{' after route")?;
        let mut stmts: Vec<Statement> = Vec::new();
        while self.peek() != Tok::RBrace && self.peek() != Tok::Eof {
            stmts.push(self.parse_route_step()?);
        }
        self.expect(Tok::RBrace, "expected '}' to close route")?;
        Ok(Statement::Atomic(AtomicBlock {
            meta: None,
            body: Block::new(stmts),
        }))
    }

    /// One route step. The body is dispatched on the leading keyword.
    /// One route step. The body is dispatched on the leading keyword.
    fn parse_route_step(&mut self) -> Result<Statement, X3Error> {
        // Peek the leading keyword; each sub-parser is responsible for
        // consuming it.
        match self.peek() {
            Tok::KwSwap => {
                self.advance();
                self.parse_swap_step()
            }
            Tok::KwBridge => {
                self.advance();
                self.parse_bridge_step()
            }
            Tok::KwLock | Tok::KwMint | Tok::KwBurn | Tok::KwRelease => {
                let kw = match self.peek() {
                    Tok::KwLock => "lock",
                    Tok::KwMint => "mint",
                    Tok::KwBurn => "burn",
                    _ => "release",
                };
                self.advance();
                self.parse_lmbr_step(kw)
            }
            // `fallback` is the one route operation that is **not** a lexer keyword, so it is the
            // only one that arrives as an identifier. The dispatcher does not advance for it: the
            // sub-parser checks for its own leading word and consumes it, which avoids a
            // rewind/peek dance.
            //
            // `swap`, `bridge` and the four `lock`/`mint`/`burn`/`release` steps used to have arms
            // here too, matching an identifier for a word the lexer sends as a keyword token — arms
            // no program could reach, with the `Tok::Kw*` arms above doing the work
            // (`no_arm_matches_an_identifier_for_a_lexer_keyword` in
            // `compiler/tests/test_keyword_clauses.rs`). The pattern is worth remembering: it is
            // how three clauses were refused while the comments beside them said they were read.
            Tok::Ident(ref s) if s == "fallback" => self.parse_route_fallback(),
            _ => Err(parse_err(
                "expected route operation (swap/bridge/lock/mint/burn/release/fallback)".into(),
                self.peek(),
            )),
        }
    }

    /// Body for `lock` / `mint` / `burn` / `release` route steps. The
    /// leading keyword has been consumed by the dispatcher.
    fn parse_lmbr_step(&mut self, kw: &str) -> Result<Statement, X3Error> {
        let asset = self.parse_asset_ref()?;
        let chain = asset.chain.clone();
        let mut amount: Option<Expression> = None;
        let mut from_or_to: Option<Expression> = None;
        loop {
            match self.peek() {
                Tok::Ident(ref s) if s == "amount" => {
                    self.advance();
                    amount = Some(self.parse_expr()?);
                }
                Tok::Ident(ref s) if s == "from" || s == "to" => {
                    self.advance();
                    from_or_to = Some(self.parse_expr()?);
                }
                _ => break,
            }
        }
        self.opt_semi();
        let amount_expr = amount.unwrap_or_else(|| {
            Expression::Literal(LiteralExpr::Int {
                value: 0,
                base: IntBase::Decimal,
                suffix: None,
            })
        });
        let target = from_or_to.unwrap_or_else(|| Expression::Literal(LiteralExpr::String(Symbol::new("sender"))));
        match kw {
            "lock" => Ok(Statement::Lock {
                chain: chain.clone(),
                asset: AssetRef::new(chain, asset.name.clone()),
                amount: amount_expr,
                from: target,
            }),
            "mint" => Ok(Statement::Mint {
                asset: AssetRef::new(chain, asset.name.clone()),
                amount: amount_expr,
                to: target,
            }),
            "burn" => Ok(Statement::Burn {
                asset: AssetRef::new(chain, asset.name.clone()),
                amount: amount_expr,
                from: target,
            }),
            _ => Ok(Statement::Release {
                chain: chain.clone(),
                asset: AssetRef::new(chain, asset.name.clone()),
                to: target,
            }),
        }
    }

    /// The word the current token spells, whether the lexer ranked it a keyword
    /// or an identifier.
    ///
    /// Constraint and metric names are ordinary words, and some of them —
    /// `slippage`, `finality`, `risk`, `atomic` — are keywords elsewhere in the
    /// language, so reading them as identifiers would refuse a legal objective.
    fn peek_word(&self) -> Option<String> {
        match self.peek() {
            Tok::Ident(word) => Some(word),
            // `atomic` is lexed as a keyword and has a token of its own; the
            // other words a constraint or metric can use that are also keywords
            // elsewhere (`risk`, `slippage`, `finality`, `profit`) fall back to
            // `Tok::Ident` and are read above.
            Tok::KwAtomic => Some("atomic".to_string()),
            _ => None,
        }
    }

    /// `objective [<name>] { <maximize|minimize> <metric>; constraints { … } }`
    ///
    /// The metric is parsed from the spec's whole list rather than only from the
    /// ones the optimizer can rank, so that `maximize profit` reaches the
    /// verifier and is refused with a reason instead of failing to parse.
    fn parse_objective_decl(&mut self) -> Result<ObjectiveDecl, X3Error> {
        self.advance(); // consume `objective`
        let name = if let Tok::Ident(candidate) = self.peek() {
            if candidate.as_str() == "maximize" || candidate.as_str() == "minimize" {
                crate::objective::ANONYMOUS_OBJECTIVE_NAME
            } else {
                self.advance();
                return self.parse_objective_body(candidate);
            }
        } else {
            crate::objective::ANONYMOUS_OBJECTIVE_NAME
        };
        self.parse_objective_body(name.to_string())
    }

    fn parse_objective_body(&mut self, name: String) -> Result<ObjectiveDecl, X3Error> {
        self.expect(Tok::LBrace, "expected '{' after objective")?;
        let mut metric: Option<ObjectiveMetric> = None;
        let mut constraints = ObjectiveConstraints::default();
        // One clause per constraint. A repeated one would silently replace the
        // first, leaving a ceiling the program wrote on the page in force
        // nowhere.
        let mut seen: Vec<String> = Vec::new();
        while self.peek() != Tok::RBrace && self.peek() != Tok::Eof {
            let section = self.expect_ident("objective section")?;
            match section.as_str() {
                "maximize" | "minimize" => {
                    if metric.is_some() {
                        return Err(parse_err(
                            "objective declares its metric twice; one objective ranks one thing".into(),
                            self.peek(),
                        ));
                    }
                    let Some(wanted) = self.peek_word() else {
                        return Err(parse_err(
                            "expected a metric name after the direction".into(),
                            self.peek(),
                        ));
                    };
                    self.advance();
                    let found = ObjectiveMetric::by_name(&wanted).ok_or_else(|| {
                        let allowed: Vec<String> = ObjectiveMetric::ALL
                            .iter()
                            .map(|m| format!("{} {}", m.direction(), m.name()))
                            .collect();
                        parse_err(
                            format!(
                                "unknown objective metric '{wanted}'; the set is the spec's: {}",
                                allowed.join(", ")
                            ),
                            self.peek(),
                        )
                    })?;
                    // `maximize fees` is not an unknown word, it is the wrong
                    // direction for a known one, and the message says so.
                    if found.direction() != section.as_str() {
                        return Err(parse_err(
                            format!(
                                "'{wanted}' is a metric to {}, not to {section}; the direction is \
                                 part of the metric",
                                found.direction()
                            ),
                            self.peek(),
                        ));
                    }
                    metric = Some(found);
                    self.opt_semi();
                }
                "constraints" => {
                    self.expect(Tok::LBrace, "expected '{' after constraints")?;
                    while self.peek() != Tok::RBrace && self.peek() != Tok::Eof {
                        self.parse_objective_constraint(&mut constraints, &mut seen)?;
                    }
                    self.expect(Tok::RBrace, "expected '}' to close constraints")?;
                }
                other => {
                    return Err(parse_err(
                        format!("unknown objective section '{other}'; expected maximize, minimize or constraints"),
                        self.peek(),
                    ))
                }
            }
        }
        self.expect(Tok::RBrace, "expected '}' to close the objective")?;
        Ok(ObjectiveDecl {
            name: Symbol::new(&name),
            metric: metric.ok_or_else(|| {
                parse_err(
                    "objective states no metric; an objective that does not say what to rank is a heading".into(),
                    self.peek(),
                )
            })?,
            constraints,
        })
    }

    /// One `name <= value` line inside `constraints`, or one of the two flags.
    fn parse_objective_constraint(
        &mut self,
        constraints: &mut ObjectiveConstraints,
        seen: &mut Vec<String>,
    ) -> Result<(), X3Error> {
        let key = self.peek_word().ok_or_else(|| {
            parse_err(
                "expected a constraint name; the set is capital, hops, chains, risk, execution_time, \
                 fees, slippage, finality, private and atomic"
                    .into(),
                self.peek(),
            )
        })?;
        self.advance();
        if seen.contains(&key) {
            return Err(parse_err(
                format!(
                    "objective declares a ceiling on '{key}' twice; the second would replace the \
                     first, so one of them would not be in force"
                ),
                self.peek(),
            ));
        }
        seen.push(key.clone());
        match key.as_str() {
            "private" => {
                constraints.private = true;
                self.opt_semi();
                Ok(())
            }
            "atomic" => {
                constraints.atomic = true;
                self.opt_semi();
                Ok(())
            }
            "capital" => {
                self.expect(Tok::Le, "expected '<=' after capital")?;
                let value = self.parse_expr()?;
                let asset = self.expect_ident("capital asset")?;
                constraints.capital = Some(AmountExpr {
                    value,
                    asset: Symbol::new(&asset),
                });
                self.opt_semi();
                Ok(())
            }
            "risk" => {
                self.expect(Tok::Le, "expected '<=' after risk")?;
                constraints.max_risk = Some(match self.peek() {
                    // `strategy` is a keyword rather than an identifier, so
                    // `strategy.policy` starts with its own token.
                    Tok::KwStrategy => {
                        self.advance();
                        self.expect(Tok::Dot, "expected '.' in `strategy.policy`")?;
                        let field = self.expect_ident("strategy field")?;
                        if field != "policy" {
                            return Err(parse_err(
                                format!("unknown strategy field '{field}'; expected `strategy.policy`"),
                                self.peek(),
                            ));
                        }
                        RiskBound::StrategyPolicy
                    }
                    _ => RiskBound::Score(self.parse_constraint_number("risk", None)?),
                });
                self.opt_semi();
                Ok(())
            }
            _ => {
                self.expect(Tok::Le, "expected '<=' in the constraint")?;
                match key.as_str() {
                    "hops" => constraints.max_hops = Some(self.parse_constraint_number("hops", None)?),
                    "chains" => constraints.max_chains = Some(self.parse_constraint_number("chains", None)?),
                    "execution_time" => {
                        constraints.max_execution_time_ms =
                            Some(self.parse_constraint_number("execution_time", Some("ms"))?)
                    }
                    "fees" => constraints.max_fees_bps = Some(self.parse_constraint_number("fees", Some("bps"))?),
                    "slippage" => {
                        constraints.max_slippage_bps = Some(self.parse_constraint_number("slippage", Some("bps"))?)
                    }
                    "finality" => {
                        constraints.max_finality_blocks =
                            Some(self.parse_constraint_number("finality", Some("blocks"))?)
                    }
                    other => {
                        return Err(parse_err(
                            format!(
                                "unknown constraint '{other}'; expected capital, hops, chains, risk, \
                                 execution_time, fees, slippage, finality, private or atomic"
                            ),
                            self.peek(),
                        ))
                    }
                }
                self.opt_semi();
                Ok(())
            }
        }
    }

    /// An integer inside `constraints`, with the unit the field is measured in.
    ///
    /// The unit is optional — the field's name is what fixes the meaning
    /// (`execution_time` is milliseconds, `fees` and `slippage` are basis
    /// points, `finality` is blocks) — and it may be written separately
    /// (`200 ms`) or attached (`200ms`), because the lexer joins a number to a
    /// following word and PHASE 15's own example writes the attached form. An
    /// attached suffix is read rather than dropped: `2_000s` for `execution_time` is a mistake
    /// about units, and stripping the digits would silently store a number the
    /// program never wrote. A suffix on a field that counts (`hops <= 4x`) is
    /// refused for the same reason. A *separate* unknown word is left where it
    /// is, so it becomes an unknown clause rather than a different number.
    fn parse_constraint_number(&mut self, field: &str, unit: Option<&str>) -> Result<u32, X3Error> {
        // `200ms` is one word to the lexer rather than a number and a unit, so
        // the attached spelling arrives here as an identifier.
        if let Tok::Ident(word) = self.peek() {
            let text = word.as_str().to_string();
            let digits = text.trim_end_matches(|ch: char| ch.is_ascii_alphabetic()).to_string();
            if !digits.is_empty() && digits.len() != text.len() {
                let written = &text[digits.len()..];
                match unit {
                    Some(unit) if written != unit => {
                        return Err(parse_err(
                            format!(
                                "objective constraint '{field}' is measured in {unit}, and \
                                 '{digits}{written}' says {written}; write '{digits} {unit}'"
                            ),
                            self.peek(),
                        ))
                    }
                    None => {
                        return Err(parse_err(
                            format!(
                                "objective constraint '{field}' counts, so it has no unit; write \
                                 '{digits}' rather than '{text}'"
                            ),
                            self.peek(),
                        ))
                    }
                    Some(_) => {}
                }
                if digits.starts_with('_') || digits.ends_with('_') || digits.contains("__") {
                    return Err(parse_err(
                        format!(
                            "objective constraint '{field}' is written '{text}', which is not a \
                             well-formed integer and its unit"
                        ),
                        self.peek(),
                    ));
                }
                self.advance();
                let plain: String = digits.chars().filter(|ch| *ch != '_').collect();
                let value: u128 = plain.parse().map_err(|_| {
                    parse_err(
                        format!(
                            "objective constraint '{field}' is written '{text}', and '{digits}' is \
                             not an integer"
                        ),
                        self.peek(),
                    )
                })?;
                return u32::try_from(value).map_err(|_| {
                    parse_err(
                        format!(
                            "objective constraint '{field}' is {value}, above the largest value the \
                             planner compares ({})",
                            u32::MAX
                        ),
                        self.peek(),
                    )
                });
            }
        }

        let expr = self.parse_expr()?;
        let value = expr_to_u128(&expr).map_err(|_| {
            parse_err(
                format!(
                    "objective constraint '{field}' must be an integer literal the compiler can \
                     evaluate"
                ),
                self.peek(),
            )
        })?;
        let value = u32::try_from(value).map_err(|_| {
            parse_err(
                format!(
                    "objective constraint '{field}' is {value}, above the largest value the planner \
                     compares ({})",
                    u32::MAX
                ),
                self.peek(),
            )
        })?;
        if let (Some(unit), Tok::Ident(ref written)) = (unit, self.peek()) {
            if written.as_str() == unit {
                self.advance();
            }
        }
        Ok(value)
    }

    /// `parallel <name> { leg <name> { <route steps> } ... }`
    ///
    /// Legs are parsed in declaration order, whatever their dependencies turn
    /// out to be: the plan is the compiler's conclusion, not the author's
    /// claim, so the parser must not reorder or drop anything before the
    /// dependency analysis sees it.
    fn parse_parallel_decl(&mut self) -> Result<ParallelDecl, X3Error> {
        self.advance(); // consume `parallel`
        let name = self.expect_ident("parallel block name")?;
        self.expect(Tok::LBrace, "expected '{' after the parallel block name")?;
        let mut legs: Vec<ParallelLeg> = Vec::new();
        while self.peek() != Tok::RBrace && self.peek() != Tok::Eof {
            match self.peek() {
                Tok::Ident(ref s) if s == "leg" => {
                    self.advance();
                    let leg_name = self.expect_ident("leg name")?;
                    self.expect(Tok::LBrace, "expected '{' after the leg name")?;
                    let mut body: Vec<Statement> = Vec::new();
                    while self.peek() != Tok::RBrace && self.peek() != Tok::Eof {
                        match self.peek() {
                            Tok::KwSwap | Tok::KwBridge | Tok::KwLock | Tok::KwMint | Tok::KwBurn | Tok::KwRelease => {
                                body.push(self.parse_route_step()?)
                            }
                            Tok::Ident(ref s)
                                if matches!(s.as_str(), "swap" | "bridge" | "lock" | "mint" | "burn" | "release") =>
                            {
                                body.push(self.parse_route_step()?)
                            }
                            // A leg that bridges must be able to declare its
                            // expiry. Without this the verdict on a bridging leg
                            // is "no refund path" — correct, and impossible for
                            // the program to answer, because a leg had no way to
                            // state a timeout. The intent clause parser is what
                            // knows the `timeout <duration> refund <asset> to
                            // <who>` form, so a leg borrows it rather than
                            // growing a second spelling of the same statement.
                            Tok::Ident(ref s) if s == "timeout" => body.push(self.parse_intent_timeout()?),
                            // A module's body can opt into a mode, and the
                            // permission it must declare for doing so is part of
                            // PHASE 23's list. Without this arm the line parses
                            // as an expression statement and the opt-in vanishes
                            // — the same way a leg's timeout did.
                            Tok::Ident(ref s) if s == "allow" => {
                                self.advance();
                                let feature = self.expect_ident("allowed feature")?;
                                self.opt_semi();
                                body.push(Statement::Allow {
                                    feature: Symbol::new(&feature),
                                });
                            }
                            _ => body.push(self.parse_statement()?),
                        }
                    }
                    self.expect(Tok::RBrace, "expected '}' to close the leg")?;
                    legs.push(ParallelLeg {
                        name: Symbol::new(&leg_name),
                        body,
                    });
                }
                other => return Err(parse_err("expected `leg <name> { ... }` inside parallel".into(), other)),
            }
        }
        self.expect(Tok::RBrace, "expected '}' to close the parallel block")?;
        Ok(ParallelDecl {
            name: Symbol::new(&name),
            legs,
        })
    }

    /// `venue <name> { kind <kind> chain <chain> domain <vm> asset_in <A>
    ///  asset_out <B> fee_bps <n> liquidity <n> slippage_bps <n>
    ///  latency_ms <n> finality_blocks <n> risk <n> [settlement <shape>]
    ///  [proof <name>] }`
    ///
    /// Every field is required except `proof`. Defaults would be worse than
    /// required fields here: a venue whose liquidity silently defaulted to zero
    /// would be unreachable in the graph, and one whose fee silently defaulted
    /// would be ranked as free — both are the kind of quiet wrong answer the
    /// graph exists to avoid.
    fn parse_venue_decl(&mut self) -> Result<VenueDecl, X3Error> {
        self.advance(); // consume `venue`
        let name = self.expect_ident("venue name")?;
        self.expect(Tok::LBrace, "expected '{' after the venue name")?;

        let mut kind: Option<VenueKind> = None;
        let mut chain: Option<ChainRef> = None;
        let mut domain: Option<Symbol> = None;
        let mut asset_in: Option<AssetRef> = None;
        let mut asset_out: Option<AssetRef> = None;
        let mut fee_bps: Option<u32> = None;
        let mut liquidity: Option<u128> = None;
        let mut slippage_bps: Option<u32> = None;
        let mut latency_ms: Option<u32> = None;
        let mut finality_blocks: Option<u32> = None;
        let mut risk: Option<u32> = None;
        let mut proof: Option<Symbol> = None;
        let mut settlement: Option<SettlementGuarantee> = None;
        let mut seen_fields: Vec<String> = Vec::new();

        while self.peek() != Tok::RBrace && self.peek() != Tok::Eof {
            let field = self.expect_ident("venue field")?;
            // A repeated field would silently overwrite the earlier one, so a
            // program could declare a venue twice in one block and believe both
            // lines. Refuse rather than pick.
            if seen_fields.contains(&field) {
                return Err(parse_err(
                    format!("venue '{name}' declares `{field}` twice; one line would silently override the other"),
                    self.peek(),
                ));
            }
            seen_fields.push(field.clone());
            match field.as_str() {
                "kind" => {
                    // `bridge` is a lexer keyword, so `kind bridge` arrives as
                    // `Tok::KwBridge` rather than an identifier. The kind set is
                    // the authority on which words are venue kinds; the token
                    // class is an accident of where else the word is used.
                    let wanted = match self.peek() {
                        Tok::KwBridge => {
                            self.advance();
                            "bridge".to_string()
                        }
                        _ => self.expect_ident("venue kind")?,
                    };
                    kind = Some(VenueKind::parse(&wanted).ok_or_else(|| {
                        let allowed: Vec<&str> = VenueKind::ALL.iter().map(|kind| kind.as_str()).collect();
                        parse_err(
                            format!("unknown venue kind '{wanted}'; expected one of: {}", allowed.join(", ")),
                            self.peek(),
                        )
                    })?);
                }
                "chain" => chain = Some(self.parse_chain_ref()?),
                "domain" => domain = Some(Symbol::new(&self.expect_ident("venue domain")?)),
                "asset_in" => asset_in = Some(self.parse_asset_ref()?),
                "asset_out" => asset_out = Some(self.parse_asset_ref()?),
                "fee_bps" => fee_bps = Some(self.parse_venue_u32("fee_bps")?),
                "liquidity" => {
                    let amount = self.parse_expr()?;
                    liquidity = Some(expr_to_u128(&amount).map_err(|_| {
                        parse_err(
                            "venue liquidity must be an integer literal; the graph compares it against \
                             trade sizes, so it cannot be an expression the compiler defers"
                                .into(),
                            self.peek(),
                        )
                    })?);
                }
                "slippage_bps" => slippage_bps = Some(self.parse_venue_u32("slippage_bps")?),
                "latency_ms" => latency_ms = Some(self.parse_venue_u32("latency_ms")?),
                "finality_blocks" => finality_blocks = Some(self.parse_venue_u32("finality_blocks")?),
                "risk" => risk = Some(self.parse_venue_u32("risk")?),
                "proof" => proof = Some(Symbol::new(&self.expect_ident("proof name")?)),
                "settlement" => {
                    // The vocabulary is the enum's, so the set of honest shapes is
                    // stated once and an unknown one is refused with the list.
                    //
                    // The word is read with `peek_word` rather than `expect_ident`
                    // because `atomic` is a keyword token everywhere else in the
                    // language, and `settlement atomic` is exactly the clause this
                    // rule exists to adjudicate.
                    let Some(wanted) = self.peek_word() else {
                        return Err(parse_err(
                            "expected a settlement guarantee: atomic, trusted_adapter, escrow, \
                             pre_funded, attested or compensating"
                                .into(),
                            self.peek(),
                        ));
                    };
                    self.advance();
                    settlement = Some(SettlementGuarantee::parse(&wanted).ok_or_else(|| {
                        let allowed: Vec<&str> = SettlementGuarantee::ALL
                            .iter()
                            .map(|guarantee| guarantee.as_str())
                            .collect();
                        parse_err(
                            format!(
                                "unknown settlement guarantee '{wanted}'; the honest shapes are: {}. \
                                 `atomic` means the venue itself enforces both sides or neither, so it \
                                 is only claimable where this VM executes the leg",
                                allowed.join(", ")
                            ),
                            self.peek(),
                        )
                    })?);
                }
                other => {
                    return Err(parse_err(
                        format!(
                            "unknown venue field '{other}'; expected kind, chain, domain, asset_in, \
                             asset_out, fee_bps, liquidity, slippage_bps, latency_ms, finality_blocks, \
                             risk, settlement or proof"
                        ),
                        self.peek(),
                    ))
                }
            }
            self.opt_semi();
        }
        self.expect(Tok::RBrace, "expected '}' to close the venue")?;

        let missing = |field: &str| parse_err(format!("venue '{name}' is missing `{field}`"), Tok::Eof);
        Ok(VenueDecl {
            name: Symbol::new(&name),
            kind: kind.ok_or_else(|| missing("kind"))?,
            chain: chain.ok_or_else(|| missing("chain"))?,
            domain: domain.ok_or_else(|| missing("domain"))?,
            asset_in: asset_in.ok_or_else(|| missing("asset_in"))?,
            asset_out: asset_out.ok_or_else(|| missing("asset_out"))?,
            fee_bps: fee_bps.ok_or_else(|| missing("fee_bps"))?,
            liquidity: liquidity.ok_or_else(|| missing("liquidity"))?,
            slippage_bps: slippage_bps.ok_or_else(|| missing("slippage_bps"))?,
            latency_ms: latency_ms.ok_or_else(|| missing("latency_ms"))?,
            finality_blocks: finality_blocks.ok_or_else(|| missing("finality_blocks"))?,
            risk: risk.ok_or_else(|| missing("risk"))?,
            settlement,
            proof,
        })
    }

    fn parse_venue_u32(&mut self, field: &str) -> Result<u32, X3Error> {
        let expr = self.parse_expr()?;
        let value = expr_to_u128(&expr).map_err(|_| {
            parse_err(
                format!("venue {field} must be an integer literal the compiler can evaluate"),
                self.peek(),
            )
        })?;
        if value > u32::MAX as u128 {
            return Err(parse_err(format!("venue {field} {value} exceeds u32"), self.peek()));
        }
        Ok(value as u32)
    }

    /// `fallback { replace with <venue> [min_output <n>]; ... require <bound>; ... }`
    ///
    /// Every replacement is listed explicitly. There is deliberately no "any
    /// venue" form and no wildcard: the compiler can only approve a
    /// substitution it can name and verify.
    fn parse_route_fallback(&mut self) -> Result<Statement, X3Error> {
        self.advance(); // consume `fallback`
        self.expect(Tok::LBrace, "expected '{' after fallback")?;
        let mut replacements: Vec<FallbackReplacement> = Vec::new();
        let mut requires: Vec<RequireGuard> = Vec::new();
        while self.peek() != Tok::RBrace && self.peek() != Tok::Eof {
            match self.peek() {
                Tok::Ident(ref s) if s == "replace" => {
                    self.advance();
                    // `replace with <venue>` and `replace <venue>` are both
                    // accepted; the `with` is there to be read, not to be
                    // required.
                    if let Tok::Ident(ref next) = self.peek() {
                        if next == "with" {
                            self.advance();
                        }
                    }
                    let venue = self.expect_ident("replacement venue")?;
                    let mut min_output: Option<Expression> = None;
                    if let Tok::Ident(ref next) = self.peek() {
                        if next == "min_output" {
                            self.advance();
                            min_output = Some(self.parse_expr()?);
                        }
                    }
                    self.opt_semi();
                    replacements.push(FallbackReplacement {
                        venue: Symbol::new(&venue),
                        min_output,
                    });
                }
                Tok::KwRequire => requires.push(self.parse_require_guard()?),
                other => {
                    return Err(parse_err(
                        "expected `replace with <venue>` or a `require` bound inside fallback".into(),
                        other,
                    ))
                }
            }
        }
        self.expect(Tok::RBrace, "expected '}' to close fallback")?;
        self.opt_semi();
        Ok(Statement::RouteFallback { replacements, requires })
    }

    fn parse_swap_step(&mut self) -> Result<Statement, X3Error> {
        let dex = self.expect_ident("swap dex")?;
        let from = self.parse_asset_ref()?;
        let to = if self.peek() == Tok::Arrow {
            self.advance();
            self.parse_asset_ref()?
        } else {
            AssetRef::new(from.chain.clone(), Symbol::new(""))
        };
        let mut amount: Option<Expression> = None;
        let mut min_output: Option<Expression> = None;
        loop {
            match self.peek() {
                Tok::Ident(ref s) if s == "amount" => {
                    self.advance();
                    amount = Some(self.parse_expr()?);
                }
                Tok::Ident(ref s) if s == "min_output" => {
                    self.advance();
                    min_output = Some(self.parse_expr()?);
                }
                _ => break,
            }
        }
        self.opt_semi();
        let dex_expr = Some(Expression::Literal(LiteralExpr::String(Symbol::new(&dex))));
        Ok(Statement::Swap {
            from,
            to,
            amount,
            min_output,
            dex: dex_expr,
        })
    }

    /// `bridge <via> <chain.ASSET> -> <chain.ASSET> [receiver <expr>]
    ///   [finality_proof <expr>] [transfer_proof <expr>]`
    /// The production intent pass fills its amount from the matching
    /// `from` endpoint.
    fn parse_bridge_step(&mut self) -> Result<Statement, X3Error> {
        // The dispatcher in `parse_route_step` only consumes the
        // leading `bridge` keyword when the tokenizer gave it a
        // KwBridge token. When the source uses the bare-identifier
        // form, the cursor is still on `bridge` — skip it here.
        if let Tok::Ident(ref s) = self.peek() {
            if s == "bridge" {
                self.advance();
            }
        }
        let via = self.expect_ident("bridge via")?;
        let from = self.parse_asset_ref()?;
        let to = if self.peek() == Tok::Arrow {
            self.advance();
            self.parse_asset_ref()?
        } else {
            AssetRef::new(from.chain.clone(), from.name.clone())
        };
        let mut amount: Option<Expression> = None;
        let mut receiver: Option<Expression> = None;
        let mut min_receive: Option<Expression> = None;
        let mut source_finality_proof: Option<Expression> = None;
        let mut transfer_proof: Option<Expression> = None;
        loop {
            match self.peek() {
                Tok::Ident(ref s) if s == "amount" => {
                    self.advance();
                    amount = Some(self.parse_expr()?);
                }
                Tok::Ident(ref s) if s == "receiver" => {
                    self.advance();
                    if !matches!(self.peek(), Tok::RBrace | Tok::Eof) {
                        receiver = Some(self.parse_expr()?);
                    }
                }
                Tok::Ident(ref s) if s == "min_receive" => {
                    self.advance();
                    min_receive = Some(self.parse_expr()?);
                }
                Tok::Ident(ref s) if s == "finality_proof" => {
                    self.advance();
                    source_finality_proof = Some(self.parse_expr()?);
                }
                Tok::Ident(ref s) if s == "source_finality_proof" => {
                    self.advance();
                    source_finality_proof = Some(self.parse_expr()?);
                }
                Tok::Ident(ref s) if s == "transfer_proof" => {
                    self.advance();
                    transfer_proof = Some(self.parse_expr()?);
                }
                _ => break,
            }
        }
        self.opt_semi();
        Ok(Statement::Bridge {
            via: Symbol::new(&via),
            from,
            to,
            amount: amount.unwrap_or_else(|| {
                Expression::Literal(LiteralExpr::Int {
                    value: 0,
                    base: IntBase::Decimal,
                    suffix: None,
                })
            }),
            receiver: receiver.unwrap_or_else(|| Expression::Ident(Symbol::new("receiver"))),
            min_receive,
            source_finality_proof,
            transfer_proof,
        })
    }

    /// A duration, in the two forms the language writes: a bare number of
    /// *blocks*, or a number with a time unit attached (`180s`, `40m`, `2h`).
    ///
    /// The two used to be the same token to everyone downstream and meant
    /// different things in different layers: `40m` reached the parser as one
    /// identifier, `lowering` read 40 blocks from its digits, and the atomic
    /// swap's ordering check read a bare number as *seconds* and skipped an
    /// identifier altogether — so the invariant that a source timeout outlasts
    /// its destination was not checked at all for a program that wrote its units.
    /// (TICKET-033.)
    ///
    /// A suffix the language does not know is refused here rather than dropped:
    /// `timeout 40x` is a program that said something about time and nothing
    /// about blocks.
    fn parse_duration_expr(&mut self, what: &str) -> Result<Expression, X3Error> {
        if let Tok::Float(_) = self.peek() {
            return Err(parse_err(
                format!(
                    "{what} is a count of blocks or a whole number with a unit; a fractional \
                     duration is not one of them"
                ),
                self.peek(),
            ));
        }
        if let Tok::Ident(word) = self.peek() {
            let text = word.as_str().to_string();
            let digits = text.trim_end_matches(|ch: char| ch.is_ascii_alphabetic());
            if !digits.is_empty() && digits.len() != text.len() {
                let suffix = &text[digits.len()..];
                if digits.contains('.') {
                    return Err(parse_err(
                        format!(
                            "{what} '{text}' is fractional; a duration is a whole number of blocks \
                             or of a time unit"
                        ),
                        self.peek(),
                    ));
                }
                let value: u64 = digits
                    .trim_end_matches('_')
                    .replace('_', "")
                    .parse()
                    .map_err(|_| parse_err(format!("{what} '{text}' has no number in it"), self.peek()))?;
                if suffix == "blocks" {
                    self.advance();
                    return Ok(Expression::Literal(LiteralExpr::Int {
                        value: u128::from(value),
                        base: IntBase::Decimal,
                        suffix: None,
                    }));
                }
                let Some(unit) = duration_unit_from_suffix(suffix) else {
                    return Err(parse_err(
                        format!(
                            "{what} '{text}' uses the unit '{suffix}', which the language does not \
                             define; write blocks as a bare number, or one of s, m, h, d, ms, us, ns"
                        ),
                        self.peek(),
                    ));
                };
                self.advance();
                return Ok(Expression::Literal(LiteralExpr::Duration { value, unit }));
            }
        }
        let expr = self.parse_expr()?;
        // The unit as its own word: `40 blocks` (the form the trading deadline is
        // written in) and the time words (`30 seconds`), which mean what the
        // suffix forms mean. `blocks` stays a count of blocks.
        if let Tok::Ident(word) = self.peek() {
            let text = word.as_str().to_string();
            if text == "blocks" {
                self.advance();
            } else if let Some(unit) = duration_unit_from_word(&text) {
                self.advance();
                let value: u64 = match &expr {
                    Expression::Literal(LiteralExpr::Int { value, .. }) => u64::try_from(*value)
                        .map_err(|_| parse_err(format!("{what} is too large to be a duration"), self.peek()))?,
                    _ => {
                        return Err(parse_err(
                            format!("{what} must be a whole number of {text}"),
                            self.peek(),
                        ))
                    }
                };
                return Ok(Expression::Literal(LiteralExpr::Duration { value, unit }));
            }
        }
        Ok(expr)
    }

    /// `timeout <N>[s] [refund <chain.ASSET> to <receiver>]`
    fn parse_intent_timeout(&mut self) -> Result<Statement, X3Error> {
        self.advance(); // consume `timeout`
        let dur = self.parse_duration_expr("timeout")?;
        let mut action = FailureAction::Rollback;
        loop {
            match self.peek() {
                Tok::Ident(ref s) if s == "refund" => {
                    self.advance();
                    let refund_asset = self.parse_asset_ref().ok();
                    let mut receiver = None;
                    if let Tok::Ident(ref s) = self.peek() {
                        if s == "to" {
                            self.advance();
                            receiver = self.parse_expr().ok();
                        }
                    }
                    if let Some(asset) = refund_asset {
                        let receiver = receiver
                            .map(|expr| expression_debug_string(&expr))
                            .unwrap_or_else(|| "sender".to_string());
                        action = FailureAction::Refund(Expression::Literal(LiteralExpr::String(Symbol::new(
                            &format!("{}.{}:{}", asset.chain.as_str(), asset.name.as_str(), receiver),
                        ))));
                    }
                }
                _ => break,
            }
        }
        self.opt_semi();
        // The duration is kept as the program wrote it — blocks or time — and
        // converted once, in lowering. It used to be converted here, which made
        // the AST hold blocks while the ordering check read the same field as
        // seconds.
        Ok(Statement::OnTimeout { duration: dur, action })
    }

    /// `on_fail rollback | halt | quarantine | refund <chain.ASSET> to <receiver>`
    fn parse_intent_onfail(&mut self) -> Result<Statement, X3Error> {
        self.advance(); // consume `on_fail`
        let action = match self.peek() {
            Tok::Ident(ref s) if s == "rollback" => {
                self.advance();
                FailureAction::Rollback
            }
            Tok::Ident(ref s) if s == "halt" => {
                self.advance();
                FailureAction::Halt
            }
            Tok::Ident(ref s) if s == "quarantine" => {
                self.advance();
                FailureAction::Quarantine
            }
            Tok::Ident(ref s) if s == "refund" => {
                self.advance();
                let refund_asset = self.parse_asset_ref().ok();
                let mut receiver = None;
                if let Tok::Ident(ref s) = self.peek() {
                    if s == "to" {
                        self.advance();
                        receiver = self.parse_expr().ok();
                    }
                }
                if let Some(asset) = refund_asset {
                    let receiver = receiver
                        .map(|expr| expression_debug_string(&expr))
                        .unwrap_or_else(|| "sender".to_string());
                    FailureAction::Refund(Expression::Literal(LiteralExpr::String(Symbol::new(&format!(
                        "{}.{}:{}",
                        asset.chain.as_str(),
                        asset.name.as_str(),
                        receiver
                    )))))
                } else {
                    FailureAction::Rollback
                }
            }
            _ => {
                return Err(parse_err(
                    "expected rollback | halt | quarantine | refund".into(),
                    self.peek(),
                ));
            }
        };
        self.opt_semi();
        Ok(Statement::OnFail(action))
    }

    /// `use solver_market best_price | use relayer_quorum N_of_M | use rpc_quorum N_of_M`
    fn parse_intent_use(&mut self) -> Result<Statement, X3Error> {
        self.advance(); // `use`
        let target = self.expect_ident("use target")?;
        let config = self.parse_expr()?;
        self.opt_semi();
        Ok(Statement::Expr(Expression::Call {
            callee: Box::new(Expression::Ident(Symbol::new("use"))),
            args: vec![Expression::Ident(Symbol::new(&target)), config],
        }))
    }

    /// `on bad_proof slash | on chain_halt pause_and_refund | on solver_fail refund_and_slash
    ///  on relayer_fail use_backup_relayer | on proof_conflict dispute`
    fn parse_intent_on_event(&mut self) -> Result<Statement, X3Error> {
        self.advance(); // `on`
        let event = self.expect_ident("event name")?;
        let action = self.expect_ident("action")?;
        self.opt_semi();
        Ok(Statement::Expr(Expression::Call {
            callee: Box::new(Expression::Ident(Symbol::new("on"))),
            args: vec![
                Expression::Ident(Symbol::new(&event)),
                Expression::Ident(Symbol::new(&action)),
            ],
        }))
    }

    fn parse_subscription_item(&mut self) -> Result<Item, X3Error> {
        self.advance();
        let name = self.expect_ident("subscription name")?;
        self.expect(Tok::Colon, "expected ':'")?;
        let amount = self.parse_expr()?;
        let amount_val = expr_to_u128(&amount)?;
        let period_blocks = if self.peek() == Tok::Comma {
            self.advance();
            let p = self.parse_expr()?;
            expr_to_u64(&p)
        } else {
            1
        };
        let body = self.parse_block()?;
        Ok(Item::SubscriptionDecl(SubscriptionDecl {
            name: Symbol::new(&name),
            amount: amount_val,
            period_blocks,
            body,
        }))
    }

    // ------------------------------------------------------------------
    // B-52 Feature Lock item parsers
    // ------------------------------------------------------------------

    /// `solver_market { mode <symbol>, min_reputation <int> }`
    fn parse_solver_market_item(&mut self) -> Result<Item, X3Error> {
        self.advance();
        self.expect(Tok::LBrace, "expected '{' after solver_market")?;
        let mut mode = Symbol::new("competitive");
        let mut min_reputation: u64 = 0;
        let mut bond: Option<AmountExpr> = None;
        while self.peek() != Tok::RBrace && self.peek() != Tok::Eof {
            match self.peek() {
                Tok::Ident(ref s) if s == "mode" => {
                    self.advance();
                    mode = Symbol::new(&self.expect_ident("solver market mode")?);
                }
                Tok::Ident(ref s) if s == "min_reputation" => {
                    self.advance();
                    let expr = self.parse_expr()?;
                    min_reputation = expr_to_u64(&expr);
                }
                Tok::Ident(ref s) if s == "bond" => {
                    self.advance();
                    let value = self.parse_amount_expr("solver bond")?;
                    if bond.replace(value).is_some() {
                        return Err(parse_err("duplicate 'bond' in solver_market".into(), self.peek()));
                    }
                }
                _ => break,
            }
        }
        self.expect(Tok::RBrace, "expected '}' after solver_market body")?;
        Ok(Item::SolverMarket(SolverMarket {
            mode,
            min_reputation,
            bond,
        }))
    }

    /// `relayers { quorum <n>_of_<m> ... }`
    fn parse_relayer_swarm_item(&mut self) -> Result<Item, X3Error> {
        self.advance();
        self.expect(Tok::LBrace, "expected '{' after relayers")?;
        let mut quorum_numerator: u32 = 1;
        let mut quorum_denominator: u32 = 1;
        let mut relayers = Vec::new();
        while self.peek() != Tok::RBrace && self.peek() != Tok::Eof {
            match self.peek() {
                Tok::Ident(ref s) if s == "quorum" => {
                    self.advance();
                    let (n, m) = self.parse_n_of_m()?;
                    quorum_numerator = n;
                    quorum_denominator = m;
                }
                Tok::Ident(ref s) if s == "quorum_numerator" => {
                    self.advance();
                    let expr = self.parse_expr()?;
                    quorum_numerator = expr_to_u32(&expr)?;
                }
                Tok::Ident(ref s) if s == "quorum_denominator" => {
                    self.advance();
                    let expr = self.parse_expr()?;
                    quorum_denominator = expr_to_u32(&expr)?;
                }
                Tok::Ident(ref s) if s == "relayers" => {
                    self.advance();
                    self.expect(Tok::LBracket, "expected '[' for relayers list")?;
                    while self.peek() != Tok::RBracket && self.peek() != Tok::Eof {
                        let name = self.expect_ident("relayer name")?;
                        relayers.push(Symbol::new(&name));
                        if self.peek() == Tok::Comma {
                            self.advance();
                        }
                    }
                    self.expect(Tok::RBracket, "expected ']' after relayers list")?;
                }
                _ => {
                    // Skip unknown config entries
                    self.advance();
                }
            }
        }
        self.expect(Tok::RBrace, "expected '}' after relayers body")?;
        Ok(Item::RelayerSwarm(RelayerSwarm {
            quorum_numerator,
            quorum_denominator,
            relayers,
        }))
    }

    /// `rpc_quorum { source require <n>_of_<m> destination require <n>_of_<m> reject if <reason> }`
    fn parse_rpc_quorum_item(&mut self) -> Result<Item, X3Error> {
        self.advance();
        self.expect(Tok::LBrace, "expected '{' after rpc_quorum")?;
        let mut source = Symbol::new("unknown");
        let mut require_numerator: u32 = 1;
        let mut require_denominator: u32 = 1;
        let mut reject_on = Vec::new();
        while self.peek() != Tok::RBrace && self.peek() != Tok::Eof {
            match self.peek() {
                Tok::Ident(ref s) if s == "source" => {
                    self.advance();
                    // source can be followed by chain name OR directly by require
                    // `require` reaches the parser as `KwRequire` — it is a lexer keyword — so the
                    // identifier form this check used to test could never match, and the inline
                    // `source require 2_of_3` form was refused with "rpc_quorum source chain:
                    // expected identifier" (TICKET-046's class).
                    if matches!(self.peek(), Tok::KwRequire) {
                        // inline: source require N_of_M
                        source = Symbol::new("source");
                    } else {
                        source = Symbol::new(&self.expect_ident("rpc_quorum source chain")?);
                    }
                }
                Tok::Ident(ref s) if s == "destination" => {
                    self.advance();
                    // skip, we only store the source
                }
                Tok::KwRequire => {
                    self.advance();
                    let (n, m) = self.parse_n_of_m()?;
                    require_numerator = n;
                    require_denominator = m;
                }
                Tok::Ident(ref s) if s == "require_numerator" => {
                    self.advance();
                    let expr = self.parse_expr()?;
                    require_numerator = expr_to_u32(&expr)?;
                }
                Tok::Ident(ref s) if s == "require_denominator" => {
                    self.advance();
                    let expr = self.parse_expr()?;
                    require_denominator = expr_to_u32(&expr)?;
                }
                Tok::Ident(ref s) if s == "reject" || s == "reject_on" => {
                    self.advance();
                    if matches!(self.peek(), Tok::KwIf) {
                        self.advance(); // `if`
                    }
                    if self.peek() == Tok::LBracket {
                        self.advance();
                        while self.peek() != Tok::RBracket && self.peek() != Tok::Eof {
                            let reason = self.expect_ident("reject reason")?;
                            reject_on.push(Symbol::new(&reason));
                            if self.peek() == Tok::Comma {
                                self.advance();
                            }
                        }
                        self.expect(Tok::RBracket, "expected ']' after reject_on list")?;
                    } else {
                        let reason = self.expect_ident("reject reason")?;
                        reject_on.push(Symbol::new(&reason));
                    }
                }
                _ => {
                    // Skip unknown entries
                    self.advance();
                }
            }
        }
        self.expect(Tok::RBrace, "expected '}' after rpc_quorum body")?;
        Ok(Item::RpcQuorum(RpcQuorum {
            source,
            require_numerator,
            require_denominator,
            reject_on,
        }))
    }

    /// `risk_policy { max_slippage <pct> max_position <int> min_route_score <n> }`
    ///
    /// An unknown field is refused by name rather than skipped. The comment here
    /// used to advertise `max_fee`, `max_route_risk` and `min_liquidity`, none of
    /// which the parser read: a program that wrote one of them declared a bound
    /// that was dropped without a word (TICKET-050).
    fn parse_risk_policy_item(&mut self) -> Result<Item, X3Error> {
        self.advance();
        self.expect(Tok::LBrace, "expected '{' after risk_policy")?;
        let mut max_slippage: u64 = 0;
        let mut max_position: Option<u128> = None;
        let mut min_route_score: Option<u32> = None;
        while self.peek() != Tok::RBrace && self.peek() != Tok::Eof {
            match self.peek() {
                Tok::Ident(ref s) if s == "max_slippage" => {
                    self.advance();
                    let expr = self.parse_expr()?;
                    max_slippage = expr_to_u64(&expr);
                }
                Tok::Ident(ref s) if s == "max_position" => {
                    self.advance();
                    let expr = self.parse_expr()?;
                    max_position = expr_to_u128(&expr).ok();
                }
                Tok::Ident(ref s) if s == "min_route_score" => {
                    self.advance();
                    let expr = self.parse_expr()?;
                    min_route_score = Some(expr_to_u32(&expr).unwrap_or(0));
                }
                Tok::Ident(ref field) => {
                    return Err(parse_err(
                        format!(
                            "unknown risk_policy field '{field}'; the fields are max_slippage, \
                             max_position and min_route_score"
                        ),
                        self.peek(),
                    ))
                }
                other => {
                    return Err(parse_err(
                        format!("expected a risk_policy field, found {other:?}"),
                        other,
                    ))
                }
            }
        }
        self.expect(Tok::RBrace, "expected '}' after risk_policy body")?;
        Ok(Item::RiskPolicy(RiskPolicy {
            max_slippage,
            max_position,
            min_route_score,
        }))
    }

    /// `privacy { hide_route_until_commit <bool>, reveal_on <symbol>, encrypted <bool> }`
    fn parse_privacy_block_item(&mut self) -> Result<Item, X3Error> {
        self.advance();
        self.expect(Tok::LBrace, "expected '{' after privacy")?;
        let mut hide_route_until_commit = false;
        let mut reveal_on = Symbol::new("claim");
        let mut encrypted = false;
        while self.peek() != Tok::RBrace && self.peek() != Tok::Eof {
            match self.peek() {
                Tok::Ident(ref s) if s == "hide_route_until_commit" => {
                    self.advance();
                    let expr = self.parse_expr()?;
                    hide_route_until_commit = expr_to_bool(&expr);
                }
                Tok::Ident(ref s) if s == "reveal_on" => {
                    self.advance();
                    reveal_on = Symbol::new(&self.expect_ident("reveal_on trigger")?);
                }
                Tok::Ident(ref s) if s == "encrypted" => {
                    self.advance();
                    let expr = self.parse_expr()?;
                    encrypted = expr_to_bool(&expr);
                }
                _ => break,
            }
        }
        self.expect(Tok::RBrace, "expected '}' after privacy body")?;
        Ok(Item::PrivacyBlock(PrivacyBlock {
            hide_route_until_commit,
            reveal_on,
            encrypted,
        }))
    }

    /// `invariant <name> { assert <expr> }`
    fn parse_invariant_decl_item(&mut self) -> Result<Item, X3Error> {
        self.advance();
        let name = Symbol::new(&self.expect_ident("invariant name")?);
        let assert_expr = if self.peek() == Tok::LBrace {
            self.advance();
            let assert_kw = self.expect_ident("expected 'assert' in invariant body")?;
            if assert_kw != "assert" {
                return Err(parse_err(
                    format!("expected 'assert' in the invariant body, found '{assert_kw}'"),
                    self.peek(),
                ));
            }
            let expr = self.parse_expr()?;
            self.expect(Tok::RBrace, "expected '}' after invariant body")?;
            // The source form, not a Rust value tree: the body reaches the IR as a
            // string, and the formatter has to be able to write it back
            // (TICKET-044).
            Symbol::new(&crate::formatter::expression_to_source(&expr))
        } else {
            name.clone()
        };
        Ok(Item::InvariantDecl(InvariantDecl { name, assert_expr }))
    }

    /// `proofs required { <proof_name> ... }`
    fn parse_proofs_required_item(&mut self) -> Result<Item, X3Error> {
        self.advance();
        let required = self.expect_ident("expected 'required' after 'proofs'")?;
        if required != "required" {
            return Err(parse_err(
                "expected 'required' after 'proofs'".into(),
                Tok::Ident(required),
            ));
        }
        self.expect(Tok::LBrace, "expected '{' for proofs list")?;
        let mut proofs = Vec::new();
        while self.peek() != Tok::RBrace && self.peek() != Tok::Eof {
            let name = self.expect_ident("proof name")?;
            proofs.push(Symbol::new(&name));
        }
        self.expect(Tok::RBrace, "expected '}' after proofs list")?;
        Ok(Item::ProofsRequired(ProofsRequired { proofs }))
    }

    /// `vm { chain <symbol>, adapter <symbol>, finality <symbol> }`
    fn parse_vm_decl_item(&mut self) -> Result<Item, X3Error> {
        self.advance(); // `vm`
        self.expect(Tok::LBrace, "expected '{' after vm")?;
        let mut chain = Symbol::new("unknown");
        let mut adapter = Symbol::new("unknown");
        let mut finality: Option<Symbol> = None;
        while self.peek() != Tok::RBrace && self.peek() != Tok::Eof {
            match self.peek() {
                Tok::Ident(ref s) if s == "chain" => {
                    self.advance();
                    chain = Symbol::new(&self.expect_ident("vm chain name")?);
                }
                Tok::Ident(ref s) if s == "adapter" => {
                    self.advance();
                    adapter = Symbol::new(&self.expect_ident("vm adapter name")?);
                }
                Tok::Ident(ref s) if s == "finality" => {
                    self.advance();
                    finality = Some(Symbol::new(&self.expect_ident("vm finality requirement")?));
                }
                _ => break,
            }
        }
        self.expect(Tok::RBrace, "expected '}' after vm body")?;
        Ok(Item::VmDecl(VmDecl {
            chain,
            adapter,
            finality,
        }))
    }

    /// `target <vm> { adapter <symbol>, contract <symbol> }`
    fn parse_vm_target_item(&mut self) -> Result<Item, X3Error> {
        self.advance();
        let vm = Symbol::new(&self.expect_ident("target vm name")?);
        self.expect(Tok::LBrace, "expected '{' after target")?;
        let mut adapter = Symbol::new("unknown");
        let mut contract: Option<Symbol> = None;
        while self.peek() != Tok::RBrace && self.peek() != Tok::Eof {
            match self.peek() {
                Tok::Ident(ref s) if s == "adapter" => {
                    self.advance();
                    adapter = Symbol::new(&self.expect_ident("target adapter name")?);
                }
                Tok::Ident(ref s) if s == "contract" => {
                    self.advance();
                    let expr = self.parse_expr()?;
                    contract = Some(match &expr {
                        Expression::Ident(sym) => sym.clone(),
                        Expression::Literal(LiteralExpr::Address(sym)) => sym.clone(),
                        Expression::Literal(LiteralExpr::String(sym)) => sym.clone(),
                        _ => Symbol::new(&expr_to_string(&expr)),
                    });
                }
                _ => break,
            }
        }
        self.expect(Tok::RBrace, "expected '}' after target body")?;
        Ok(Item::VmTarget(VmTarget { vm, adapter, contract }))
    }

    /// `rebalance <name> { <ASSET> = <pct>; … minimize { <metric>; … } atomic; }`
    ///
    /// A target portfolio. The weights are percentages and the `minimize` set names
    /// metrics the optimizer knows; `atomic` is required, because a rebalance that
    /// can half-execute leaves the portfolio off-target (PHASE 11).
    fn parse_rebalance_item(&mut self) -> Result<Item, X3Error> {
        self.advance();
        let name = Symbol::new(&self.expect_ident("rebalance name")?);
        self.expect(Tok::LBrace, "expected '{' after the rebalance name")?;
        let mut weights: Vec<(AssetRef, u32)> = Vec::new();
        let mut holdings: Vec<(AssetRef, u128)> = Vec::new();
        let mut minimize: Vec<ObjectiveMetric> = Vec::new();
        let mut atomic = false;

        while self.peek() != Tok::RBrace && self.peek() != Tok::Eof {
            if self.peek() == Tok::KwAtomic {
                self.advance();
                self.opt_semi();
                atomic = true;
                continue;
            }
            let clause = self.peek_word().ok_or_else(|| {
                parse_err(
                    "expected a weight, `holds { … }`, `minimize { … }` or `atomic;`".into(),
                    self.peek(),
                )
            })?;
            if clause == "holds" {
                // What the account holds now, which is the input the target alone lacks.
                self.advance();
                self.expect(Tok::LBrace, "expected '{' after `holds`")?;
                while self.peek() != Tok::RBrace && self.peek() != Tok::Eof {
                    let asset = self.parse_hedge_asset()?;
                    self.expect(Tok::Eq, "expected '=' after the asset in a holding")?;
                    let amount = match self.peek() {
                        Tok::Int(value) => {
                            self.advance();
                            value
                        }
                        _ => {
                            return Err(parse_err(
                                "a holding is an amount in the asset's own units: write \
                                 `<chain.ASSET> = <n>`"
                                    .into(),
                                self.peek(),
                            ))
                        }
                    };
                    self.opt_semi();
                    holdings.push((asset, amount));
                }
                self.expect(Tok::RBrace, "expected '}' after the holdings")?;
                self.opt_semi();
                continue;
            }
            if clause == "minimize" {
                self.advance();
                self.expect(Tok::LBrace, "expected '{' after `minimize`")?;
                while self.peek() != Tok::RBrace && self.peek() != Tok::Eof {
                    // The metric vocabulary is the objective's: one place says which
                    // metrics exist and which direction each has.
                    let metric = self.parse_metric_name("minimize")?;
                    self.opt_semi();
                    minimize.push(metric);
                }
                self.expect(Tok::RBrace, "expected '}' after the minimize set")?;
                self.opt_semi();
                continue;
            }

            // `<ASSET> = <pct>` — the asset is an identifier or `chain.ASSET`.
            let asset = self.parse_hedge_asset()?;
            self.expect(Tok::Eq, "expected '=' after the asset in a rebalance weight")?;
            let percent = match self.peek() {
                Tok::Int(value) => {
                    self.advance();
                    u32::try_from(value).map_err(|_| {
                        parse_err(
                            "a portfolio weight is a percentage between 0 and 100".into(),
                            self.peek(),
                        )
                    })?
                }
                Tok::Float(value) => {
                    return Err(parse_err(
                        format!(
                            "the weight '{value}' is fractional; weights are whole percentages, and a \
                             share finer than one percent is not representable yet rather than rounded"
                        ),
                        self.peek(),
                    ))
                }
                _ => {
                    return Err(parse_err(
                        "a portfolio weight is a percentage: write `<ASSET> = <n>%`".into(),
                        self.peek(),
                    ))
                }
            };
            // `40%` is an integer and the `%` operator token; the sign is written
            // because the weight is a share of the portfolio, not a count.
            if self.peek() == Tok::Percent {
                self.advance();
            } else {
                return Err(parse_err(
                    "a portfolio weight is a percentage: write `<ASSET> = <n>%`".into(),
                    self.peek(),
                ));
            }
            self.opt_semi();
            weights.push((asset, percent));
        }
        self.expect(Tok::RBrace, "expected '}' after the rebalance")?;

        if !atomic {
            return Err(parse_err(
                "a `rebalance` has to say `atomic;`: a partially executed rebalance leaves the \
                 portfolio off-target, which is the state the declaration exists to reach"
                    .into(),
                self.peek(),
            ));
        }
        Ok(Item::Rebalance(RebalanceDecl {
            name,
            holdings,
            weights,
            minimize,
        }))
    }

    /// `netting <name> { consent <party>; <debtor> owes <amount> <chain.ASSET> to
    /// <creditor>; … }` — spec PHASE 22.
    ///
    /// Two clause forms, told apart by their first word, the way every other block
    /// in this language is read: `consent <party>` and `<debtor> owes <amount>
    /// <chain.ASSET> to <creditor>`. The obligation clause starts with the debtor's
    /// name, which is the only thing that can start it, so the parser never has to
    /// guess which form it is looking at.
    ///
    /// Nothing here validates the book — whether the weights of an obligation set
    /// add up, whether a party consented, whether two obligations are even
    /// comparable is decided in `compiler/src/netting.rs`, where the reason can be
    /// stated with figures. The parser's job is only to say what was written.
    fn parse_netting_item(&mut self) -> Result<Item, X3Error> {
        self.advance();
        let name = Symbol::new(&self.expect_ident("netting name")?);
        self.expect(Tok::LBrace, "expected '{' after the netting name")?;
        let mut consent: Vec<Symbol> = Vec::new();
        let mut accounts: Vec<(Symbol, Expression)> = Vec::new();
        let mut obligations: Vec<ObligationDecl> = Vec::new();

        while self.peek() != Tok::RBrace && self.peek() != Tok::Eof {
            let clause = self.peek_word().ok_or_else(|| {
                parse_err(
                    "expected `consent <party>`, `account <party> = <address>` or `<debtor> owes \
                     <amount> <chain.ASSET> to <creditor>`"
                        .into(),
                    self.peek(),
                )
            })?;

            if clause == "consent" {
                self.advance();
                let party = Symbol::new(&self.expect_ident("the party consenting to netting")?);
                self.opt_semi();
                consent.push(party);
                continue;
            }

            if clause == "account" {
                self.advance();
                let party = Symbol::new(&self.expect_ident("the party whose account this is")?);
                self.expect(Tok::Eq, "expected '=' after the party in an `account` clause")?;
                // An address is an expression for the same reason a lock's payer is
                // (`lock … from <expr>`): the language has hexadecimal literals and
                // string literals for addresses, and one reader serves both.
                let address = self.parse_expr()?;
                self.opt_semi();
                accounts.push((party, address));
                continue;
            }

            // `<debtor> owes <amount> <chain.ASSET> to <creditor>`
            let debtor = Symbol::new(&clause);
            self.advance();
            let verb = self.expect_ident("`owes`")?;
            if verb != "owes" {
                return Err(parse_err(
                    format!(
                        "expected `{clause} owes <amount> <chain.ASSET> to <creditor>`, found \
                         `{clause} {verb}`; an obligation is written as an amount one party owes \
                         another"
                    ),
                    self.peek(),
                ));
            }
            let amount = match self.peek() {
                Tok::Int(value) => {
                    self.advance();
                    value
                }
                _ => {
                    return Err(parse_err(
                        format!(
                            "`{clause} owes <amount> …` needs a whole number of base units: an \
                             obligation with a fractional amount would have to name its rounding, \
                             and how much of a debt is discharged is not a rounding decision"
                        ),
                        self.peek(),
                    ))
                }
            };
            let asset = self.parse_hedge_asset()?;
            let connective = self.expect_ident("`to`")?;
            if connective != "to" {
                return Err(parse_err(
                    format!(
                        "an obligation names the party it is owed to: write `{clause} owes \
                         {amount} <chain.ASSET> to <creditor>`, found `{connective}`"
                    ),
                    self.peek(),
                ));
            }
            let creditor = Symbol::new(&self.expect_ident("the party the obligation is owed to")?);
            self.opt_semi();
            obligations.push(ObligationDecl {
                debtor,
                creditor,
                amount,
                asset,
            });
        }
        self.expect(Tok::RBrace, "expected '}' after the netting book")?;

        if obligations.is_empty() {
            return Err(parse_err(
                format!(
                    "the netting book '{name}' declares no obligation; a book with nothing in it \
                     has no net position and nothing to offset"
                ),
                self.peek(),
            ));
        }
        Ok(Item::Netting(NettingDecl {
            name,
            consent,
            accounts,
            obligations,
        }))
    }

    /// `arb <name> { discover { … } capital { … } execution { … } risk { … } }` —
    /// spec PHASE 37.
    ///
    /// Four blocks, each `<clause> = <value>;`. Nothing is validated here: which
    /// hop bounds are sane, whether a chain in scope can reach an asset, whether
    /// flash capital is allowed to ship at all, and whether the declared risk
    /// floor is enforced by a guard are decided in `compiler/src/arb.rs`, where
    /// the reason can be stated with figures. The parser's job is to say what was
    /// written, including refusing a clause it has already seen — a block that
    /// says `max_hops` twice has two answers and no way to pick between them.
    fn parse_arb_item(&mut self) -> Result<Item, X3Error> {
        self.advance();
        let name = Symbol::new(&self.expect_ident("arb name")?);
        self.expect(Tok::LBrace, "expected '{' after the arb name")?;

        let mut discover: Option<ArbDiscover> = None;
        let mut capital: Option<ArbCapital> = None;
        let mut execution: Option<ArbExecution> = None;
        let mut risk: Option<ArbRisk> = None;

        while self.peek() != Tok::RBrace && self.peek() != Tok::Eof {
            let block = self.peek_word().ok_or_else(|| {
                parse_err(
                    "expected `discover { … }`, `capital { … }`, `execution { … }` or `risk { … }`".into(),
                    self.peek(),
                )
            })?;
            self.advance();
            let slot_name = match block.as_str() {
                "discover" | "capital" | "execution" | "risk" => block.clone(),
                other => {
                    return Err(parse_err(
                        format!(
                            "unknown arb block '{other}'; an `arb` declares `discover`, `capital`, \
                             `execution` and `risk`"
                        ),
                        self.peek(),
                    ))
                }
            };
            self.expect(Tok::LBrace, &format!("expected '{{' after `{block}`"))?;
            match block.as_str() {
                "discover" => {
                    if discover.is_some() {
                        return Err(parse_err("duplicate `discover` block in arb".into(), self.peek()));
                    }
                    discover = Some(self.parse_arb_discover()?);
                }
                "capital" => {
                    if capital.is_some() {
                        return Err(parse_err("duplicate `capital` block in arb".into(), self.peek()));
                    }
                    capital = Some(self.parse_arb_capital()?);
                }
                "execution" => {
                    if execution.is_some() {
                        return Err(parse_err("duplicate `execution` block in arb".into(), self.peek()));
                    }
                    execution = Some(self.parse_arb_execution()?);
                }
                _ => {
                    if risk.is_some() {
                        return Err(parse_err("duplicate `risk` block in arb".into(), self.peek()));
                    }
                    risk = Some(self.parse_arb_risk()?);
                }
            }
            self.expect(Tok::RBrace, &format!("expected '}}' after the `{slot_name}` block"))?;
            self.opt_semi();
        }
        self.expect(Tok::RBrace, "expected '}' after the arb")?;

        // Built here rather than borrowed from the lowerer: this is a syntax
        // error about a missing block, and the lowerer's `semantic` helper is not
        // reachable from the parser.
        let written = name.as_str().to_string();
        let missing = |what: &str| {
            parse_err(
                format!("the arb '{written}' has no `{what}` block; a scope without it is not a strategy"),
                Tok::RBrace,
            )
        };
        Ok(Item::Arb(ArbDecl {
            name,
            discover: discover.ok_or_else(|| missing("discover"))?,
            capital: capital.ok_or_else(|| missing("capital"))?,
            execution: execution.ok_or_else(|| missing("execution"))?,
            risk: risk.ok_or_else(|| missing("risk"))?,
        }))
    }

    /// The clause name and its `=`, or a refusal naming the shape.
    ///
    /// The name is read as a *word* rather than as an identifier, because one of
    /// them — `atomic` — is a keyword token everywhere else in the language and
    /// would otherwise never reach an `execution` block.
    fn parse_arb_field(&mut self, block: &str) -> Result<String, X3Error> {
        let Some(field) = self.peek_word() else {
            return Err(parse_err(format!("a `{block}` clause"), self.peek()));
        };
        self.advance();
        if self.peek() != Tok::Eq {
            return Err(parse_err(
                format!("`{block}` clause '{field}' needs `= <value>`"),
                self.peek(),
            ));
        }
        self.advance();
        Ok(field)
    }

    /// `enabled` and `disabled` are accepted alongside `true` and `false`: the
    /// phase's own example writes `flash = enabled`, and a language that refused
    /// its own spec's spelling would be refusing nothing useful.
    fn parse_arb_flag(&mut self, field: &str) -> Result<bool, X3Error> {
        // `true` and `false` are keyword tokens of their own; `enabled` and
        // `disabled` are ordinary words. All four spell a flag, so all four are
        // read here rather than only the pair the lexer ranked.
        match self.peek() {
            Tok::KwTrue => {
                self.advance();
                Ok(true)
            }
            Tok::KwFalse => {
                self.advance();
                Ok(false)
            }
            Tok::Ident(ref word) if word.as_str() == "enabled" => {
                self.advance();
                Ok(true)
            }
            Tok::Ident(ref word) if word.as_str() == "disabled" => {
                self.advance();
                Ok(false)
            }
            Tok::Ident(ref word) => {
                let other = word.as_str().to_string();
                Err(parse_err(
                    format!(
                        "`{field}` is a flag: write `true`, `false`, `enabled` or `disabled`, not \
                         '{other}'"
                    ),
                    self.peek(),
                ))
            }
            _ => Err(parse_err(
                format!("`{field}` is a flag: write `true`, `false`, `enabled` or `disabled`"),
                self.peek(),
            )),
        }
    }

    /// `<n>bps` or `<n> bps` — a bound in basis points.
    ///
    /// The unit is **required** here, unlike `risk_policy`'s fields whose names
    /// already end in `_bps`: this block's clause is called `min_profit`, and
    /// `min_profit = 20` could as easily be twenty USDC as twenty basis points.
    /// Requiring the unit is what keeps the declaration from meaning two things.
    ///
    /// The attached spelling arrives as one word because the lexer joins a number
    /// to the word after it, so it is split here rather than refused — the phase's
    /// own example writes `20bps`.
    fn parse_arb_bps(&mut self, field: &str) -> Result<u16, X3Error> {
        let (value, consumed_attached) = match self.peek() {
            Tok::Int(value) => (value, false),
            Tok::Ident(word) => {
                let text = word.as_str().to_string();
                let Some(digits) = text.strip_suffix("bps") else {
                    return Err(parse_err(
                        format!(
                            "`{field}` is a bound in basis points: write `{field} = <n>bps`, found \
                             '{text}'"
                        ),
                        self.peek(),
                    ));
                };
                let digits = digits.trim_end_matches('_').replace('_', "");
                if digits.is_empty() || digits.contains('.') {
                    return Err(parse_err(
                        format!("`{field}` is a whole number of basis points; '{text}' is not one"),
                        self.peek(),
                    ));
                }
                let value: u128 = digits.parse().map_err(|_| {
                    parse_err(
                        format!("`{field}` is written '{text}', which has no number in it"),
                        self.peek(),
                    )
                })?;
                (value, true)
            }
            _ => {
                return Err(parse_err(
                    format!("`{field}` is a bound in basis points: write `{field} = <n>bps`"),
                    self.peek(),
                ))
            }
        };
        if !consumed_attached {
            self.advance();
            if self.peek_word().as_deref() != Some("bps") {
                return Err(parse_err(
                    format!("`{field}` is a bound in basis points: write `{field} = {value}bps`"),
                    self.peek(),
                ));
            }
            self.advance();
        } else {
            self.advance();
        }
        u16::try_from(value).map_err(|_| {
            parse_err(
                format!("`{field} = {value}bps` is larger than a basis-point bound can be"),
                self.peek(),
            )
        })
    }

    /// `<n> <ASSET>` — an amount and the asset it is denominated in.
    fn parse_arb_amount(&mut self, field: &str) -> Result<(u128, AssetRef), X3Error> {
        let amount = match self.peek() {
            Tok::Int(value) => {
                self.advance();
                value
            }
            _ => {
                return Err(parse_err(
                    format!("`{field}` is an amount: write `<n> <ASSET>`"),
                    self.peek(),
                ))
            }
        };
        let asset = self.parse_hedge_asset()?;
        Ok((amount, asset))
    }

    fn parse_arb_discover(&mut self) -> Result<ArbDiscover, X3Error> {
        let mut chains: Option<Vec<ChainRef>> = None;
        let mut max_hops: Option<u32> = None;
        let mut liquidity_min: Option<(u128, AssetRef)> = None;
        while self.peek() != Tok::RBrace && self.peek() != Tok::Eof {
            let field = self.parse_arb_field("discover")?;
            match field.as_str() {
                "chains" => {
                    if chains.is_some() {
                        return Err(parse_err("duplicate 'chains' in discover".into(), self.peek()));
                    }
                    self.expect(Tok::LBracket, "expected '[' after `chains =`")?;
                    let mut found: Vec<ChainRef> = Vec::new();
                    while self.peek() != Tok::RBracket && self.peek() != Tok::Eof {
                        found.push(self.parse_chain_ref()?);
                        if self.peek() == Tok::Comma {
                            self.advance();
                        }
                    }
                    self.expect(Tok::RBracket, "expected ']' after the chain list")?;
                    chains = Some(found);
                }
                "max_hops" => {
                    if max_hops.is_some() {
                        return Err(parse_err("duplicate 'max_hops' in discover".into(), self.peek()));
                    }
                    // A count, not an identifier: `max_hops = 4 x` would be a hop
                    // count with a unit, which is a mistake about what is being
                    // counted, so the number is read as a number and the rest is
                    // left to become an unknown clause.
                    let value = match self.peek() {
                        Tok::Int(value) => {
                            self.advance();
                            value
                        }
                        _ => {
                            return Err(parse_err(
                                "`max_hops` is a count of hops: write `max_hops = <n>`".into(),
                                self.peek(),
                            ))
                        }
                    };
                    max_hops = Some(u32::try_from(value).map_err(|_| {
                        parse_err(
                            format!("`max_hops = {value}` is larger than a hop count can be"),
                            self.peek(),
                        )
                    })?);
                }
                "liquidity_min" => {
                    if liquidity_min.is_some() {
                        return Err(parse_err("duplicate 'liquidity_min' in discover".into(), self.peek()));
                    }
                    liquidity_min = Some(self.parse_arb_amount("liquidity_min")?);
                }
                other => {
                    return Err(parse_err(
                        format!(
                            "unknown `discover` clause '{other}'; it declares `chains`, `max_hops` \
                             and `liquidity_min`"
                        ),
                        self.peek(),
                    ))
                }
            }
            self.opt_semi();
        }
        Ok(ArbDiscover {
            chains: chains.unwrap_or_default(),
            max_hops: max_hops.unwrap_or(0),
            liquidity_min,
        })
    }

    fn parse_arb_capital(&mut self) -> Result<ArbCapital, X3Error> {
        let mut flash: Option<bool> = None;
        let mut max: Option<(u128, AssetRef)> = None;
        while self.peek() != Tok::RBrace && self.peek() != Tok::Eof {
            let field = self.parse_arb_field("capital")?;
            match field.as_str() {
                "flash" => {
                    if flash.is_some() {
                        return Err(parse_err("duplicate 'flash' in capital".into(), self.peek()));
                    }
                    flash = Some(self.parse_arb_flag("flash")?);
                }
                "max" => {
                    if max.is_some() {
                        return Err(parse_err("duplicate 'max' in capital".into(), self.peek()));
                    }
                    max = Some(self.parse_arb_amount("max")?);
                }
                other => {
                    return Err(parse_err(
                        format!("unknown `capital` clause '{other}'; it declares `flash` and `max`"),
                        self.peek(),
                    ))
                }
            }
            self.opt_semi();
        }
        Ok(ArbCapital {
            flash: flash.unwrap_or(false),
            max,
        })
    }

    fn parse_arb_execution(&mut self) -> Result<ArbExecution, X3Error> {
        let mut atomic = None;
        let mut parallel = None;
        let mut private = None;
        while self.peek() != Tok::RBrace && self.peek() != Tok::Eof {
            let field = self.parse_arb_field("execution")?;
            match field.as_str() {
                "atomic" => {
                    if atomic.is_some() {
                        return Err(parse_err("duplicate 'atomic' in execution".into(), self.peek()));
                    }
                    atomic = Some(self.parse_arb_flag("atomic")?);
                }
                "parallel" => {
                    if parallel.is_some() {
                        return Err(parse_err("duplicate 'parallel' in execution".into(), self.peek()));
                    }
                    parallel = Some(self.parse_arb_flag("parallel")?);
                }
                "private" => {
                    if private.is_some() {
                        return Err(parse_err("duplicate 'private' in execution".into(), self.peek()));
                    }
                    private = Some(self.parse_arb_flag("private")?);
                }
                other => {
                    return Err(parse_err(
                        format!(
                            "unknown `execution` clause '{other}'; it declares `atomic`, `parallel` \
                             and `private`"
                        ),
                        self.peek(),
                    ))
                }
            }
            self.opt_semi();
        }
        Ok(ArbExecution {
            atomic: atomic.unwrap_or(false),
            parallel: parallel.unwrap_or(false),
            private: private.unwrap_or(false),
        })
    }

    fn parse_arb_risk(&mut self) -> Result<ArbRisk, X3Error> {
        let mut min_profit_bps = None;
        let mut max_slippage_bps = None;
        let mut max_total_fee_bps = None;
        let mut deadline = None;
        while self.peek() != Tok::RBrace && self.peek() != Tok::Eof {
            let field = self.parse_arb_field("risk")?;
            match field.as_str() {
                "min_profit" | "max_slippage" | "max_total_fee" => {
                    let value = self.parse_arb_bps(&field)?;
                    let slot = match field.as_str() {
                        "min_profit" => &mut min_profit_bps,
                        "max_slippage" => &mut max_slippage_bps,
                        _ => &mut max_total_fee_bps,
                    };
                    if slot.replace(value).is_some() {
                        return Err(parse_err(format!("duplicate '{field}' in risk"), self.peek()));
                    }
                }
                "deadline" => {
                    if deadline.is_some() {
                        return Err(parse_err("duplicate 'deadline' in risk".into(), self.peek()));
                    }
                    deadline = Some(self.parse_duration_expr("an arb deadline")?);
                }
                other => {
                    return Err(parse_err(
                        format!(
                            "unknown `risk` clause '{other}'; it declares `min_profit`, \
                             `max_slippage`, `max_total_fee` and `deadline`"
                        ),
                        self.peek(),
                    ))
                }
            }
            self.opt_semi();
        }
        Ok(ArbRisk {
            min_profit_bps,
            max_slippage_bps,
            max_total_fee_bps,
            deadline,
        })
    }

    /// `hyperarb <name> { capital = …; parallel { … } choose …; hedge volatility;
    /// settle_across_domains; require net_profit >= <n>bps; }` — spec PHASE 38.
    ///
    /// The clauses are read in any order and each may appear once: a declaration
    /// that says `choose` twice has two answers and no way to pick between them.
    /// Nothing is validated here — flash capital, a leg that resolves to nothing,
    /// a hedge with no bound and `settle_across_domains` over one domain are all
    /// decided in `compiler/src/hyperarb.rs`, where the reason can be stated with
    /// figures.
    fn parse_hyperarb_item(&mut self) -> Result<Item, X3Error> {
        self.advance();
        let name = Symbol::new(&self.expect_ident("hyperarb name")?);
        self.expect(Tok::LBrace, "expected '{' after the hyperarb name")?;

        let mut capital: Option<(u128, AssetRef)> = None;
        let mut flash = false;
        let mut legs: Option<Vec<HyperarbLeg>> = None;
        let mut choose: Option<ChoiceCriterion> = None;
        let mut hedge_volatility = false;
        let mut settle_across_domains = false;
        let mut net_profit_bps: Option<u16> = None;
        let mut seen: Vec<String> = Vec::new();

        let mut note = |seen: &mut Vec<String>, clause: &str| -> Result<(), X3Error> {
            if seen.iter().any(|name| name == clause) {
                return Err(parse_err(
                    format!("the hyperarb declares `{clause}` twice; one line would have to override the other"),
                    Tok::LBrace,
                ));
            }
            seen.push(clause.to_string());
            Ok(())
        };

        while self.peek() != Tok::RBrace && self.peek() != Tok::Eof {
            if self.peek() == Tok::KwRequire {
                self.advance();
                let field = self.expect_ident("`net_profit`")?;
                if field != "net_profit" {
                    return Err(parse_err(
                        format!(
                            "a hyperarb's floor is `require net_profit >= <n>bps`, found `require \
                             {field}`"
                        ),
                        self.peek(),
                    ));
                }
                if self.peek() != Tok::Ge && self.peek() != Tok::Gt {
                    return Err(parse_err(
                        "`require net_profit` needs a floor: write `require net_profit >= <n>bps`".into(),
                        self.peek(),
                    ));
                }
                self.advance();
                let value = self.parse_arb_bps("net_profit")?;
                note(&mut seen, "require net_profit")?;
                net_profit_bps = Some(value);
                self.opt_semi();
                continue;
            }

            let clause = self.peek_word().ok_or_else(|| {
                parse_err(
                    "expected `capital = …`, `parallel { … }`, `choose …`, `hedge volatility`, \
                     `settle_across_domains` or `require net_profit >= <n>bps`"
                        .into(),
                    self.peek(),
                )
            })?;
            match clause.as_str() {
                "capital" => {
                    note(&mut seen, "capital")?;
                    self.advance();
                    self.expect(Tok::Eq, "expected '=' after `capital`")?;
                    // `flash(…)` is read rather than rejected: the refusal has to
                    // be able to quote the amount the author wrote.
                    if self.peek_word().as_deref() == Some("flash") {
                        self.advance();
                        self.expect(Tok::LParen, "expected '(' after `flash`")?;
                        capital = Some(self.parse_arb_amount("flash")?);
                        self.expect(Tok::RParen, "expected ')' after the flash amount")?;
                        flash = true;
                    } else {
                        capital = Some(self.parse_arb_amount("capital")?);
                    }
                }
                "parallel" => {
                    note(&mut seen, "parallel")?;
                    self.advance();
                    self.expect(Tok::LBrace, "expected '{' after `parallel`")?;
                    let mut found: Vec<HyperarbLeg> = Vec::new();
                    while self.peek() != Tok::RBrace && self.peek() != Tok::Eof {
                        let leg = self.expect_ident("a parallel leg name")?;
                        self.expect(Tok::Eq, &format!("expected '=' after the leg '{leg}'"))?;
                        let verb = self.expect_ident("`evaluate`")?;
                        if verb != "evaluate" {
                            return Err(parse_err(
                                format!("a hyperarb leg is `{leg} = evaluate(<target>)`, found `{verb}`"),
                                self.peek(),
                            ));
                        }
                        self.expect(Tok::LParen, "expected '(' after `evaluate`")?;
                        let target = self.expect_ident("the path a leg evaluates")?;
                        self.expect(Tok::RParen, "expected ')' after the leg's target")?;
                        self.opt_semi();
                        found.push(HyperarbLeg {
                            name: Symbol::new(&leg),
                            target: Symbol::new(&target),
                        });
                    }
                    self.expect(Tok::RBrace, "expected '}' after the parallel legs")?;
                    legs = Some(found);
                }
                "choose" => {
                    note(&mut seen, "choose")?;
                    self.advance();
                    let wanted = self.expect_ident("a choice criterion")?;
                    choose = Some(ChoiceCriterion::parse(&wanted).ok_or_else(|| {
                        let allowed: Vec<&str> = ChoiceCriterion::ALL.iter().map(|c| c.as_str()).collect();
                        parse_err(
                            format!(
                                "unknown choice criterion '{wanted}'; a hyperarb chooses one of: {}",
                                allowed.join(", ")
                            ),
                            self.peek(),
                        )
                    })?);
                }
                "hedge" => {
                    note(&mut seen, "hedge")?;
                    self.advance();
                    let what = self.expect_ident("`volatility`")?;
                    if what != "volatility" {
                        return Err(parse_err(
                            format!(
                                "a hyperarb can `hedge volatility`, which is the exposure it names; \
                                 found `hedge {what}`"
                            ),
                            self.peek(),
                        ));
                    }
                    hedge_volatility = true;
                }
                "settle_across_domains" => {
                    note(&mut seen, "settle_across_domains")?;
                    self.advance();
                    settle_across_domains = true;
                }
                other => {
                    return Err(parse_err(
                        format!(
                            "unknown hyperarb clause '{other}'; it declares `capital`, `parallel`, \
                             `choose`, `hedge volatility`, `settle_across_domains` and `require \
                             net_profit >= <n>bps`"
                        ),
                        self.peek(),
                    ))
                }
            }
            self.opt_semi();
        }
        self.expect(Tok::RBrace, "expected '}' after the hyperarb")?;

        let written = name.as_str().to_string();
        let missing = |clause: &str| {
            parse_err(
                format!(
                    "the hyperarb '{written}' declares no `{clause}`; a declaration without it is \
                     not the primitive the phase describes"
                ),
                Tok::RBrace,
            )
        };
        Ok(Item::Hyperarb(HyperarbDecl {
            name,
            capital: capital.or_else(|| {
                Some((
                    0,
                    AssetRef::new(ChainRef::new(Symbol::new("unknown")), Symbol::new("unknown")),
                ))
            }),
            flash,
            legs: legs.ok_or_else(|| missing("parallel"))?,
            choose: choose.ok_or_else(|| missing("choose"))?,
            hedge_volatility,
            settle_across_domains,
            net_profit_bps: net_profit_bps.ok_or_else(|| missing("require net_profit"))?,
        }))
    }

    /// A metric name, read through the objective's table so the two constructs cannot
    /// disagree about which metrics exist or which way each points.
    fn parse_metric_name(&mut self, direction: &str) -> Result<ObjectiveMetric, X3Error> {
        let Some(wanted) = self.peek_word() else {
            return Err(parse_err(
                format!("expected a metric name after `{direction}`"),
                self.peek(),
            ));
        };
        self.advance();
        let found = ObjectiveMetric::by_name(&wanted).ok_or_else(|| {
            let allowed: Vec<String> = ObjectiveMetric::ALL
                .iter()
                .map(|metric| format!("{} {}", metric.direction(), metric.name()))
                .collect();
            parse_err(
                format!(
                    "unknown '{}' target '{wanted}'; the metrics the optimizer knows are: {}",
                    direction,
                    allowed.join(", ")
                ),
                self.peek(),
            )
        })?;
        if found.direction() != direction {
            return Err(parse_err(
                format!(
                    "'{wanted}' is a metric to {}, not to {direction}; the direction is part of the \
                     metric",
                    found.direction()
                ),
                self.peek(),
            ));
        }
        Ok(found)
    }

    /// `atomic_liquidation { liquidate <n> <ASSET> of <ref>; receive <n> <ASSET> collateral;
    /// swap <n> <ASSET> -> <ASSET> min_output <n>; repay <n> <ASSET>;
    /// require net_profit >= <n> <ASSET>; }`
    ///
    /// One of each clause, and every amount required: the verifier's job is to check
    /// that the swap covers the repayment and that nothing is left over, and a
    /// missing figure would leave it nothing to check (PHASE 10).
    fn parse_atomic_liquidation_item(&mut self) -> Result<Item, X3Error> {
        self.advance();
        self.expect(Tok::LBrace, "expected '{' after atomic_liquidation")?;
        let mut position: Option<Symbol> = None;
        let mut capital: Option<(u128, AssetRef)> = None;
        let mut collateral: Option<(u128, AssetRef)> = None;
        let mut swap: Option<LiquidationSwap> = None;
        let mut repaid: Option<(u128, AssetRef)> = None;
        let mut profit_floor: Option<(u128, AssetRef)> = None;

        while self.peek() != Tok::RBrace && self.peek() != Tok::Eof {
            // `require` is a keyword token; the other clauses are identifiers.
            if self.peek() == Tok::KwRequire {
                self.advance();
                let kind = self.expect_ident("require kind")?;
                if kind != "net_profit" {
                    return Err(parse_err(
                        format!(
                            "an `atomic_liquidation` guard is about the money left after repaying: \
                             write `require net_profit >= <n> <ASSET>`, not `require {kind}`"
                        ),
                        self.peek(),
                    ));
                }
                let is_floor = matches!(self.peek(), Tok::Ge | Tok::Gt);
                if !is_floor {
                    return Err(parse_err(
                        "a liquidation's net_profit guard is a floor — write `require net_profit >= <n> \
                         <ASSET>`"
                            .into(),
                        self.peek(),
                    ));
                }
                self.advance();
                let amount = self.expect_uint("net_profit floor")?;
                let asset = self.parse_hedge_asset()?;
                self.opt_semi();
                profit_floor = Some((amount, asset));
                continue;
            }

            // `swap` is a keyword token too, so it cannot come through `peek_word`.
            if self.peek() == Tok::KwSwap {
                self.advance();
                let amount = self.expect_uint("swap amount")?;
                let from = self.parse_hedge_asset()?;
                self.expect(Tok::Arrow, "expected `-> <ASSET>` in the liquidation's swap")?;
                let to = self.parse_hedge_asset()?;
                self.expect(
                    Tok::Ident("min_output".into()),
                    "expected `min_output <n>`: a liquidation that does not bound what the collateral \
                     converts to cannot be checked against the repayment",
                )?;
                let min_output = self.expect_uint("swap min_output")?;
                self.opt_semi();
                swap = Some(LiquidationSwap {
                    amount,
                    from,
                    to,
                    min_output,
                });
                continue;
            }

            let clause = self
                .peek_word()
                .ok_or_else(|| parse_err("expected a liquidation clause".into(), self.peek()))?;
            match clause.as_str() {
                "liquidate" => {
                    self.advance();
                    let amount = self.expect_uint("liquidate amount")?;
                    let asset = self.parse_hedge_asset()?;
                    self.expect(Tok::Ident("of".into()), "expected `of <position>` after the amount")?;
                    // `borrower.position`: a dotted reference, carried verbatim so the
                    // artifact names whose position was liquidated. It is not resolved
                    // against a data model — this language has none — and saying so is
                    // better than silently keeping only its first word.
                    let mut name = self.expect_ident("position reference")?;
                    while self.peek() == Tok::Dot {
                        self.advance();
                        name.push('.');
                        name.push_str(&self.expect_ident("position reference")?);
                    }
                    self.opt_semi();
                    capital = Some((amount, asset));
                    position = Some(Symbol::new(&name));
                }
                "receive" => {
                    self.advance();
                    let amount = self.expect_uint("collateral amount")?;
                    let asset = self.parse_hedge_asset()?;
                    self.expect(
                        Tok::Ident("collateral".into()),
                        "expected the word `collateral` after the amount, so what is being received is \
                         not left to the reader",
                    )?;
                    self.opt_semi();
                    collateral = Some((amount, asset));
                }
                "repay" => {
                    self.advance();
                    let amount = self.expect_uint("repay amount")?;
                    let asset = self.parse_hedge_asset()?;
                    self.opt_semi();
                    repaid = Some((amount, asset));
                }
                other => {
                    return Err(parse_err(
                        format!(
                            "expected `liquidate`, `receive`, `swap`, `repay` or a `require \
                             net_profit` guard, found '{other}'"
                        ),
                        self.peek(),
                    ))
                }
            }
        }
        self.expect(Tok::RBrace, "expected '}' after the liquidation")?;

        let missing = |what: &str| parse_err(format!("the liquidation states no {what}"), self.peek());
        Ok(Item::AtomicLiquidation(AtomicLiquidationDecl {
            position: position.ok_or_else(|| missing("`liquidate <n> <ASSET> of <position>` clause"))?,
            capital: capital.ok_or_else(|| missing("`liquidate` amount"))?,
            collateral: collateral.ok_or_else(|| missing("`receive … collateral` clause"))?,
            swap: swap.ok_or_else(|| missing("`swap … min_output …` clause"))?,
            repaid: repaid.ok_or_else(|| missing("`repay <n> <ASSET>` clause"))?,
            profit_floor,
        }))
    }

    /// `atomic_hedge { buy <n> <ASSET> spot; short equivalent <ASSET> perp; require delta <= <pct>; }`
    ///
    /// The legs are two statements about one asset inside one plan: a long and a
    /// short, on a spot or a perp venue, and the guard is the claim their net has to
    /// satisfy. The asset may be written as `chain.ASSET` or bare — a hedge nets by
    /// asset, and `hedge::verify` refuses to net two different ones.
    fn parse_atomic_hedge_item(&mut self) -> Result<Item, X3Error> {
        self.advance();
        self.expect(Tok::LBrace, "expected '{' after atomic_hedge")?;
        let mut legs = Vec::new();
        let mut delta_bound_bps = None;
        while self.peek() != Tok::RBrace && self.peek() != Tok::Eof {
            // `require` is a keyword token, so it cannot come through `peek_word`
            // (which reads identifiers) — matching the token is what makes the guard
            // part of the block rather than a parse error.
            if self.peek() == Tok::KwRequire {
                self.advance();
                let kind = self.expect_ident("require kind")?;
                if kind != "delta" {
                    return Err(parse_err(
                        format!(
                            "an `atomic_hedge` guard is about the net exposure: write `require delta \
                             <= <pct>`, not `require {kind}`"
                        ),
                        self.peek(),
                    ));
                }
                let is_ceiling = matches!(self.peek(), Tok::Le | Tok::Lt);
                if !is_ceiling {
                    return Err(parse_err(
                        "a hedge's delta guard is a ceiling — write `require delta <= <pct>`".into(),
                        self.peek(),
                    ));
                }
                self.advance();
                let value = self.parse_expr()?;
                // The unit every other basis-point clause in the language writes
                // (`50 bps`, `100bps`) is accepted here too. It was not, while this
                // construct's own formatter emitted it — so `x3c fmt` produced an
                // `atomic_hedge` the parser could not read back, and formatting a
                // program corrupted it.
                if self.peek_word().as_deref() == Some("bps") {
                    self.advance();
                }
                delta_bound_bps = Some(crate::semantic::bound_bps_from_expr(&value).ok_or_else(|| {
                    parse_err(
                        "the delta bound must be a percentage (`0.01%`), a count of basis points \
                         (`1`) or a count with its unit (`1 bps`)"
                            .into(),
                        self.peek(),
                    )
                })?);
                self.opt_semi();
                continue;
            }

            let side = match self.peek() {
                Tok::Ident(ref word) if word == "buy" => HedgeSide::Long,
                Tok::Ident(ref word) if word == "short" => HedgeSide::Short,
                _ => {
                    return Err(parse_err(
                        "expected a hedge leg (`buy … spot`, `short … perp`) or a `require delta` guard".into(),
                        self.peek(),
                    ))
                }
            };
            self.advance();
            let quantity = match self.peek() {
                Tok::Int(value) => {
                    self.advance();
                    HedgeQuantity::Amount(value)
                }
                Tok::Ident(ref word) if word == "equivalent" => {
                    self.advance();
                    HedgeQuantity::Equivalent
                }
                _ => {
                    return Err(parse_err(
                        "a hedge leg needs a size: write `<n>` or `equivalent`".into(),
                        self.peek(),
                    ))
                }
            };
            let asset = self.parse_hedge_asset()?;
            let venue = match self.peek_word().as_deref() {
                Some("spot") => HedgeVenue::Spot,
                Some("perp") => HedgeVenue::Perp,
                other => {
                    return Err(parse_err(
                        format!(
                            "a hedge leg names its venue: `spot` or `perp`, not {}",
                            other.unwrap_or("nothing")
                        ),
                        self.peek(),
                    ))
                }
            };
            self.advance();
            self.opt_semi();
            legs.push(HedgeLeg {
                side,
                quantity,
                asset,
                venue,
            });
        }
        self.expect(Tok::RBrace, "expected '}' after the hedge")?;
        Ok(Item::AtomicHedge(AtomicHedgeDecl { legs, delta_bound_bps }))
    }

    /// An integer literal, for a clause whose value has to be a number the compiler
    /// can evaluate — a hedge leg's size, a liquidation's amount. A value it would
    /// have to read at run time leaves the verifier nothing to check.
    fn expect_uint(&mut self, what: &str) -> Result<u128, X3Error> {
        match self.peek() {
            Tok::Int(value) => {
                self.advance();
                Ok(value)
            }
            _ => Err(parse_err(
                format!("expected {what} as an integer, not a value the compiler cannot evaluate"),
                self.peek(),
            )),
        }
    }

    /// An asset reference in a hedge leg: `chain.ASSET`, or a bare asset name whose
    /// chain is `unknown` — a hedge nets by asset, and two spellings of the same
    /// asset are still two different references, which `hedge::verify` reports.
    fn parse_hedge_asset(&mut self) -> Result<AssetRef, X3Error> {
        let first = self.expect_ident("hedge asset")?;
        if self.peek() == Tok::Dot {
            self.advance();
            let name = self.expect_ident("hedge asset name")?;
            return Ok(AssetRef::new(ChainRef::new(Symbol::new(&first)), Symbol::new(&name)));
        }
        Ok(AssetRef::new(
            ChainRef::new(Symbol::new("unknown")),
            Symbol::new(&first),
        ))
    }

    /// `finality_policy <name> { [chain <c>] [requirement <mode>] [blocks <n>] }`
    fn parse_finality_policy_item(&mut self) -> Result<Item, X3Error> {
        self.advance();
        let mode = Symbol::new(&self.expect_ident("finality_policy mode")?);
        self.expect(Tok::LBrace, "expected '{' after finality_policy")?;
        let mut chain = Symbol::new("unknown");
        let mut requirement = Symbol::new("finalized");
        let mut blocks: Option<u32> = None;
        while self.peek() != Tok::RBrace && self.peek() != Tok::Eof {
            match self.peek() {
                // The clause form the examples are written in:
                // `chain <name>` and `requirement <mode>`. Reading these as the
                // terse form below is what made `finality_policy strict { chain
                // ethereum requirement finalized }` store the word
                // "requirement" as the chain and lose `ethereum` — which then
                // became the *subject* of the finality requirement in the IR.
                Tok::Ident(ref s) if s == "chain" => {
                    self.advance();
                    chain = Symbol::new(&self.expect_ident("finality chain name")?);
                }
                // `requirement` only: the `require` half of this guard was a second word the arm
                // could never see, because `require` reaches the parser as `KwRequire`. The form
                // that spells it that way is the terse one below, which matches the keyword.
                Tok::Ident(ref s) if s == "requirement" => {
                    self.advance();
                    requirement = Symbol::new(&self.expect_ident("finality requirement")?);
                }
                // The depth clause, and it has to be matched *before* the terse
                // form below: `blocks 32` is a chain name followed by a number to
                // that arm, so the depth would be read as a chain called
                // `blocks` and the number left where the next clause begins.
                Tok::Ident(ref s) if s == "blocks" => {
                    self.advance();
                    blocks = Some(self.parse_finality_blocks()?);
                }
                // The terse form: `<chain_name> require <mode>`.
                Tok::Ident(_) => {
                    chain = Symbol::new(&self.expect_ident("finality chain name")?);
                    // `require` is a lexer keyword, so it arrives as `KwRequire`; the identifier
                    // form this used to test could never match and the terse form —
                    // `ethereum require finalized`, which this comment documents — failed with
                    // "expected '}' after finality_policy body" (TICKET-046's class).
                    if matches!(self.peek(), Tok::KwRequire) {
                        self.advance();
                        requirement = Symbol::new(&self.expect_ident("finality requirement")?);
                    }
                }
                _ => break,
            }
        }
        self.expect(Tok::RBrace, "expected '}' after finality_policy body")?;
        Ok(Item::FinalityPolicy(FinalityPolicy {
            mode,
            chain,
            requirement,
            blocks,
        }))
    }

    /// `blocks <n>` inside `finality_policy`.
    ///
    /// The count is a depth in blocks and the field's own name fixes the unit,
    /// the same rule `constraints { finality <= 64 blocks }` follows: the unit
    /// may be written (`blocks 32 blocks`) and is then checked rather than
    /// stepped over, because a word that is not the unit would otherwise be left
    /// where the next clause begins.
    fn parse_finality_blocks(&mut self) -> Result<u32, X3Error> {
        let expr = self.parse_expr()?;
        let value = expr_to_u128(&expr).map_err(|_| {
            parse_err(
                "finality_policy `blocks` must be an integer literal the compiler can evaluate; a \
                 depth read at run time is a depth no guard can be checked against"
                    .into(),
                self.peek(),
            )
        })?;
        if matches!(self.peek(), Tok::Ident(ref unit) if unit == "blocks") {
            self.advance();
        }
        // Zero is not a depth: `blocks 0` would be a policy that requires the
        // chain to be no blocks deep, which states nothing, and the artifact
        // carries "no depth stated" as a zero operand — so the two would be the
        // same number (TICKET-059).
        if value == 0 {
            return Err(parse_err(
                "finality_policy `blocks 0` states no depth; omit the clause when the policy states \
                 none, so that a missing depth and a zero one cannot be read as each other"
                    .into(),
                self.peek(),
            ));
        }
        // The artifact carries the depth in a `u16` operand, so the bound is the
        // format's rather than a preference.
        u32::try_from(value)
            .ok()
            .filter(|value| *value <= u32::from(u16::MAX))
            .ok_or_else(|| {
                parse_err(
                    format!(
                        "finality_policy `blocks` {value} is above the largest depth the artifact can \
                     carry ({})",
                        u16::MAX
                    ),
                    self.peek(),
                )
            })
    }

    /// `error <name>`
    fn parse_error_decl_item(&mut self) -> Result<Item, X3Error> {
        self.advance();
        let name = Symbol::new(&self.expect_ident("error name")?);
        Ok(Item::ErrorDecl(ErrorDecl { name }))
    }

    // ------------------------------------------------------------------
    // Helpers
    // ------------------------------------------------------------------

    fn parse_fn_into_struct(&mut self) -> Result<Function, X3Error> {
        self.advance(); // 'fn'
        let name = self.expect_ident("function name")?;
        let generics = self.parse_optional_generics()?;
        let params = self.parse_param_list()?;
        let ret = self.parse_optional_ret_type()?;
        let body = self.parse_block()?;
        Ok(Function {
            name: Symbol::new(&name),
            id: None,
            params,
            ret,
            generics,
            body,
            visibility: Visibility::Pub,
            is_async: false,
            annotations: vec![],
        })
    }

    fn parse_strategy_decl(&mut self) -> Result<StrategyDecl, X3Error> {
        self.advance(); // 'strategy'
        let name = self.expect_ident("strategy name")?;
        let params = self.parse_param_list()?;
        let body = self.parse_block()?;
        Ok(StrategyDecl {
            name: Symbol::new(&name),
            id: None,
            params,
            body,
            is_async: false,
        })
    }

    fn parse_annotations(&mut self) -> Result<Vec<Annotation>, X3Error> {
        let mut annots = Vec::new();
        while self.peek() == Tok::At {
            self.advance();
            annots.push(self.parse_single_annotation()?);
        }
        Ok(annots)
    }

    fn parse_single_annotation(&mut self) -> Result<Annotation, X3Error> {
        // A word the lexer reserves cannot name an annotation, because `expect_ident` refuses a keyword
        // token — and its message ("annotation name: expected identifier") told an author their *syntax*
        // was wrong when the fact is about the word. `subscription` is the one such word today, and it
        // names an item (TICKET-111).
        let name = match self.peek() {
            Tok::Ident(_) => self.expect_ident("annotation name")?,
            Tok::KwSubscription => {
                return Err(parse_err(
                    "`@subscription` is not an annotation: `subscription` begins a `subscription \
                     <name>: <amount>, <period> { … }` item, which is where a subscription's work is \
                     written — a keyword cannot name an annotation"
                        .to_string(),
                    self.peek(),
                ))
            }
            other => {
                return Err(parse_err(
                    format!("an annotation is named by an identifier, and `{other:?}` is not one"),
                    other,
                ))
            }
        };
        let args: Vec<Expression> = if self.peek() == Tok::LParen {
            self.advance();
            let exprs = self.parse_expr_list()?;
            self.expect(Tok::RParen, "expected ')'")?;
            exprs
        } else {
            vec![]
        };
        annotation_from_name_args(&name, &args)
    }

    fn parse_param_list(&mut self) -> Result<Vec<Parameter>, X3Error> {
        self.expect(Tok::LParen, "expected '('")?;
        let mut params = Vec::new();
        if self.peek() == Tok::RParen {
            self.advance();
            return Ok(params);
        }
        loop {
            let is_mut = if self.peek() == Tok::KwMut {
                self.advance();
                true
            } else {
                false
            };
            let name = if matches!(self.peek(), Tok::Ident(_)) {
                Some(Symbol::new(&self.expect_ident("param name")?))
            } else {
                None
            };
            let ty = if self.peek() == Tok::Colon {
                self.advance();
                Some(self.parse_type()?)
            } else {
                None
            };
            params.push(Parameter { name, ty, is_mut });
            if self.peek() == Tok::Comma {
                self.advance();
                continue;
            }
            break;
        }
        self.expect(Tok::RParen, "expected ')'")?;
        Ok(params)
    }

    fn parse_optional_generics(&mut self) -> Result<Vec<GenericParam>, X3Error> {
        if self.peek() != Tok::Lt {
            return Ok(vec![]);
        }
        self.advance();
        let mut g = Vec::new();
        loop {
            let name = Symbol::new(&self.expect_ident("generic param")?);
            let mut bounds = Vec::new();
            if self.peek() == Tok::Colon {
                self.advance();
                loop {
                    bounds.push(self.parse_type()?);
                    if self.peek() == Tok::Plus {
                        self.advance();
                        continue;
                    }
                    break;
                }
            }
            g.push(GenericParam { name, bounds });
            if self.peek() == Tok::Comma {
                self.advance();
                continue;
            }
            break;
        }
        self.expect(Tok::Gt, "expected '>'")?;
        Ok(g)
    }

    fn parse_optional_ret_type(&mut self) -> Result<Option<TypeExpr>, X3Error> {
        if self.peek() == Tok::Arrow {
            self.advance();
            Ok(Some(self.parse_type()?))
        } else {
            Ok(None)
        }
    }

    fn parse_block(&mut self) -> Result<Block, X3Error> {
        self.expect(Tok::LBrace, "expected '{'")?;
        let mut stmts = Vec::new();
        while self.peek() != Tok::RBrace && self.peek() != Tok::Eof {
            stmts.push(self.parse_statement()?);
        }
        self.expect(Tok::RBrace, "expected '}'")?;
        Ok(Block::new(stmts))
    }

    fn parse_statement(&mut self) -> Result<Statement, X3Error> {
        match self.peek() {
            Tok::KwLet => self.parse_let_stmt(),
            Tok::KwReturn => {
                self.advance();
                let expr = if self.peek() == Tok::Semicolon || self.peek() == Tok::RBrace {
                    None
                } else {
                    Some(self.parse_expr()?)
                };
                self.opt_semi();
                Ok(Statement::Return(expr))
            }
            Tok::KwBreak => {
                self.advance();
                self.opt_semi();
                Ok(Statement::Break)
            }
            Tok::KwContinue => {
                self.advance();
                self.opt_semi();
                Ok(Statement::Continue)
            }
            Tok::KwIf => {
                self.advance();
                let cond = self.parse_expr()?;
                let then_block = self.parse_block()?;
                let else_block = if self.peek() == Tok::KwElse {
                    self.advance();
                    if self.peek() == Tok::KwIf {
                        // else if -> wrap as block with single if statement
                        let inner = self.parse_statement()?;
                        Some(Block::new(vec![inner]))
                    } else {
                        Some(self.parse_block()?)
                    }
                } else {
                    None
                };
                Ok(Statement::If {
                    cond,
                    then_block,
                    else_block,
                })
            }
            Tok::KwWhile => {
                self.advance();
                let cond = self.parse_expr()?;
                let body = self.parse_block()?;
                Ok(Statement::While { cond, body })
            }
            Tok::KwFor => {
                self.advance();
                let pattern = self.parse_pattern()?;
                self.expect(Tok::KwIn, "expected 'in'")?;
                let iterable = self.parse_expr()?;
                let body = self.parse_block()?;
                Ok(Statement::For {
                    pattern,
                    iterable,
                    body,
                })
            }
            Tok::KwLoop => {
                self.advance();
                let body = self.parse_block()?;
                Ok(Statement::Loop(body))
            }
            Tok::KwAtomic => {
                self.advance();
                let meta = if self.peek() == Tok::LParen {
                    self.advance();
                    let expr = self.parse_expr()?;
                    self.expect(Tok::RParen, "expected ')'")?;
                    Some(expr)
                } else {
                    None
                };
                let body = self.parse_block()?;
                Ok(Statement::Atomic(AtomicBlock { meta, body }))
            }
            Tok::KwEmit => {
                self.advance();
                let name = self.expect_ident("event name")?;
                let mut payload = Vec::new();
                if self.peek() == Tok::LParen {
                    self.advance();
                    payload = self.parse_expr_list()?;
                    self.expect(Tok::RParen, "expected ')'")?;
                }
                self.opt_semi();
                Ok(Statement::Emit(EventEmit {
                    name: Symbol::new(&name),
                    payload,
                }))
            }
            // Cross-chain statements
            Tok::KwLock => {
                self.advance();
                let chain = self.parse_chain_ref()?;
                self.expect(Tok::Dot, "expected '.' in chain.asset")?;
                let asset_name = self.expect_ident("asset name")?;
                self.expect_ident("amount")?;
                let amount = self.parse_expr()?;
                self.expect_ident("from")?;
                let from = self.parse_expr()?;
                self.opt_semi();
                Ok(Statement::Lock {
                    asset: AssetRef::new(chain.clone(), Symbol::new(&asset_name)),
                    chain,
                    amount,
                    from,
                })
            }
            Tok::KwMint => {
                self.advance();
                let asset = self.parse_asset_ref()?;
                self.expect_ident("amount")?;
                let amount = self.parse_expr()?;
                self.expect_ident("to")?;
                let to = self.parse_expr()?;
                self.opt_semi();
                Ok(Statement::Mint { asset, amount, to })
            }
            Tok::KwBurn => {
                self.advance();
                let asset = self.parse_asset_ref()?;
                self.expect_ident("amount")?;
                let amount = self.parse_expr()?;
                self.expect_ident("from")?;
                let from = self.parse_expr()?;
                self.opt_semi();
                Ok(Statement::Burn { asset, amount, from })
            }
            Tok::KwRelease => {
                self.advance();
                let chain = self.parse_chain_ref()?;
                self.expect(Tok::Dot, "expected '.'")?;
                let asset_name = self.expect_ident("asset name")?;
                self.expect_ident("to")?;
                let to = self.parse_expr()?;
                self.opt_semi();
                Ok(Statement::Release {
                    chain: chain.clone(),
                    asset: AssetRef::new(chain, Symbol::new(&asset_name)),
                    to,
                })
            }
            Tok::KwSwap => {
                self.advance();
                let from = self.parse_asset_ref()?;
                // A body-level swap has no `amount` clause: the arrow between the
                // assets is all that is read here, and the amount comes from the
                // matching endpoint (the same rule `bridge` follows below).
                if self.peek() == Tok::Arrow {
                    self.advance();
                }
                let to = self.parse_asset_ref()?;
                let dex = if matches!(self.peek(), Tok::Ident(ref s) if s == "dex") {
                    self.advance();
                    Some(self.parse_expr()?)
                } else {
                    None
                };
                let min_output = if matches!(self.peek(), Tok::Ident(ref s) if s == "min_output") {
                    self.advance();
                    Some(self.parse_expr()?)
                } else {
                    None
                };
                self.opt_semi();
                Ok(Statement::Swap {
                    from,
                    to,
                    amount: None,
                    min_output,
                    dex,
                })
            }
            Tok::KwRequire => Ok(self.parse_require_stmt()?),
            Tok::KwOnFail => {
                self.advance();
                let action = self.parse_failure_action()?;
                self.opt_semi();
                Ok(Statement::OnFail(action))
            }
            Tok::KwOnTimeout => {
                self.advance();
                let duration = self.parse_expr()?;
                let action = self.parse_failure_action()?;
                self.opt_semi();
                Ok(Statement::OnTimeout { duration, action })
            }
            Tok::KwMatch => {
                self.advance();
                let expr = self.parse_expr()?;
                self.expect(Tok::LBrace, "expected '{'")?;
                let mut arms = Vec::new();
                while self.peek() != Tok::RBrace && self.peek() != Tok::Eof {
                    let pattern = self.parse_pattern()?;
                    self.expect(Tok::FatArrow, "expected '=>'")?;
                    let body = self.parse_expr()?;
                    arms.push((pattern, body));
                    if self.peek() == Tok::Comma {
                        self.advance();
                    }
                }
                self.expect(Tok::RBrace, "expected '}'")?;
                Ok(Statement::Expr(Expression::Match {
                    expr: Box::new(expr),
                    arms,
                }))
            }
            Tok::KwTry => {
                self.advance();
                let expr = self.parse_expr()?;
                self.opt_semi();
                Ok(Statement::Expr(Expression::Try(Box::new(expr))))
            }
            Tok::KwAwait => {
                self.advance();
                let expr = self.parse_expr()?;
                self.opt_semi();
                Ok(Statement::Expr(Expression::Await(Box::new(expr))))
            }
            Tok::KwAsync => {
                self.advance();
                let expr = self.parse_expr()?;
                self.opt_semi();
                Ok(Statement::Expr(Expression::Async(Box::new(expr))))
            }
            _ => {
                // Expression statement
                let expr = self.parse_expr()?;
                // Check for capability short-form like `snapshot`, `diff(...)`, etc.
                if let Expression::Call { callee, args } = &expr {
                    if let Expression::Ident(sym) = callee.as_ref() {
                        let name = sym.as_str();
                        let cap = capability_from_call(name, args);
                        if !matches!(cap, Statement::Expr(_)) {
                            self.opt_semi();
                            return Ok(cap);
                        }
                    }
                }
                self.opt_semi();
                Ok(Statement::Expr(expr))
            }
        }
    }

    fn parse_let_stmt(&mut self) -> Result<Statement, X3Error> {
        self.advance(); // 'let'
        let is_mut = if self.peek() == Tok::KwMut {
            self.advance();
            true
        } else {
            false
        };
        let name = self.expect_ident("variable name")?;
        let ty = if self.peek() == Tok::Colon {
            self.advance();
            Some(self.parse_type()?)
        } else {
            None
        };
        let expr = if self.peek() == Tok::Eq {
            self.advance();
            Some(self.parse_expr()?)
        } else {
            None
        };
        self.expect(Tok::Semicolon, "expected ';' after let")?;
        Ok(Statement::Let {
            name: Symbol::new(&name),
            ty,
            expr,
            is_mut,
        })
    }

    // ------------------------------------------------------------------
    // Expressions (pratt-style)
    // ------------------------------------------------------------------

    fn parse_expr(&mut self) -> Result<Expression, X3Error> {
        self.parse_binary(0)
    }

    fn parse_binary(&mut self, min_prec: u8) -> Result<Expression, X3Error> {
        let mut lhs = self.parse_unary()?;
        loop {
            let op = match self.peek() {
                Tok::PipePipe => (1, BinOp::OrOr),
                Tok::AmpAmp => (2, BinOp::AndAnd),
                Tok::EqEq => (3, BinOp::EqEq),
                Tok::Ne => (3, BinOp::Ne),
                Tok::Lt => (4, BinOp::Lt),
                Tok::Gt => (4, BinOp::Gt),
                Tok::Le => (4, BinOp::Le),
                Tok::Ge => (4, BinOp::Ge),
                Tok::Plus => (5, BinOp::Plus),
                Tok::Minus => (5, BinOp::Minus),
                Tok::Star => (6, BinOp::Star),
                Tok::Slash => (6, BinOp::Slash),
                Tok::Percent => (6, BinOp::Percent),
                _ => break,
            };
            if op.0 < min_prec {
                break;
            }
            self.advance();
            let rhs = self.parse_binary(op.0 + 1)?;
            lhs = Expression::Binary {
                op: op.1,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            };
        }
        Ok(lhs)
    }

    fn parse_unary(&mut self) -> Result<Expression, X3Error> {
        match self.peek() {
            Tok::Minus => {
                self.advance();
                Ok(Expression::Unary {
                    op: UnOp::Neg,
                    expr: Box::new(self.parse_unary()?),
                })
            }
            Tok::Bang => {
                self.advance();
                Ok(Expression::Unary {
                    op: UnOp::Not,
                    expr: Box::new(self.parse_unary()?),
                })
            }
            _ => self.parse_primary_expr(),
        }
    }

    fn parse_primary_expr(&mut self) -> Result<Expression, X3Error> {
        let mut expr = match self.advance() {
            Tok::Int(v) => {
                // Check for percentage literal: Int.Dot.Int.Percent or Int.Percent
                //
                // Only the dotted form is implemented here, and that is deliberate rather
                // than the omission it looks like: `5 % 6` is a modulo between two integer
                // literals and `compiler/tests/test_parser_coverage.rs` exercises it, so
                // claiming every `Int` followed by `%` as a percentage would take a real
                // expression away. A whole-number percent — `1%`, which is the most
                // natural way to write one percent — is read where the language documents
                // the percent spelling, in a guard's bound: `parse_guard_bound`
                // (TICKET-089).
                if matches!(self.peek(), Tok::Dot)
                    && matches!(self.peek_n(1), Tok::Int(_))
                    && matches!(self.peek_n(2), Tok::Percent)
                {
                    let frac_val = match self.peek_n(1) {
                        Tok::Int(n) => n,
                        _ => unreachable!(),
                    };
                    self.advance(); // Dot
                    self.advance(); // Int(frac)
                    self.advance(); // Percent
                    Expression::Literal(LiteralExpr::Percentage {
                        value: Symbol::new(&format!("{}.{}%", v, frac_val)),
                    })
                } else {
                    Expression::Literal(LiteralExpr::Int {
                        value: v,
                        base: IntBase::Decimal,
                        suffix: None,
                    })
                }
            }
            Tok::Float(raw) => {
                if self.peek() == Tok::Percent {
                    self.advance();
                    Expression::Literal(LiteralExpr::Percentage {
                        value: Symbol::new(&format!("{raw}%")),
                    })
                } else {
                    Expression::Literal(LiteralExpr::Float { raw, suffix: None })
                }
            }
            Tok::String_(s) => Expression::Literal(LiteralExpr::String(Symbol::new(&s))),
            Tok::KwTrue => Expression::Literal(LiteralExpr::Bool(true)),
            Tok::KwFalse => Expression::Literal(LiteralExpr::Bool(false)),
            Tok::Ident(name) => Expression::Ident(Symbol::new(&name)),
            Tok::LParen => {
                let inner = self.parse_expr()?;
                self.expect(Tok::RParen, "expected ')'")?;
                inner
            }
            Tok::LBrace => {
                // Block expression
                let mut stmts = Vec::new();
                while self.peek() != Tok::RBrace && self.peek() != Tok::Eof {
                    stmts.push(self.parse_statement()?);
                }
                self.expect(Tok::RBrace, "expected '}'")?;
                Expression::BlockExpr(Block::new(stmts))
            }
            Tok::PipePipe => {
                // Closure: || body or |a, b| body
                let mut params = Vec::new();
                if self.peek() != Tok::PipePipe {
                    loop {
                        let name = self.expect_ident("closure param")?;
                        params.push(Parameter {
                            name: Some(Symbol::new(&name)),
                            ty: None,
                            is_mut: false,
                        });
                        if self.peek() == Tok::Comma {
                            self.advance();
                            continue;
                        }
                        break;
                    }
                }
                self.expect(Tok::PipePipe, "expected '|'")?;
                let body = self.parse_expr()?;
                Expression::Closure {
                    params,
                    body: Box::new(body),
                    is_async: false,
                }
            }
            found => return Err(parse_err("expected expression".into(), found)),
        };

        // Postfix: call, method, field, index
        loop {
            match self.peek() {
                Tok::LParen => {
                    self.advance();
                    let args = self.parse_expr_list()?;
                    self.expect(Tok::RParen, "expected ')'")?;
                    expr = Expression::Call {
                        callee: Box::new(expr),
                        args,
                    };
                }
                Tok::Dot => {
                    self.advance();
                    let field = self.expect_ident("field name")?;
                    // Check if followed by '(' -> method call
                    if self.peek() == Tok::LParen {
                        self.advance();
                        let args = self.parse_expr_list()?;
                        self.expect(Tok::RParen, "expected ')'")?;
                        expr = Expression::MethodCall {
                            receiver: Box::new(expr),
                            method: Symbol::new(&field),
                            args,
                        };
                    } else {
                        expr = Expression::FieldAccess {
                            target: Box::new(expr),
                            field: Symbol::new(&field),
                        };
                    }
                }
                Tok::LBracket => {
                    self.advance();
                    let index = self.parse_expr()?;
                    self.expect(Tok::RBracket, "expected ']'")?;
                    expr = Expression::Index {
                        target: Box::new(expr),
                        index: Box::new(index),
                    };
                }
                _ => break,
            }
        }
        Ok(expr)
    }

    fn parse_expr_list(&mut self) -> Result<Vec<Expression>, X3Error> {
        let mut exprs = Vec::new();
        if self.peek() == Tok::RParen || self.peek() == Tok::RBracket {
            return Ok(exprs);
        }
        loop {
            if matches!(self.peek(), Tok::Ident(_)) && self.peek_n(1) == Tok::Eq {
                self.advance();
                self.advance();
            }
            exprs.push(self.parse_expr()?);
            if self.peek() == Tok::Comma {
                self.advance();
                continue;
            }
            break;
        }
        Ok(exprs)
    }

    // ------------------------------------------------------------------
    // Types
    // ------------------------------------------------------------------

    fn parse_type(&mut self) -> Result<TypeExpr, X3Error> {
        match self.peek() {
            Tok::Ident(name) => {
                self.advance();
                let mut path = vec![Symbol::new(&name)];
                while self.peek() == Tok::Colon {
                    self.advance();
                    self.advance(); // second ':'
                    path.push(Symbol::new(&self.expect_ident("type path")?));
                }
                let base = if path.len() == 1 {
                    TypeExpr::Path(path)
                } else {
                    // multi-segment
                    TypeExpr::Path(path)
                };
                // Optional generic args
                if self.peek() == Tok::Lt {
                    self.advance();
                    let mut args = Vec::new();
                    loop {
                        args.push(self.parse_type()?);
                        if self.peek() == Tok::Comma {
                            self.advance();
                            continue;
                        }
                        break;
                    }
                    self.expect(Tok::Gt, "expected '>'")?;
                    Ok(TypeExpr::Generic {
                        base: Box::new(base),
                        args,
                    })
                } else {
                    Ok(base)
                }
            }
            Tok::LBracket => {
                self.advance();
                let inner = self.parse_type()?;
                let size = if self.peek() == Tok::Semicolon {
                    self.advance();
                    if let Tok::Int(n) = self.advance() {
                        Some(n as usize)
                    } else {
                        return Err(parse_err("expected array size".into(), self.peek()));
                    }
                } else {
                    None
                };
                self.expect(Tok::RBracket, "expected ']'")?;
                Ok(TypeExpr::Array(Box::new(inner), size))
            }
            Tok::LParen => {
                self.advance();
                let mut types = Vec::new();
                if self.peek() != Tok::RParen {
                    loop {
                        types.push(self.parse_type()?);
                        if self.peek() == Tok::Comma {
                            self.advance();
                            continue;
                        }
                        break;
                    }
                }
                self.expect(Tok::RParen, "expected ')'")?;
                Ok(TypeExpr::Tuple(types))
            }
            Tok::KwFn => {
                self.advance();
                self.expect(Tok::LParen, "expected '('")?;
                let mut params = Vec::new();
                if self.peek() != Tok::RParen {
                    loop {
                        params.push(self.parse_type()?);
                        if self.peek() == Tok::Comma {
                            self.advance();
                            continue;
                        }
                        break;
                    }
                }
                self.expect(Tok::RParen, "expected ')'")?;
                self.expect(Tok::Arrow, "expected '->' in function type")?;
                let ret = self.parse_type()?;
                Ok(TypeExpr::Func {
                    params,
                    ret: Box::new(ret),
                })
            }
            _ => Err(parse_err("expected type".into(), self.peek())),
        }
    }

    // ------------------------------------------------------------------
    // Special parsers
    // ------------------------------------------------------------------

    fn parse_asset_ref(&mut self) -> Result<AssetRef, X3Error> {
        let chain = self.parse_chain_ref()?;
        self.expect(Tok::Dot, "expected '.' in asset ref")?;
        let name = self.expect_ident("asset name")?;
        Ok(AssetRef::new(chain, Symbol::new(&name)))
    }

    fn parse_chain_ref(&mut self) -> Result<ChainRef, X3Error> {
        let name = self.expect_ident("chain name")?;
        Ok(ChainRef(Symbol::new(&name)))
    }
    /// `require <kind> [<subject>] [<cmp> <expr>] [<expr>]`.
    ///
    /// The grammar has six shapes, and the ambiguity between them is decided by
    /// looking at what follows, never by guessing: a bare `<kind>` which is a
    /// property on its own, `<kind> <name>` which is a property *of* that name,
    /// `<kind> <expr>` which is a threshold, an explicit `<kind>.<subject>`, and
    /// the two comparison forms `<kind> <subject> <cmp> <expr>` and
    /// `<kind> <expr>`. The shapes are named rather than laid out as a block on
    /// purpose: an indented block in a doc comment is a doctest, and a grammar
    /// sketch is not Rust.
    ///
    /// The two-token form is the one that cannot be told apart by looking at the
    /// first token alone, so it is decided by what the token *is*: a name is the
    /// only thing that can be a subject, so `require canonical_supply USDC` is
    /// the canonical supply of USDC and `require slippage 50` is a threshold of
    /// 50. `require proof verified` used to be unparseable — the parser read
    /// `verified` as the subject and then demanded a value that the program
    /// never wrote (TICKET-045).
    /// A guard's right-hand side, where a whole-number percent is read.
    ///
    /// `0.5%` is one `Percentage` token and reaches the literal reader; `1%` is an integer
    /// followed by `%`, and the literal reader deliberately leaves that to the modulo
    /// operator because `5 % 6` is a real expression. A guard's bound is where the language
    /// documents the percent spelling, so it is claimed here — and nowhere else
    /// (TICKET-089).
    ///
    /// Without this, `require slippage <= 1%` left the `%` to the expression parser, which
    /// read it as a **modulo** and consumed the next token as its right-hand operand: in a
    /// guard that token is the next clause, so `require slippage <= 1%` followed by
    /// `timeout 45s …` was refused with `unexpected clause in intent body: Ident("45s")` —
    /// naming a line that was correct. It survived because a percent guard written *last*
    /// in a body has no following clause to swallow, which is where every passing example
    /// and fixture happens to put its own.
    fn parse_guard_bound(&mut self) -> Result<Expression, X3Error> {
        if let Tok::Int(value) = self.peek() {
            if self.peek_n(1) == Tok::Percent {
                self.advance(); // the integer
                self.advance(); // the '%'
                return Ok(Expression::Literal(LiteralExpr::Percentage {
                    value: Symbol::new(&format!("{value}%")),
                }));
            }
        }
        self.parse_expr()
    }

    fn parse_require_guard(&mut self) -> Result<RequireGuard, X3Error> {
        self.advance(); // 'require'
        let ident = self.expect_ident("require kind")?;
        // Special-case source_finality and dest_finality: kind=Finality, subject=ident
        let (kind, mut subject) = match ident.as_str() {
            "source_finality" => (RequireKind::Finality, Some(Symbol::new("source_finality"))),
            "dest_finality" => (RequireKind::Finality, Some(Symbol::new("dest_finality"))),
            _ => (require_kind_from_str(&ident)?, None),
        };
        // `require finality.sol == finalized` — the subject written explicitly,
        // which is what makes it unambiguous when no comparison follows.
        if subject.is_none() && self.peek() == Tok::Dot {
            self.advance();
            subject = Some(Symbol::new(&self.expect_ident("require subject after '.'")?));
        }
        // `require finality Ethereum >= 64` — a bare name in front of a
        // comparison is a subject, which the one-token lookahead settles.
        if subject.is_none() && matches!(self.peek(), Tok::Ident(_)) && self.next_is_comparison() {
            subject = Some(Symbol::new(&self.expect_ident("require subject")?));
        }
        // The comparison is part of the guard, not punctuation to step over:
        // `slippage <= 50` and `slippage >= 50` are opposite claims, and a check
        // that reads one as the other is reading a direction nobody wrote.
        let comparison = match self.peek() {
            Tok::Ge => Some(ComparisonOp::GreaterOrEqual),
            Tok::Gt => Some(ComparisonOp::Greater),
            Tok::Le => Some(ComparisonOp::LessOrEqual),
            Tok::Lt => Some(ComparisonOp::Less),
            Tok::EqEq => Some(ComparisonOp::Equal),
            Tok::Ne => Some(ComparisonOp::NotEqual),
            _ => None,
        };
        if comparison.is_some() {
            self.advance();
        }
        let value = if comparison.is_some() {
            // A comparison with no right-hand side is not a comparison.
            Some(self.parse_guard_bound()?)
        } else if subject.is_some() {
            // An explicit subject may stand alone (`require canonical_supply.USDC`).
            if self.can_start_expression() && !self.next_word_begins_a_clause() {
                Some(self.parse_expr()?)
            } else {
                None
            }
        } else if !self.can_start_expression() || self.next_word_begins_a_clause() {
            // Either nothing follows, or what follows begins the next clause:
            // `require proof_complete` then `amount 500` is a property guard and
            // an amount, not a guard about an amount.
            None
        } else {
            let first = self.parse_expr()?;
            match first {
                // A name, then either the block's end or the next clause: the
                // name is what the guard is about. A name followed by something
                // that can begin an expression is that expression's subject:
                // `require nonce unused <id>`.
                Expression::Ident(name) => {
                    if self.can_start_expression() && !self.next_word_begins_a_clause() {
                        let value = self.parse_expr()?;
                        subject = Some(name);
                        Some(value)
                    } else {
                        subject = Some(name);
                        None
                    }
                }
                // Anything else is a value: `require slippage 50`.
                other => Some(other),
            }
        };
        self.opt_semi();
        if value.is_none() && subject.is_none() {
            // `require mainnet_safe` asserts a property of the program and needs
            // nothing else. `require slippage` asserts nothing at all: a bound
            // with no bound in it.
            if !kind.asserts_a_property() {
                return Err(parse_err(
                    format!(
                        "`require {ident}` states no bound; write one (`require {ident} <= 50`) or, if \
                         this is a property rather than a bound, use the property's own name"
                    ),
                    self.peek(),
                ));
            }
        }
        if !kind.asserts_a_property() && value.is_none() {
            return Err(parse_err(
                format!(
                    "`require {ident}` is a bound and has no value to compare against; a check that \
                     reads it would find nothing where it expected a number, so write `require \
                     {ident}{} <value>`",
                    subject
                        .as_ref()
                        .map(|subject| format!(".{}", subject.as_str()))
                        .unwrap_or_default()
                ),
                self.peek(),
            ));
        }
        Ok(RequireGuard {
            kind,
            subject,
            comparison,
            value,
        })
    }

    /// Whether the token after the cursor is a comparison operator.
    fn next_is_comparison(&self) -> bool {
        matches!(
            self.peek_n(1),
            Tok::Ge | Tok::Gt | Tok::Le | Tok::Lt | Tok::EqEq | Tok::Ne
        )
    }

    /// Whether the word at the cursor begins a clause rather than continuing the
    /// guard being read.
    ///
    /// A guard is the last thing read before a block ends or another clause
    /// begins, and a block's clause words are identifiers rather than keywords
    /// (`amount`, `receiver`, `to`, …), so a guard that stops at the wrong place
    /// eats the next clause. Measured: `require proof_complete` followed by
    /// `amount 500` used to become a guard *about* `amount` with the number left
    /// over as a statement — it parsed, it lowered, and the swap had no amount.
    ///
    /// The list is the union of the clause words a guard can be followed by.
    /// It is a list because the parser's clause dispatch is per-block rather
    /// than shared and the words are not keywords in the lexer; a word here that
    /// a program meant as a guard's *value* is refused loudly, since the value
    /// position is then empty and the word starts a clause the block may not
    /// allow.
    fn next_word_begins_a_clause(&self) -> bool {
        match self.peek() {
            Tok::Ident(ref word) => CLAUSE_WORDS.contains(&word.as_str()),
            _ => false,
        }
    }

    /// Whether the token at the cursor can begin an expression.
    ///
    /// Used only to decide whether a guard has a right-hand side or a second
    /// name at all: a clause terminator, a keyword, or the end of the file
    /// cannot begin one.
    fn can_start_expression(&self) -> bool {
        matches!(
            self.peek(),
            Tok::Int(_)
                | Tok::Float(_)
                | Tok::String_(_)
                | Tok::Ident(_)
                | Tok::LParen
                | Tok::Minus
                | Tok::KwTrue
                | Tok::KwFalse
        )
    }

    fn parse_require_stmt(&mut self) -> Result<Statement, X3Error> {
        let guard = self.parse_require_guard()?;
        Ok(Statement::Require(guard))
    }

    /// One action slot, two clauses.
    ///
    /// A `bridge` or `atomic swap` declaration carries a single failure action, and both
    /// `on_fail <action>` and `on_timeout <duration> <action>` write into it. This kept whichever
    /// was parsed first and discarded a second, different one without a word: measured,
    /// `bridge b ethereum.USDC to solana.USDC { on_fail halt on_timeout 30s refund solana.USDC to
    /// bob }` built, and the refund the program wrote reached no field (TICKET-120). The language
    /// refuses every other contradiction, so this one is refused and both actions are named.
    ///
    /// The same action stated twice stays accepted — that is what the formatter writes back when a
    /// declaration states a timeout at all, so refusing it would make the formatter's own output
    /// unparseable. The comparison is on the source text of each action, which is the same reading
    /// the formatter writes them with (`failure_action_source`).
    fn merge_failure_action(
        &self,
        stated: Option<FailureAction>,
        from_timeout: FailureAction,
    ) -> Result<FailureAction, X3Error> {
        let Some(stated) = stated else {
            return Ok(from_timeout);
        };
        let (first, second) = (
            crate::formatter::failure_action_source(&stated),
            crate::formatter::failure_action_source(&from_timeout),
        );
        if first == second {
            return Ok(stated);
        }
        Err(parse_err(
            format!(
                "`on_fail` and `on_timeout` state two different actions for the same failure path: \
                 `{first}` and `{second}`. This declaration carries one action, so state the same \
                 action in both clauses, or drop the one you do not want"
            ),
            self.peek(),
        ))
    }

    fn parse_failure_action(&mut self) -> Result<FailureAction, X3Error> {
        match self.peek() {
            Tok::Ident(ref s) if s == "rollback" => {
                self.advance();
                Ok(FailureAction::Rollback)
            }
            Tok::Ident(ref s) if s == "refund" => {
                self.advance();
                // The clause is `refund <chain.ASSET> to <receiver>`, and this arm used to read only
                // the first half: `to <receiver>` was left in the token stream, where the enclosing
                // body parsed it as two expression statements (`to;` and `sender;`) that lower to
                // nothing. The receiver — the whole point of naming a refund — never reached the
                // action, and `x3c fmt` wrote the residue back out as statements. Folded the way the
                // intent path folds it (`chain.ASSET:receiver`, receiver defaulting to `sender`, which
                // the formatter splits back with `refund_target`), so one clause has one shape.
                let asset = self.parse_asset_ref()?;
                let mut receiver = None;
                if matches!(self.peek(), Tok::Ident(ref s) if s == "to") {
                    self.advance();
                    receiver = Some(self.parse_expr()?);
                }
                let receiver = receiver
                    .map(|expr| expression_debug_string(&expr))
                    .unwrap_or_else(|| "sender".to_string());
                Ok(FailureAction::Refund(Expression::Literal(LiteralExpr::String(
                    Symbol::new(&format!(
                        "{}.{}:{}",
                        asset.chain.as_str(),
                        asset.name.as_str(),
                        receiver
                    )),
                ))))
            }
            Tok::Ident(ref s) if s == "halt" => {
                self.advance();
                Ok(FailureAction::Halt)
            }
            Tok::Ident(ref s) if s == "quarantine" => {
                self.advance();
                Ok(FailureAction::Quarantine)
            }
            _ => Err(parse_err(
                "expected rollback | refund | halt | quarantine".into(),
                self.peek(),
            )),
        }
    }

    fn parse_pattern(&mut self) -> Result<Pattern, X3Error> {
        match self.peek() {
            Tok::Ident(ref s) if s == "_" => {
                self.advance();
                Ok(Pattern::Wildcard)
            }
            Tok::Ident(name) => {
                self.advance();
                Ok(Pattern::Ident(Symbol::new(&name)))
            }
            Tok::KwTrue => Ok(Pattern::Literal(LiteralExpr::Bool(true))),
            Tok::KwFalse => Ok(Pattern::Literal(LiteralExpr::Bool(false))),
            Tok::Int(v) => Ok(Pattern::Literal(LiteralExpr::Int {
                value: v,
                base: IntBase::Decimal,
                suffix: None,
            })),
            _ => Err(parse_err("expected pattern".into(), self.peek())),
        }
    }

    /// Parse N_of_M syntax (e.g. 3_of_5) -> (3, 5)
    fn parse_n_of_m(&mut self) -> Result<(u32, u32), X3Error> {
        let s = self.expect_ident("N_of_M")?;
        let parts: Vec<&str> = s.split("_of_").collect();
        if parts.len() != 2 {
            return Err(parse_err(
                format!("expected N_of_M syntax (e.g. 3_of_5), got '{s}'"),
                Tok::Eof,
            ));
        }
        let n = parts[0]
            .parse::<u32>()
            .map_err(|_| parse_err(format!("invalid N_of_M numerator '{0}'", parts[0]), Tok::Eof))?;
        let m = parts[1]
            .parse::<u32>()
            .map_err(|_| parse_err(format!("invalid N_of_M denominator '{0}'", parts[1]), Tok::Eof))?;
        Ok((n, m))
    }

    fn opt_semi(&mut self) {
        if self.peek() == Tok::Semicolon {
            self.advance();
        }
    }

    /// Consume the next token if it matches `expected`.
    fn check(&mut self, expected: Tok) -> bool {
        if self.peek() == expected {
            self.advance();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, expected: Tok, msg: &str) -> Result<(), X3Error> {
        let found = self.advance();
        if found == expected {
            Ok(())
        } else {
            Err(parse_err(msg.into(), found))
        }
    }
}

// ===========================================================================
// Standalone helpers

/// Words that begin a clause in some block a guard can appear in.
///
/// See `Parser::next_word_begins_a_clause`: a guard stops here rather than
/// treating the word as what it is about.
/// Only the words that reach the parser as *identifiers* need listing: `swap`,
/// `bridge`, `require`, `emit`, `use`, `mint`, `burn`, `lock` and `release` are
/// mapped to keyword tokens, which cannot begin an expression either, so they
/// stop a guard without help.
///
/// Every word here is dispatched by an arm of the form `Tok::Ident(ref s) if s
/// == "<word>"`; `every_word_in_this_list_begins_a_clause` in
/// `compiler/tests/test_require_guards.rs` reads this const out of the source
/// and checks that, so an entry the grammar has moved past fails a test instead
/// of stopping a guard at a word that begins nothing. `balance` was exactly
/// that: it was listed under "statements and trade bodies" and no arm anywhere
/// dispatched on it, so `require <kind> balance` — a guard whose subject is the
/// identifier `balance` — was read as a guard with no subject.
const CLAUSE_WORDS: &[&str] = &[
    // intent body
    "from",
    "to",
    "route",
    "timeout",
    // `on_fail` is deliberately absent: it is `Tok::KwOnFail`, and a keyword cannot begin an
    // expression, so a valueless guard stops at it without help — the rule this const's own doc
    // states. It was listed anyway, which is a second statement of the grammar naming a word the
    // lookahead does not need.
    "allow",
    "on",
    "proofs",
    // route steps and `atomic swap` bodies
    "amount",
    "receiver",
    "hashlock",
    "min_output",
    // choice paths and route fallbacks
    "net_output",
    "replace",
    "leg",
    "path",
    "choose",
    // statements and trade bodies that carry a guard
    "repay",
    "borrow",
    "invariant",
    "net_profit",
];

/// The time unit a suffix names, or `None` for a suffix the language does not
/// define.
/// The unit a word names (`30 seconds`), for the clauses that write the unit
/// apart from the number.
pub(crate) fn duration_unit_from_word(word: &str) -> Option<x3_lang_common::DurationUnit> {
    use x3_lang_common::DurationUnit;
    Some(match word {
        "nanoseconds" => DurationUnit::Nanoseconds,
        "microseconds" => DurationUnit::Microseconds,
        "milliseconds" => DurationUnit::Milliseconds,
        "seconds" => DurationUnit::Seconds,
        "minutes" => DurationUnit::Minutes,
        "hours" => DurationUnit::Hours,
        "days" => DurationUnit::Days,
        _ => return None,
    })
}

pub(crate) fn duration_unit_from_suffix(suffix: &str) -> Option<x3_lang_common::DurationUnit> {
    use x3_lang_common::DurationUnit;
    Some(match suffix {
        "ns" => DurationUnit::Nanoseconds,
        "us" => DurationUnit::Microseconds,
        "ms" => DurationUnit::Milliseconds,
        "s" => DurationUnit::Seconds,
        "m" => DurationUnit::Minutes,
        "h" => DurationUnit::Hours,
        "d" => DurationUnit::Days,
        _ => return None,
    })
}

/// The words a guard's kind can be spelled with.
///
/// A list beside the match below, and the direction that matters is checked by a
/// test: every name here must map to a kind that is not `Custom`, because a word
/// outside this set is a guard whose condition no checker will ever read.
pub const REQUIRE_KIND_NAMES: &[&str] = &[
    "finality",
    "slippage",
    "fees",
    "profit",
    "invariant",
    "risk",
    "nonce",
    "audit_gate",
    "bridge_liquidity",
    "canonical_supply",
    "relayer_quorum",
    "route_score",
    "solver_bond",
    "proof_complete",
    "refund_path",
    "refund_to",
    "finality_explicit",
    "vm_supported",
    "mainnet_safe",
];

fn require_kind_from_str(name: &str) -> Result<RequireKind, X3Error> {
    Ok(match name {
        "finality" => RequireKind::Finality,
        "slippage" => RequireKind::Slippage,
        "fees" => RequireKind::Fees,
        "profit" => RequireKind::Profit,
        "invariant" => RequireKind::InvariantCheck,
        "risk" => RequireKind::RiskScore,
        "nonce" => RequireKind::Nonce,
        "audit_gate" => RequireKind::AuditGate,
        "bridge_liquidity" => RequireKind::BridgeLiquidity,
        "canonical_supply" => RequireKind::CanonicalSupply,
        "relayer_quorum" => RequireKind::RelayerQuorum,
        "route_score" => RequireKind::RouteScore,
        "solver_bond" => RequireKind::SolverBond,
        "proof_complete" => RequireKind::ProofComplete,
        "refund_path" => RequireKind::RefundPath,
        "refund_to" => RequireKind::RefundPath,
        "finality_explicit" => RequireKind::FinalityExplicit,
        "vm_supported" => RequireKind::VmSupported,
        "mainnet_safe" => RequireKind::MainnetSafe,
        "price_impact" => RequireKind::PriceImpact,
        "mev_leakage" => RequireKind::MevLeakage,
        other => RequireKind::Custom(Symbol::new(other)),
    })
}

/// Render a closed vocabulary for a diagnostic: sorted, comma-separated.
fn known_names<'a>(names: impl Iterator<Item = &'a str>) -> String {
    let mut names: Vec<&str> = names.collect();
    names.sort_unstable();
    names.join(", ")
}

fn parse_err(message: String, found: Tok) -> X3Error {
    X3Error::ParseError {
        message,
        span: Span::DUMMY,
        expected: vec![],
        found: format!("{:?}", found),
    }
}

// ===========================================================================
// Annotation, capability, expression helpers
// ===========================================================================

fn annotation_from_name_args(name: &str, args: &[Expression]) -> Result<Annotation, X3Error> {
    let s = |idx: usize| -> Result<String, X3Error> {
        let e = args.get(idx).ok_or_else(|| X3Error::ParseError {
            message: format!("@{name}: missing argument {idx}"),
            span: Span::DUMMY,
            expected: vec![],
            found: "".into(),
        })?;
        Ok(expr_to_string(e))
    };
    let u = |idx: usize| -> Result<u32, X3Error> {
        let e = args.get(idx).ok_or_else(|| X3Error::ParseError {
            message: format!("@{name}: missing numeric arg {idx}"),
            span: Span::DUMMY,
            expected: vec![],
            found: "".into(),
        })?;
        expr_to_u32(e)
    };
    match name {
        "no_heap" => Ok(Annotation::NoHeap),
        "no_recursion" => Ok(Annotation::NoRecursion(u(0)?)),
        "hot" => Ok(Annotation::Hot),
        "audit" => Ok(Annotation::Audit),
        "role" => Ok(Annotation::Role(Symbol::new(&s(0)?))),
        "multisig" => Ok(Annotation::Multisig(u(0)?, u(1)?)),
        "version" => Ok(Annotation::Version(Symbol::new(&s(0)?))),
        "upgrade_from" => Ok(Annotation::UpgradeFrom(Symbol::new(&s(0)?))),
        "on_chain" => Ok(Annotation::OnChain),
        "off_chain" => Ok(Annotation::OffChain),
        "sandbox" => Ok(Annotation::Sandbox),
        "whitelist" => {
            let items: Vec<Symbol> = args.iter().map(|e| Symbol::new(&expr_to_string(e))).collect();
            Ok(Annotation::Whitelist(items))
        }
        "concurrent" => Ok(Annotation::Concurrent),
        "scheduled" => {
            let period = args
                .iter()
                .find_map(|e| {
                    let s = expr_to_string(e);
                    if let Some(val) = s.strip_prefix("period=") {
                        val.parse::<u64>().ok()
                    } else {
                        None
                    }
                })
                .or_else(|| u(0).ok().map(|v| v as u64))
                .unwrap_or(1);
            Ok(Annotation::Scheduled(period))
        }
        // `"subscription"` is deliberately absent. The word is a **keyword** the lexer reserves for the
        // `subscription <name>: <amount>, <period> { … }` item, so `parse_single_annotation`'s
        // `expect_ident` never sees it as a name: the arm that used to be here could not be reached by
        // any program (measured: `@subscription(amount=100, period=30)` fails with "annotation name:
        // expected identifier", and the other twenty spellings parse). A name map that claims a
        // spelling the lexer forbids is a capability nothing can use (TICKET-111).
        "extern" => Ok(Annotation::Extern),
        "payable" => Ok(Annotation::Payable),
        "simd" => Ok(Annotation::Simd),
        "subscribe" => Ok(Annotation::Subscribe(Symbol::new(&s(0)?))),
        "sponsor" => Ok(Annotation::Sponsor),
        "gas_adaptive" => Ok(Annotation::GasAdaptive),
        _ => Err(X3Error::ParseError {
            // A word the lexer reserves cannot name an annotation — `expect_ident` refuses a keyword —
            // so a program that writes one is told what the language does accept instead of being told
            // the word is unknown. `subscription` is the only one today: an annotation spelling for it
            // existed in this map and no program could reach it (TICKET-111).
            message: if name == "subscription" {
                "`@subscription` is not an annotation: `subscription` begins a `subscription <name>: \
                 <amount>, <period> { … }` item, which is where a subscription's work is written — a \
                 keyword cannot name an annotation"
                    .to_string()
            } else {
                format!("unknown annotation @{name}")
            },
            span: Span::DUMMY,
            expected: vec![],
            found: name.into(),
        }),
    }
}

fn capability_from_call(name: &str, args: &[Expression]) -> Statement {
    let a = |i: usize| -> Expression { args.get(i).cloned().unwrap_or(Expression::Literal(LiteralExpr::Unit)) };
    match name {
        "snapshot" => Statement::Snapshot,
        "diff" => Statement::Diff {
            before: a(0),
            after: a(1),
        },
        "crdt_get" | "get_crdt" => Statement::CrdtOp(CrdtOp {
            kind: CrdtOpKind::Get,
            key: a(0),
            value: None,
        }),
        "crdt_set" | "set_crdt" => Statement::CrdtOp(CrdtOp {
            kind: CrdtOpKind::Set,
            key: a(0),
            value: Some(a(1)),
        }),
        "crdt_append" => Statement::CrdtOp(CrdtOp {
            kind: CrdtOpKind::Append,
            key: a(0),
            value: Some(a(1)),
        }),
        "migrate_and_destroy" => Statement::Migrate { new_contract: a(0) },
        "self_destruct" => Statement::SelfDestruct,
        "verify_zk" => Statement::ZkVerify {
            proof: a(0),
            public_input: a(1),
            key: a(2),
        },
        "verify_mpc" => Statement::MpcVerify {
            result: a(0),
            signatures: a(1),
            threshold: a(2),
        },
        "storage_store" => Statement::StorageRef {
            op: StorageRefOp::Store,
            data: a(0),
        },
        "storage_load" => Statement::StorageRef {
            op: StorageRefOp::Load,
            data: a(0),
        },
        "pathfind" => Statement::Pathfind {
            from: a(0),
            to: a(1),
            max_depth: a(2),
        },
        "mempool_scan" => Statement::MempoolScan { max_results: a(0) },
        "oracle_request" => Statement::OracleRequest {
            token: a(0),
            reward: a(1),
        },
        "pause" => Statement::Pause,
        "resume" => Statement::Resume,
        _ => Statement::Expr(Expression::Call {
            callee: Box::new(Expression::Ident(Symbol::new(name))),
            args: args.to_vec(),
        }),
    }
}

fn expr_to_string(e: &Expression) -> String {
    match e {
        Expression::Literal(LiteralExpr::Int { value, .. }) => value.to_string(),
        Expression::Literal(LiteralExpr::String(s)) | Expression::Literal(LiteralExpr::Hash(s)) => {
            s.as_str().to_string()
        }
        Expression::Ident(s) => s.as_str().to_string(),
        _ => format!("{:?}", e),
    }
}

fn expr_to_u32(e: &Expression) -> Result<u32, X3Error> {
    match e {
        Expression::Literal(LiteralExpr::Int { value, .. }) => Ok(*value as u32),
        _ => Err(parse_err("expected integer".into(), Tok::Eof)),
    }
}

fn expr_to_u64(e: &Expression) -> u64 {
    match e {
        Expression::Literal(LiteralExpr::Int { value, .. }) => *value as u64,
        _ => 0,
    }
}

/// Fill a route step's amount from the intent's source endpoint when the step states none.
///
/// Named for the *step*, not for the bridge: `Statement::Bridge` was the first statement
/// this handled and the name outlived the fact, so a reader looking for the rule the
/// parser's own comment describes on a body-level swap would not have found it here —
/// the swap arm was missing for as long as the name said it only filled bridges
/// (TICKET-088).
///
/// The source amounts come from the route's `Lock` steps, so this only reaches a step
/// whose `from` is the intent's own source asset. A later leg's input is what the
/// previous leg returns, which is a market outcome rather than a constant, and is left
/// alone — `lowering` refuses it rather than writing a zero.
fn fill_route_step_amounts(stmts: &mut [Statement]) {
    let source_amounts: Vec<(String, String, Expression)> = stmts
        .iter()
        .filter_map(|stmt| match stmt {
            Statement::Lock {
                chain, asset, amount, ..
            } if !expression_is_zero(amount) => Some((
                chain.as_str().to_ascii_lowercase(),
                asset.name.as_str().to_string(),
                amount.clone(),
            )),
            _ => None,
        })
        .collect();

    for stmt in stmts {
        if let Statement::Atomic(atomic) = stmt {
            fill_route_step_amounts_in_block(&mut atomic.body, &source_amounts);
        }
    }
}

fn fill_route_step_amounts_in_block(block: &mut Block, source_amounts: &[(String, String, Expression)]) {
    for stmt in &mut block.stmts {
        match stmt {
            Statement::Bridge { from, amount, .. } if expression_is_zero(amount) => {
                if let Some((_, _, source_amount)) = source_amounts.iter().find(|(source_chain, source_asset, _)| {
                    *source_chain == from.chain.as_str().to_ascii_lowercase() && source_asset == from.name.as_str()
                }) {
                    *amount = source_amount.clone();
                }
            }
            // A route **swap** follows the same rule `bridge` does — the comment on the
            // body-level swap at `parse_route_step` says so in as many words — but this
            // pass only ever filled bridges. A swap whose `from` is the intent's source
            // endpoint and which states no `amount` therefore lowered to zero and was
            // refused two passes later with "swap input_amount must be greater than
            // zero": a report about a zero that names neither the step nor the reason
            // (TICKET-088). This pass is not named after bridges because bridges are the
            // only thing that can be filled from an endpoint; that was an accident of
            // which statement was handled first.
            //
            // A step whose `from` is *not* a source endpoint still has nothing to fill
            // from and stays at zero, which is correct: its input is what the previous
            // leg returns, and that is a market outcome rather than a constant.
            Statement::Swap { from, amount, .. } if amount.as_ref().is_none_or(expression_is_zero) => {
                if let Some((_, _, source_amount)) = source_amounts.iter().find(|(source_chain, source_asset, _)| {
                    *source_chain == from.chain.as_str().to_ascii_lowercase() && source_asset == from.name.as_str()
                }) {
                    *amount = Some(source_amount.clone());
                }
            }
            Statement::Atomic(atomic) => {
                fill_route_step_amounts_in_block(&mut atomic.body, source_amounts);
            }
            _ => {}
        }
    }
}

fn expression_is_zero(expr: &Expression) -> bool {
    matches!(expr, Expression::Literal(LiteralExpr::Int { value: 0, .. }))
}

fn expression_debug_string(expr: &Expression) -> String {
    match expr {
        Expression::Ident(sym) => sym.as_str().to_string(),
        Expression::Literal(LiteralExpr::String(sym))
        | Expression::Literal(LiteralExpr::Address(sym))
        | Expression::Literal(LiteralExpr::Hash(sym)) => sym.as_str().to_string(),
        Expression::Literal(LiteralExpr::Int { value, .. }) => value.to_string(),
        _ => format!("{:?}", expr),
    }
}

fn expr_to_bool(e: &Expression) -> bool {
    match e {
        Expression::Literal(LiteralExpr::Bool(b)) => *b,
        _ => false,
    }
}

fn expr_to_u128(e: &Expression) -> Result<u128, X3Error> {
    match e {
        Expression::Literal(LiteralExpr::Int { value, .. }) => Ok(*value),
        _ => Err(parse_err("expected integer".into(), Tok::Eof)),
    }
}

fn annotate_item(item: Item, annotations: Vec<Annotation>) -> Item {
    match item {
        Item::Function(mut f) => {
            f.annotations = annotations;
            Item::Function(f)
        }
        Item::Agent(mut a) => {
            a.annotations = annotations;
            Item::Agent(a)
        }
        other => other,
    }
}

/// Tokenize source via the x3-lang-lexer crate, converting its
/// `Token` stream into the parser's internal `Tok` enum.
fn tokenize(source: &str) -> Vec<ParserToken> {
    let lexer = x3_lang_lexer::Lexer::new(source, 0);
    lexer
        .filter_map(|token| {
            let span = token.span;
            lexer_token_to_tok(token).map(|kind| ParserToken { kind, span })
        })
        .collect()
}

/// A comment, with the text and where it starts.
///
/// Read from the same lexer the parser reads, so the two agree about what a
/// comment is and where it ends — the formatter used to scan the source text
/// itself, with its own rules for strings and both comment forms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceComment {
    pub text: String,
    pub start: usize,
}

/// Every comment in a source, in the order they appear.
pub fn source_comments(source: &str) -> Vec<SourceComment> {
    let lexer = x3_lang_lexer::Lexer::new(source, 0);
    lexer
        .filter_map(|token| match token.kind {
            x3_lang_lexer::token::TokenKind::Comment(_) => {
                let start = token.span.start.as_usize();
                let end = token.span.end.as_usize();
                Some(SourceComment {
                    text: source.get(start..end).unwrap_or_default().to_string(),
                    start,
                })
            }
            _ => None,
        })
        .collect()
}

/// Convert a lexer token to the parser's Tok enum.
fn lexer_token_to_tok(token: Token) -> Option<Tok> {
    Some(match token.kind {
        TokenKind::Eof => Tok::Eof,
        TokenKind::Newline => return None,
        TokenKind::Unknown(_c) => return None,
        // A comment is not part of the grammar, so the parser steps over it —
        // but the lexer keeps it, and `source_comments` reads the same stream, so a
        // formatter has something to put back.
        TokenKind::Comment(_) => return None,

        TokenKind::Ident(sym) => ident_to_tok(sym.as_str()),
        TokenKind::Keyword(kw) => keyword_to_tok(kw).unwrap_or_else(|| Tok::Ident(kw.as_str().to_string())),

        TokenKind::Literal(lit) => match lit {
            x3_lang_lexer::token::Literal::Int { value, .. } => Tok::Int(value),
            x3_lang_lexer::token::Literal::String(sym) => Tok::String_(sym.as_str().to_string()),
            x3_lang_lexer::token::Literal::Float { value, .. } => Tok::Float(value),
            _ => return None,
        },

        // Delimiters
        TokenKind::Delimiter(d) => match d {
            x3_lang_lexer::token::Delimiter::OpenParen => Tok::LParen,
            x3_lang_lexer::token::Delimiter::CloseParen => Tok::RParen,
            x3_lang_lexer::token::Delimiter::OpenBrace => Tok::LBrace,
            x3_lang_lexer::token::Delimiter::CloseBrace => Tok::RBrace,
            x3_lang_lexer::token::Delimiter::OpenBracket => Tok::LBracket,
            x3_lang_lexer::token::Delimiter::CloseBracket => Tok::RBracket,
            x3_lang_lexer::token::Delimiter::OpenAngle => Tok::Lt,
            x3_lang_lexer::token::Delimiter::CloseAngle => Tok::Gt,
        },

        TokenKind::Comma => Tok::Comma,
        TokenKind::Semi => Tok::Semicolon,
        TokenKind::Colon => Tok::Colon,
        TokenKind::Eq => Tok::Eq,
        TokenKind::Dot => Tok::Dot,
        TokenKind::At => Tok::At,

        TokenKind::Arrow => Tok::Arrow,
        TokenKind::FatArrow => Tok::FatArrow,

        TokenKind::BinOp(op) => match op {
            CBinOp::Plus => Tok::Plus,
            CBinOp::Minus => Tok::Minus,
            CBinOp::Star => Tok::Star,
            CBinOp::Slash => Tok::Slash,
            CBinOp::Percent => Tok::Percent,
            CBinOp::AndAnd => Tok::AmpAmp,
            CBinOp::OrOr => Tok::PipePipe,
            CBinOp::EqEq => Tok::EqEq,
            CBinOp::Ne => Tok::Ne,
            CBinOp::Lt => Tok::Lt,
            CBinOp::Gt => Tok::Gt,
            CBinOp::Le => Tok::Le,
            CBinOp::Ge => Tok::Ge,
            _ => return None,
        },
        TokenKind::UnOp(CUnOp::Not) => Tok::Bang,
        TokenKind::UnOp(CUnOp::Neg) => Tok::Minus,
        TokenKind::UnOp(_) => return None,

        TokenKind::BinOpEq(_)
        | TokenKind::Question
        | TokenKind::Hash
        | TokenKind::Dollar
        | TokenKind::DotDot
        | TokenKind::DotDotDot
        | TokenKind::DotDotEq
        | TokenKind::PathSep => return None,
    })
}

fn ident_to_tok(word: &str) -> Tok {
    match word {
        "as" => Tok::KwAs,
        "in" => Tok::KwIn,
        "import" => Tok::KwImport,
        "atomic_swap" => Tok::KwAtomicSwap,
        "simd" => Tok::Ident(word.to_string()),
        other => Tok::Ident(other.to_string()),
    }
}

fn keyword_to_tok(kw: Keyword) -> Option<Tok> {
    Some(match kw {
        Keyword::Fn => Tok::KwFn,
        Keyword::Let => Tok::KwLet,
        Keyword::Mut => Tok::KwMut,
        Keyword::Return => Tok::KwReturn,
        Keyword::If => Tok::KwIf,
        Keyword::Else => Tok::KwElse,
        Keyword::While => Tok::KwWhile,
        Keyword::For => Tok::KwFor,
        Keyword::Loop => Tok::KwLoop,
        Keyword::Break => Tok::KwBreak,
        Keyword::Continue => Tok::KwContinue,
        Keyword::Agent => Tok::KwAgent,
        Keyword::Struct => Tok::KwStruct,
        Keyword::Enum => Tok::KwEnum,
        Keyword::Use => Tok::KwUse,
        Keyword::Mod => Tok::KwMod,
        Keyword::Const => Tok::KwConst,
        Keyword::Bridge => Tok::KwBridge,
        Keyword::Strategy => Tok::KwStrategy,
        Keyword::Proposal => Tok::KwProposal,
        Keyword::Gpu => Tok::KwGpu,
        Keyword::Simulate => Tok::KwSimulate,
        Keyword::Schedule => Tok::KwScheduled,
        Keyword::Intent => Tok::KwIntent,
        Keyword::Subscription => Tok::KwSubscription,
        Keyword::Pub => Tok::KwPub,
        Keyword::Async => Tok::KwAsync,
        Keyword::True => Tok::KwTrue,
        Keyword::False => Tok::KwFalse,
        Keyword::Require => Tok::KwRequire,
        Keyword::OnFail => Tok::KwOnFail,
        Keyword::OnTimeout => Tok::KwOnTimeout,
        Keyword::Lock => Tok::KwLock,
        Keyword::Mint => Tok::KwMint,
        Keyword::Burn => Tok::KwBurn,
        Keyword::Release => Tok::KwRelease,
        Keyword::Swap => Tok::KwSwap,
        Keyword::Match => Tok::KwMatch,
        Keyword::Atomic => Tok::KwAtomic,
        Keyword::Emit => Tok::KwEmit,
        Keyword::Try => Tok::KwTry,
        Keyword::Await => Tok::KwAwait,
        _ => return None,
    })
}

/// Re-export the `BinOp` and `UnOp` types the parser uses so callers can
/// reference them through the compiler crate.
pub use x3_lang_common::{BinOp, UnOp};
