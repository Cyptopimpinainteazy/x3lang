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

struct Parser<'a> {
    tokens: &'a [Tok],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn new(tokens: &'a [Tok]) -> Self {
        Parser { tokens, pos: 0 }
    }

    fn peek(&self) -> Tok {
        self.tokens.get(self.pos).cloned().unwrap_or(Tok::Eof)
    }

    fn peek_n(&self, n: usize) -> Tok {
        self.tokens.get(self.pos + n).cloned().unwrap_or(Tok::Eof)
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
            match self.peek() {
                Tok::Eof => break,
                Tok::At => {
                    // Annotations are collected eagerly and attached to the
                    // next top-level item.
                    let annots = self.parse_annotations()?;
                    let item = self.parse_top_item()?;
                    let with_annots = annotate_item(item, annots);
                    items.push(Spanned::new(with_annots, Span::DUMMY));
                }
                _ => {
                    let item = self.parse_top_item()?;
                    items.push(Spanned::new(item, Span::DUMMY));
                }
            }
        }
        Ok(items)
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
                } else {
                    // Parser saw 'atomic' at top level without 'swap' —
                    // not a valid top-level item.
                    Err(parse_err(
                        "expected 'swap' after 'atomic' at top level".into(),
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
                    on_fail = Some(self.parse_failure_action()?);
                }
                Tok::KwOnTimeout => {
                    self.advance();
                    let dur = self.parse_expr()?;
                    let action = self.parse_failure_action()?;
                    timeout = Some(dur);
                    if on_fail.is_none() {
                        on_fail = Some(action);
                    }
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
                    on_fail = Some(self.parse_failure_action()?);
                }
                Tok::KwOnTimeout => {
                    self.advance();
                    let duration = self.parse_expr()?;
                    let action = self.parse_failure_action()?;
                    // Store timeout on destination by default for backward compat
                    timeout_destination = Some(duration);
                    if on_fail.is_none() {
                        on_fail = Some(action);
                    }
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
                    let duration = self.parse_expr()?;
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

    fn parse_strategy_item(&mut self) -> Result<Item, X3Error> {
        self.advance();
        let name = self.expect_ident("strategy name")?;
        let mut max_steps = None;
        let mut max_gas = None;
        if self.peek() == Tok::LBrace {
            // no config; proceed
        } else {
            // optional max_steps / max_gas
            while self.peek() != Tok::LBrace && self.peek() != Tok::Eof {
                match self.peek() {
                    Tok::Ident(key) if key.as_str() == "max_steps" => {
                        self.advance();
                        self.expect(Tok::Eq, "expected '='")?;
                        max_steps = Some(self.parse_expr()?);
                        if self.peek() == Tok::Comma {
                            self.advance();
                        }
                    }
                    Tok::Ident(key) if key.as_str() == "max_gas" => {
                        self.advance();
                        self.expect(Tok::Eq, "expected '='")?;
                        max_gas = Some(self.parse_expr()?);
                        if self.peek() == Tok::Comma {
                            self.advance();
                        }
                    }
                    _ => break,
                }
            }
        }
        self.expect(Tok::LBrace, "expected '{'")?;
        let mut body = Vec::new();
        let mut requires = Vec::new();
        let mut on_fail = None;
        while self.peek() != Tok::RBrace && self.peek() != Tok::Eof {
            match self.peek() {
                Tok::KwRequire => requires.push(self.parse_require_guard()?),
                Tok::KwOnFail => {
                    self.advance();
                    on_fail = Some(self.parse_failure_action()?);
                }
                _ => body.push(self.parse_statement()?),
            }
        }
        self.expect(Tok::RBrace, "expected '}'")?;
        Ok(Item::Strategy(CrossChainStrategy {
            name: Symbol::new(&name),
            max_steps,
            max_gas,
            body,
            requires,
            on_fail,
        }))
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
        fill_route_bridge_amounts(&mut stmts);
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
            Tok::Ident(ref s) if s == "on_fail" => self.parse_intent_onfail(),
            Tok::KwOnFail => self.parse_intent_onfail(),
            Tok::Ident(ref s) if s == "use" => self.parse_intent_use(),
            Tok::Ident(ref s) if s == "on" => self.parse_intent_on_event(),
            _ => {
                // Fallback: treat as an expression statement so the typechecker
                // gets a real diagnostic instead of a hard panic.
                let e = self.parse_expr()?;
                self.opt_semi();
                Ok(Statement::Expr(e))
            }
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
            // Bare-identifier route keywords. The dispatcher does NOT
            // advance: each sub-parser checks for its own leading
            // identifier and consumes it. This avoids a rewind/peek
            // dance.
            Tok::Ident(ref s) if s == "swap" => self.parse_swap_step(),
            Tok::Ident(ref s) if s == "bridge" => self.parse_bridge_step(),
            Tok::Ident(ref s) if s == "lock" || s == "mint" || s == "burn" || s == "release" => {
                let kw = s.clone();
                self.advance();
                self.parse_lmbr_step(&kw)
            }
            _ => Err(parse_err(
                "expected route operation (swap/bridge/lock/mint/burn/release)".into(),
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
            route: amount,
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
            source_finality_proof,
            transfer_proof,
        })
    }

    /// `timeout <N>[s] [refund <chain.ASSET> to <receiver>]`
    fn parse_intent_timeout(&mut self) -> Result<Statement, X3Error> {
        self.advance(); // consume `timeout`
        let dur = self.parse_expr()?;
        // Pull the block count from an integer literal; anything else
        // falls back to 0 (the existing `OnTimeout` IR only stores u32).
        let dur_blocks: u32 = match &dur {
            Expression::Literal(LiteralExpr::Int { value, .. }) => *value as u32,
            Expression::Ident(sym) => numeric_prefix_u32(sym.as_str()).unwrap_or(0),
            _ => 0,
        };
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
        Ok(Statement::OnTimeout {
            duration: Expression::Literal(LiteralExpr::Int {
                value: dur_blocks as u128,
                base: IntBase::Decimal,
                suffix: None,
            }),
            action,
        })
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
                _ => break,
            }
        }
        self.expect(Tok::RBrace, "expected '}' after solver_market body")?;
        Ok(Item::SolverMarket(SolverMarket { mode, min_reputation }))
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
                    if matches!(self.peek(), Tok::Ident(ref s2) if s2 == "require") {
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
                Tok::Ident(ref s) if s == "require" => {
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

    /// `risk_policy { max_slippage <pct> max_fee <pct> max_route_risk <level> min_liquidity <int> ... }`
    fn parse_risk_policy_item(&mut self) -> Result<Item, X3Error> {
        self.advance();
        self.expect(Tok::LBrace, "expected '{' after risk_policy")?;
        let mut max_slippage: u64 = 0;
        let mut max_position: Option<u128> = None;
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
                _ => {
                    // Skip unknown config fields
                    self.advance();
                }
            }
        }
        self.expect(Tok::RBrace, "expected '}' after risk_policy body")?;
        Ok(Item::RiskPolicy(RiskPolicy {
            max_slippage,
            max_position,
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
            let _assert_kw = self.expect_ident("expected 'assert' in invariant body")?;
            let expr = self.parse_expr()?;
            self.expect(Tok::RBrace, "expected '}' after invariant body")?;
            Symbol::new(&format!("{:?}", expr))
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

    /// `finality_policy <name> { <vm> require <mode> ... }`
    fn parse_finality_policy_item(&mut self) -> Result<Item, X3Error> {
        self.advance();
        let mode = Symbol::new(&self.expect_ident("finality_policy mode")?);
        self.expect(Tok::LBrace, "expected '{' after finality_policy")?;
        let mut chain = Symbol::new("unknown");
        let mut requirement = Symbol::new("finalized");
        while self.peek() != Tok::RBrace && self.peek() != Tok::Eof {
            match self.peek() {
                // Parse: <chain_name> require <mode>
                Tok::Ident(ref s) if s != "require" => {
                    chain = Symbol::new(&self.expect_ident("finality chain name")?);
                    // Expect `require`
                    if matches!(self.peek(), Tok::Ident(ref r) if r == "require") {
                        self.advance();
                    }
                    requirement = Symbol::new(&self.expect_ident("finality requirement")?);
                }
                _ => break,
            }
        }
        self.expect(Tok::RBrace, "expected '}' after finality_policy body")?;
        Ok(Item::FinalityPolicy(FinalityPolicy {
            mode,
            chain,
            requirement,
        }))
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
        let name = self.expect_ident("annotation name")?;
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
                let route: Option<Expression> = if self.peek() == Tok::Arrow {
                    // skip arrow between assets, e.g., eth.USDC -> sol.USDC
                    self.advance();
                    None
                } else {
                    None
                };
                let _route_expr = route;
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
                    route: None,
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

    fn parse_require_guard(&mut self) -> Result<RequireGuard, X3Error> {
        self.advance(); // 'require'
        let ident = self.expect_ident("require kind")?;
        // Special-case source_finality and dest_finality: kind=Finality, subject=ident
        let (kind, subject) = match ident.as_str() {
            "source_finality" => (RequireKind::Finality, Some(Symbol::new("source_finality"))),
            "dest_finality" => (RequireKind::Finality, Some(Symbol::new("dest_finality"))),
            _ => {
                let kind = require_kind_from_str(&ident)?;
                let subject = if self.peek() == Tok::Dot {
                    self.advance(); // '.'
                    Some(Symbol::new(&self.expect_ident("require subject after '.'")?))
                } else if matches!(self.peek(), Tok::Ident(_)) {
                    Some(Symbol::new(&self.expect_ident("require subject")?))
                } else {
                    None
                };
                (kind, subject)
            }
        };
        // Skip optional comparison operator (>=, ==, <=, >, <, !=)
        match self.peek() {
            Tok::Ge | Tok::Gt | Tok::Le | Tok::Lt | Tok::EqEq | Tok::Ne => {
                self.advance();
            }
            _ => {}
        }
        let value = self.parse_expr()?;
        self.opt_semi();
        Ok(RequireGuard { kind, subject, value })
    }

    fn parse_require_stmt(&mut self) -> Result<Statement, X3Error> {
        let guard = self.parse_require_guard()?;
        Ok(Statement::Require(guard))
    }

    fn parse_failure_action(&mut self) -> Result<FailureAction, X3Error> {
        match self.peek() {
            Tok::Ident(ref s) if s == "rollback" => {
                self.advance();
                Ok(FailureAction::Rollback)
            }
            Tok::Ident(ref s) if s == "refund" => {
                self.advance();
                let expr = self.parse_expr()?;
                Ok(FailureAction::Refund(expr))
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

fn require_kind_from_str(name: &str) -> Result<RequireKind, X3Error> {
    Ok(match name {
        "finality" => RequireKind::Finality,
        "slippage" => RequireKind::Slippage,
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
        other => RequireKind::Custom(Symbol::new(other)),
    })
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
        "subscription" => {
            let amount = args
                .iter()
                .find_map(|e| {
                    let s = expr_to_string(e);
                    if let Some(val) = s.strip_prefix("amount=") {
                        val.parse::<u128>().ok()
                    } else {
                        None
                    }
                })
                .or_else(|| args.first().and_then(|e| expr_to_u128(e).ok()))
                .unwrap_or(0);
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
                .or_else(|| args.get(1).and_then(|e| expr_to_u128(e).ok().map(|v| v as u64)))
                .unwrap_or(1);
            Ok(Annotation::Subscription(amount, period))
        }
        "extern" => Ok(Annotation::Extern),
        "payable" => Ok(Annotation::Payable),
        "simd" => Ok(Annotation::Simd),
        "subscribe" => Ok(Annotation::Subscribe(Symbol::new(&s(0)?))),
        "sponsor" => Ok(Annotation::Sponsor),
        "gas_adaptive" => Ok(Annotation::GasAdaptive),
        _ => Err(X3Error::ParseError {
            message: format!("unknown annotation @{name}"),
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

fn fill_route_bridge_amounts(stmts: &mut [Statement]) {
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
            fill_route_bridge_amounts_in_block(&mut atomic.body, &source_amounts);
        }
    }
}

fn fill_route_bridge_amounts_in_block(block: &mut Block, source_amounts: &[(String, String, Expression)]) {
    for stmt in &mut block.stmts {
        match stmt {
            Statement::Bridge { from, amount, .. } if expression_is_zero(amount) => {
                if let Some((_, _, source_amount)) = source_amounts.iter().find(|(source_chain, source_asset, _)| {
                    *source_chain == from.chain.as_str().to_ascii_lowercase() && source_asset == from.name.as_str()
                }) {
                    *amount = source_amount.clone();
                }
            }
            Statement::Atomic(atomic) => {
                fill_route_bridge_amounts_in_block(&mut atomic.body, source_amounts);
            }
            _ => {}
        }
    }
}

fn expression_is_zero(expr: &Expression) -> bool {
    matches!(expr, Expression::Literal(LiteralExpr::Int { value: 0, .. }))
}

fn numeric_prefix_u32(value: &str) -> Option<u32> {
    let digits: String = value.chars().take_while(|ch| ch.is_ascii_digit()).collect();
    if digits.is_empty() {
        None
    } else {
        digits.parse::<u32>().ok()
    }
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
fn tokenize(source: &str) -> Vec<Tok> {
    let lexer = x3_lang_lexer::Lexer::new(source, 0);
    lexer.filter_map(lexer_token_to_tok).collect()
}

/// Convert a lexer token to the parser's Tok enum.
fn lexer_token_to_tok(token: Token) -> Option<Tok> {
    Some(match token.kind {
        TokenKind::Eof => Tok::Eof,
        TokenKind::Newline => return None,
        TokenKind::Unknown(_c) => return None,

        TokenKind::Ident(sym) => ident_to_tok(sym.as_str()),
        TokenKind::Keyword(kw) => keyword_to_tok(kw).unwrap_or_else(|| Tok::Ident(kw.as_str().to_string())),

        TokenKind::Literal(lit) => match lit {
            x3_lang_lexer::token::Literal::Int { value, .. } => Tok::Int(value),
            x3_lang_lexer::token::Literal::String(sym) => Tok::String_(sym.as_str().to_string()),
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
