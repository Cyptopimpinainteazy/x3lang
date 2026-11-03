//! Lowering of X3 Lang trading programs into **canonical X3BC** — the bytecode the X3 chain
//! executes (`crates/x3-vm`), defined once in `crates/x3bc-core`
//! (docs/roadmap/X3LANG_PRODUCTION_UNIFICATION.md).
//!
//! The front end is this crate's: the program is parsed and goes through every verification layer
//! [`crate::run_pre_emission_layers`] runs (AST checks, trading semantics, the trading verifier,
//! lowering, the IR verifier, supply and semantic verification), in the requested
//! [`CompilationMode`]. The verified trading IR is then lowered into the canonical trading section
//! and a straight-line X3BC entry function:
//!
//! ```text
//! atomic_begin 0
//! trade_begin
//! trade_<op> i          ; one per section operation, in order
//! trade_commit
//! trade_load_profit r0  ; the profit asset's net change, a checked i64
//! atomic_commit 0
//! ret r0
//! ```
//!
//! Costs are charged by an explicit `charge_costs` operation placed immediately before the first
//! profit floor, so every cost is committed before profit is evaluated.
//!
//! Anything the chain cannot execute is refused here with an explicit `X3BC-L…` diagnostic —
//! never dropped, never replaced with something that does less.

use std::collections::BTreeMap;

use x3_lang_ast::Program;
use x3_lang_common::X3Error;

use crate::ir::{
    AssetKey, CompiledTradingPolicy, CostKind as IrCostKind, InvariantKind, Operation, SubmissionProfile,
    TradingOperation, ValueRef,
};
use crate::semantic::CompilationMode;
use x3bc_core::opcode::Opcode;
use x3bc_core::receipt::CostKind;
use x3bc_core::trading::{
    assemble_trading_module, asset_id_for, chain_id_for, venue_pool_id, verify_section, Amount, AssetRef,
    CapabilityKind, CapabilityRequirement, CompilationModeId, TradeOp, TradingPolicy, TradingSection,
    CAPABILITY_VERSION, FLASH_PROVIDER, ORACLE, TRADING_SECTION_VERSION,
};

/// A compiled canonical trading artifact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct X3bcArtifact {
    /// The X3BC module (format 1.1, with its trading section).
    pub bytes: Vec<u8>,
    /// SHA-256 of the source text.
    pub source_hash: [u8; 32],
    /// The mode the program was compiled in (also carried in the artifact and its receipts).
    pub mode: CompilationModeId,
    /// The trading section the module carries.
    pub section: TradingSection,
}

fn refuse(code: &str, message: impl Into<String>) -> X3Error {
    X3Error::CodegenError {
        message: format!("X3BC-{code}: {}", message.into()),
        span: None,
    }
}

fn mode_id(mode: CompilationMode) -> CompilationModeId {
    match mode {
        CompilationMode::Dev => CompilationModeId::Dev,
        CompilationMode::Testnet => CompilationModeId::Testnet,
        CompilationMode::Mainnet => CompilationModeId::Mainnet,
    }
}

/// The variant name of an IR operation, for a diagnostic that names what was refused.
fn construct_name(op: &Operation) -> String {
    let debug = format!("{op:?}");
    debug
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .next()
        .unwrap_or("operation")
        .to_string()
}

/// Compile `.x3` source containing one atomic trade into canonical X3BC.
pub fn compile_trading_x3bc(source: &str, mode: CompilationMode) -> Result<X3bcArtifact, X3Error> {
    let program: Program = crate::parse_source(source)?;
    let pre = crate::run_pre_emission_layers(&program, mode)?;
    if !pre.errors.is_empty() {
        return Err(crate::format_pipeline_errors(&pre.errors));
    }

    let mut trading = Vec::new();
    let mut private_mode_check = false;
    for op in &pre.ir.operations {
        match op {
            Operation::Trading(trading_op) => trading.push(trading_op.clone()),
            Operation::ModeCheck { mode, restriction } if mode == "submission" && restriction == "private_required" => {
                private_mode_check = true;
            }
            other => {
                return Err(refuse(
                    "L001",
                    format!(
                        "`{}` cannot be lowered to canonical X3BC: only an `atomic trade` program \
                         is a trading artifact",
                        construct_name(other)
                    ),
                ))
            }
        }
    }

    let source_hash = x3bc_core::hash::sha256(source.as_bytes());
    let section = lower_trade(&trading, source_hash, mode_id(mode))?;
    if section.policy.private_submission_required != private_mode_check {
        return Err(refuse(
            "L090",
            "the private-submission mode check and the compiled policy disagree",
        ));
    }
    verify_section(&section).map_err(|error| {
        refuse(
            "L099",
            format!(
                "the lowered trading section is unsound ({}{}); this is a compiler defect",
                error.reason,
                error.op_index.map(|i| format!(", operation {i}")).unwrap_or_default()
            ),
        )
    })?;

    let code = entry_code(&section);
    let bytes = assemble_trading_module(section.clone(), code);
    Ok(X3bcArtifact {
        bytes,
        source_hash,
        mode: section.mode,
        section,
    })
}

/// The straight-line entry function for `section`.
fn entry_code(section: &TradingSection) -> Vec<u8> {
    let mut code = Vec::new();
    code.push(Opcode::AtomicBegin as u8);
    code.extend_from_slice(&0u16.to_le_bytes());
    code.push(Opcode::TradeBegin as u8);
    for (index, op) in section.ops.iter().enumerate() {
        code.push(op.opcode() as u8);
        // `verify_section` bounds the operation count far below `u16::MAX`.
        code.extend_from_slice(&(index as u16).to_le_bytes());
    }
    code.push(Opcode::TradeCommit as u8);
    code.push(Opcode::TradeLoadProfit as u8);
    code.push(0); // r0
    code.push(Opcode::AtomicCommit as u8);
    code.extend_from_slice(&0u16.to_le_bytes());
    code.push(Opcode::Ret as u8);
    code.push(0); // r0
    code
}

/// Tables built while lowering: each distinct asset, capability, debt and binding gets one index.
#[derive(Default)]
struct Tables {
    assets: Vec<AssetRef>,
    asset_index: BTreeMap<u32, u16>,
    capabilities: Vec<CapabilityRequirement>,
    debts: Vec<String>,
    debt_asset: BTreeMap<String, u16>,
    bindings: Vec<String>,
}

impl Tables {
    fn asset(&mut self, key: &AssetKey, chain: &str) -> Result<u16, X3Error> {
        let family = key.vm_family.to_ascii_lowercase();
        if family != "x3" && family != "x3vm" {
            return Err(refuse(
                "L010",
                format!(
                    "asset `{}` is on `{}`/`{}`: only X3-chain assets lower to canonical X3BC \
                     (cross-chain trading has no on-chain venue yet)",
                    key.symbol, key.vm_family, key.chain
                ),
            ));
        }
        if key.chain != chain {
            return Err(refuse(
                "L010",
                format!(
                    "asset `{}` is on chain `{}`, the trade on `{chain}`",
                    key.symbol, key.chain
                ),
            ));
        }
        let id = asset_id_for(&key.canonical_id).ok_or_else(|| {
            refuse(
                "L011",
                format!(
                    "asset `{}` has canonical id `{}`: an X3-chain asset is `asset_<id>` with an \
                     id that fits the chain's u32 asset type (checked conversion)",
                    key.symbol, key.canonical_id
                ),
            )
        })?;
        if let Some(index) = self.asset_index.get(&id) {
            let existing = &self.assets[usize::from(*index)];
            if existing.decimals != key.decimals {
                return Err(refuse(
                    "L011",
                    format!("asset id {id} is declared with two different decimal counts"),
                ));
            }
            return Ok(*index);
        }
        let index = u16::try_from(self.assets.len()).map_err(|_| refuse("L012", "too many assets"))?;
        self.assets.push(AssetRef {
            id,
            decimals: key.decimals,
            symbol: key.symbol.clone(),
        });
        self.asset_index.insert(id, index);
        Ok(index)
    }

    fn capability(&mut self, kind: CapabilityKind, name: &str) -> Result<u16, X3Error> {
        if let Some(index) = self
            .capabilities
            .iter()
            .position(|cap| cap.kind == kind && cap.name == name)
        {
            return u16::try_from(index).map_err(|_| refuse("L012", "too many capabilities"));
        }
        let known = match kind {
            CapabilityKind::FlashProvider => name == FLASH_PROVIDER,
            CapabilityKind::Venue => venue_pool_id(name).is_some(),
            CapabilityKind::Oracle => name == ORACLE,
        };
        if !known {
            let (code, what, expected) = match kind {
                CapabilityKind::FlashProvider => ("L020", "flash provider", FLASH_PROVIDER.to_string()),
                CapabilityKind::Venue => ("L021", "venue", "x3dex_pool_<pool id>".to_string()),
                CapabilityKind::Oracle => ("L022", "oracle", ORACLE.to_string()),
            };
            return Err(refuse(
                code,
                format!("{what} `{name}` is not offered by the X3 chain (expected `{expected}`)"),
            ));
        }
        let index = u16::try_from(self.capabilities.len()).map_err(|_| refuse("L012", "too many capabilities"))?;
        self.capabilities.push(CapabilityRequirement {
            kind,
            name: name.to_string(),
            version: CAPABILITY_VERSION,
        });
        Ok(index)
    }

    fn slot(names: &mut Vec<String>, name: &str) -> Result<u8, X3Error> {
        if names.iter().any(|existing| existing == name) {
            return Err(refuse("L030", format!("`{name}` is bound twice")));
        }
        let slot = u8::try_from(names.len()).map_err(|_| refuse("L012", "too many slots"))?;
        names.push(name.to_string());
        Ok(slot)
    }

    fn find(names: &[String], name: &str) -> Option<u8> {
        names
            .iter()
            .position(|existing| existing == name)
            .and_then(|i| u8::try_from(i).ok())
    }
}

fn lower_policy(policy: &CompiledTradingPolicy, tables: &mut Tables, chain: &str) -> Result<TradingPolicy, X3Error> {
    if policy.policy_version != 1 {
        return Err(refuse(
            "L040",
            format!("policy version {} is not supported", policy.policy_version),
        ));
    }
    if policy.max_mev_leakage_bps.is_some() {
        return Err(refuse(
            "L041",
            "`max_mev_leakage` has no measurement source on the X3 chain; the policy cannot be \
             enforced, so it is refused rather than ignored",
        ));
    }
    if policy.max_cumulative_loss.is_some() || policy.max_cumulative_loss_asset.is_some() {
        return Err(refuse(
            "L042",
            "`max_cumulative_loss` is a cross-trade breaker with no on-chain state to keep it; \
             refused rather than ignored",
        ));
    }
    if policy.allow_mint || policy.allow_burn {
        return Err(refuse(
            "L043",
            "minting and burning are not trading operations on chain",
        ));
    }
    if !policy.submission_profile_is_consistent() {
        return Err(refuse("L044", "inconsistent submission profile"));
    }
    if policy.submission_profile == SubmissionProfile::Protected {
        return Err(refuse(
            "L045",
            "the `Protected` submission profile has no channel on the X3 chain",
        ));
    }
    let max_cost_asset = tables.asset(&policy.max_gas_asset, chain)?;
    let mut allowed: u16 = 0;
    for kind in &policy.allowed_cost_kinds {
        // Measures (slippage, impact, leakage) bound figures; they are not charges. A proof fee has
        // nothing on chain that charges it, so admitting it changes nothing.
        let canonical = match kind {
            IrCostKind::Gas => Some(CostKind::Gas),
            IrCostKind::LiquidityFee => Some(CostKind::VenueFee),
            IrCostKind::FlashLiquidityFee => Some(CostKind::FlashFee),
            IrCostKind::SolverInfrastructureFee => Some(CostKind::SolverFee),
            IrCostKind::CrossDomainFee => Some(CostKind::CrossDomainFee),
            _ => None,
        };
        if let Some(kind) = canonical {
            allowed |= kind.bit();
        }
    }
    Ok(TradingPolicy {
        policy_id: policy.policy_id.clone(),
        policy_version: policy.policy_version,
        max_slippage_bps: policy.max_slippage_bps,
        max_cost: policy.max_gas,
        max_cost_asset,
        max_flash_fee_bps: policy.max_flash_fee_bps,
        deadline_blocks: policy.deadline_blocks,
        private_submission_required: policy.submission_profile == SubmissionProfile::Private,
        quote_freshness_blocks: policy.quote_freshness_blocks,
        max_oracle_deviation_bps: policy.max_oracle_deviation_bps,
        max_price_impact_bps: policy.max_price_impact_bps,
        allowed_cost_kinds: allowed,
    })
}

fn lower_trade(
    ops: &[TradingOperation],
    source_hash: [u8; 32],
    mode: CompilationModeId,
) -> Result<TradingSection, X3Error> {
    let Some(TradingOperation::BeginAtomicTrade { trade_id, policy }) = ops.first() else {
        return Err(refuse("L002", "the program contains no atomic trade"));
    };
    if ops
        .iter()
        .skip(1)
        .any(|op| matches!(op, TradingOperation::BeginAtomicTrade { .. }))
    {
        return Err(refuse("L003", "one artifact carries one atomic trade"));
    }
    let chain = policy.chain.clone();
    let chain_id = chain_id_for(&chain).ok_or_else(|| {
        refuse(
            "L004",
            format!(
                "chain `{chain}` is not the X3 chain (`{}`)",
                x3bc_core::trading::X3_CHAIN_NAME
            ),
        )
    })?;

    // Refuse what cannot run before building any table, so the diagnostic names the construct.
    for op in ops {
        match op {
            TradingOperation::Bridge { via, .. } => {
                return Err(refuse(
                    "L005",
                    format!(
                        "`bridge ... via {via}` cannot lower: the X3 chain has no proof-verified \
                         bridge venue in the trading extension yet"
                    ),
                ))
            }
            TradingOperation::AbortAtomicTrade => return Err(refuse("L006", "an explicit abort has no X3BC lowering")),
            _ => {}
        }
    }

    let mut tables = Tables::default();
    let trading_policy = lower_policy(policy, &mut tables, &chain)?;
    let mut lowered: Vec<TradeOp> = Vec::new();
    let mut charged = false;

    let push_profit = |lowered: &mut Vec<TradeOp>, charged: &mut bool, asset: u16, minimum: u128| {
        if !*charged {
            lowered.push(TradeOp::ChargeCosts);
            *charged = true;
        }
        lowered.push(TradeOp::AssertMinProfit { asset, minimum });
    };

    for op in ops.iter().skip(1) {
        match op {
            TradingOperation::OpenDebt {
                debt_id,
                provider,
                asset,
                principal,
            } => {
                let provider = tables.capability(CapabilityKind::FlashProvider, provider)?;
                let asset = tables.asset(asset, &chain)?;
                let debt = Tables::slot(&mut tables.debts, debt_id)?;
                tables.debt_asset.insert(debt_id.clone(), asset);
                lowered.push(TradeOp::Borrow {
                    debt,
                    provider,
                    asset,
                    principal: *principal,
                });
            }
            TradingOperation::ExecuteSwap {
                binding,
                venue,
                from,
                to,
                input,
                min_output,
            } => {
                let venue = tables.capability(CapabilityKind::Venue, venue)?;
                let from = tables.asset(from, &chain)?;
                let to = tables.asset(to, &chain)?;
                let input = match input {
                    ValueRef::Literal(amount) => Amount::Literal(*amount),
                    ValueRef::Binding(name) => match name.split_once('.') {
                        Some((debt_id, "amount")) => Amount::DebtPrincipal(
                            Tables::find(&tables.debts, debt_id)
                                .ok_or_else(|| refuse("L031", format!("unresolved debt `{debt_id}`")))?,
                        ),
                        Some((_, field)) => {
                            return Err(refuse(
                                "L032",
                                format!("`{name}`: a debt exposes `.amount`, not `.{field}`"),
                            ))
                        }
                        None => Amount::Binding(
                            Tables::find(&tables.bindings, name)
                                .ok_or_else(|| refuse("L031", format!("unresolved binding `{name}`")))?,
                        ),
                    },
                };
                let binding = Tables::slot(&mut tables.bindings, binding)?;
                lowered.push(TradeOp::Swap {
                    binding,
                    venue,
                    from,
                    to,
                    input,
                    min_out: *min_output,
                });
            }
            TradingOperation::CloseDebt { debt_id } => {
                let debt = Tables::find(&tables.debts, debt_id)
                    .ok_or_else(|| refuse("L031", format!("unresolved debt `{debt_id}`")))?;
                lowered.push(TradeOp::Repay { debt });
            }
            TradingOperation::AssertMinNetProfit {
                settlement_asset,
                minimum,
            } => {
                let asset = tables.asset(settlement_asset, &chain)?;
                push_profit(&mut lowered, &mut charged, asset, *minimum);
            }
            TradingOperation::AssertAllDebtsClosed => lowered.push(TradeOp::AssertAllDebtsClosed),
            TradingOperation::AssertInvariant {
                kind: InvariantKind::Solvent,
            } => lowered.push(TradeOp::AssertSolvent),
            // Every committed trade produces a receipt; the statement asks for what always happens.
            TradingOperation::EmitTradeReceipt => {}
            TradingOperation::CommitAtomicTrade => {}
            TradingOperation::BeginAtomicTrade { .. }
            | TradingOperation::Bridge { .. }
            | TradingOperation::AbortAtomicTrade => return Err(refuse("L099", "unreachable trading operation")),
        }
    }
    // The oracle the swaps are cross-checked against, required exactly when the policy bounds
    // oracle deviation.
    if trading_policy.max_oracle_deviation_bps.is_some() {
        tables.capability(CapabilityKind::Oracle, ORACLE)?;
    }
    if !matches!(ops.last(), Some(TradingOperation::CommitAtomicTrade)) {
        return Err(refuse("L007", "the trade does not end in a commit"));
    }
    // The policy's own profit floor, in its named asset, after every cost like any other.
    if let (Some(minimum), Some(asset)) = (policy.minimum_net_profit, policy.minimum_net_profit_asset.as_ref()) {
        let asset = tables.asset(asset, &chain)?;
        push_profit(&mut lowered, &mut charged, asset, minimum);
    }

    Ok(TradingSection {
        version: TRADING_SECTION_VERSION,
        chain_id,
        mode,
        source_hash,
        compiler_version: crate::COMPILER_VERSION.to_string(),
        trade_id: trade_id.clone(),
        policy: trading_policy,
        capabilities: tables.capabilities,
        assets: tables.assets,
        debts: tables.debts,
        bindings: tables.bindings,
        ops: lowered,
    })
}
