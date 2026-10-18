//! The X3BC trading extension (format 1.1): the section a trading artifact carries, its canonical
//! encoding, the stateful static verifier every reader runs, and the catalog of chains,
//! capabilities and assets the X3 chain offers.
//!
//! The section is data the trading opcodes index (`opcode.rs`, `0xE0–0xE9`): the policy compiled
//! from the source, the capabilities the program requires (with versions), the assets it touches,
//! and one entry per trading statement. Nothing in it is executable on its own; the code stream
//! decides what runs, and the verifier holds the two to each other.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use crate::bytecode::{
    FEATURE_PRIVATE_SUBMISSION_REQUIRED, FEATURE_TRADING_EXTENSION, TRADING_VERSION,
};
use crate::codec::{CodecError, Reader, Writer};
use crate::format::{BytecodeModule, FeatureFlags, FunctionEntry, VersionInfo};
use crate::opcode::Opcode;
use crate::receipt::CostKind;

/// Trading section format version.
pub const TRADING_SECTION_VERSION: u16 = 1;

/// The X3 chain, as the source names it and as the runtime reports it (`runtime` `ChainId`).
pub const X3_CHAIN_NAME: &str = "x3chain";
pub const X3_CHAIN_ID: u64 = 650_000;

/// The interface version of every capability this engine implements.
pub const CAPABILITY_VERSION: u16 = 1;

/// The chain's flash-liquidity provider (`pallet_x3_flashloan` asset pools).
pub const FLASH_PROVIDER: &str = "x3_flashloan";
/// Venue names are `x3dex_pool_<pool id>` (`pallet_x3_dex` pools).
pub const VENUE_PREFIX: &str = "x3dex_pool_";
/// The chain's oracle (`pallet_x3_oracle`).
pub const ORACLE: &str = "x3_oracle";
/// Asset canonical ids on the X3 chain are `asset_<CanonicalLedger asset id>`.
pub const ASSET_PREFIX: &str = "asset_";

/// Upper bounds a section must respect, so a verifier's work is bounded by the section's size.
pub const MAX_OPS: usize = 64;
pub const MAX_ASSETS: usize = 16;
pub const MAX_CAPABILITIES: usize = 32;
pub const MAX_SLOTS: usize = 16;

/// The numeric chain id for a chain name in a source program.
pub fn chain_id_for(name: &str) -> Option<u64> {
    (name == X3_CHAIN_NAME).then_some(X3_CHAIN_ID)
}

/// The `pallet_x3_dex` pool id a venue name denotes.
pub fn venue_pool_id(name: &str) -> Option<u64> {
    let digits = name.strip_prefix(VENUE_PREFIX)?;
    if digits.is_empty() || digits.starts_with('0') && digits.len() > 1 {
        return None;
    }
    digits.parse().ok()
}

/// The `CanonicalLedger` asset id an X3-chain canonical id denotes, by checked conversion.
pub fn asset_id_for(canonical_id: &str) -> Option<u32> {
    let digits = canonical_id.strip_prefix(ASSET_PREFIX)?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let wide: u64 = digits.parse().ok()?;
    u32::try_from(wide).ok()
}

/// The compilation mode the artifact was built in, carried into its receipt.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum CompilationModeId {
    Dev = 0,
    Testnet = 1,
    Mainnet = 2,
}

impl CompilationModeId {
    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::Dev),
            1 => Some(Self::Testnet),
            2 => Some(Self::Mainnet),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum CapabilityKind {
    FlashProvider = 0,
    Venue = 1,
    Oracle = 2,
}

impl CapabilityKind {
    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::FlashProvider),
            1 => Some(Self::Venue),
            2 => Some(Self::Oracle),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapabilityRequirement {
    pub kind: CapabilityKind,
    pub name: String,
    pub version: u16,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssetRef {
    /// `CanonicalLedger` asset id.
    pub id: u32,
    pub decimals: u8,
    pub symbol: String,
}

/// The risk policy compiled from the source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TradingPolicy {
    pub policy_id: String,
    pub policy_version: u16,
    pub max_slippage_bps: u16,
    /// Ceiling on the sum of every cost denominated in `max_cost_asset` (the source's `max_gas`).
    pub max_cost: u128,
    /// Index into the section's assets.
    pub max_cost_asset: u16,
    pub max_flash_fee_bps: u16,
    /// Counted from the block the submission was signed against.
    pub deadline_blocks: u64,
    pub private_submission_required: bool,
    pub quote_freshness_blocks: Option<u64>,
    pub max_oracle_deviation_bps: Option<u16>,
    pub max_price_impact_bps: Option<u16>,
    /// Bitmask of `CostKind::bit` the program admits beyond the chain-mandated kinds.
    pub allowed_cost_kinds: u16,
}

/// Where a swap's input comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Amount {
    Literal(u128),
    /// The output of an earlier swap, by binding slot.
    Binding(u8),
    /// An open debt's principal, by debt slot.
    DebtPrincipal(u8),
}

/// One trading statement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TradeOp {
    Borrow {
        debt: u8,
        provider: u16,
        asset: u16,
        principal: u128,
    },
    Swap {
        binding: u8,
        venue: u16,
        from: u16,
        to: u16,
        input: Amount,
        min_out: u128,
    },
    Repay {
        debt: u8,
    },
    ChargeCosts,
    AssertMinProfit {
        asset: u16,
        minimum: u128,
    },
    AssertAllDebtsClosed,
    AssertSolvent,
}

impl TradeOp {
    pub fn name(&self) -> &'static str {
        match self {
            TradeOp::Borrow { .. } => "borrow",
            TradeOp::Swap { .. } => "swap",
            TradeOp::Repay { .. } => "repay",
            TradeOp::ChargeCosts => "charge_costs",
            TradeOp::AssertMinProfit { .. } => "assert_min_profit",
            TradeOp::AssertAllDebtsClosed => "assert_all_debts_closed",
            TradeOp::AssertSolvent => "assert_solvent",
        }
    }

    /// The opcode that executes this operation.
    pub const fn opcode(&self) -> Opcode {
        match self {
            TradeOp::Borrow { .. } => Opcode::TradeBorrow,
            TradeOp::Swap { .. } => Opcode::TradeSwap,
            TradeOp::Repay { .. } => Opcode::TradeRepay,
            TradeOp::ChargeCosts => Opcode::TradeChargeCosts,
            TradeOp::AssertMinProfit { .. } => Opcode::TradeAssertProfit,
            TradeOp::AssertAllDebtsClosed => Opcode::TradeAssertDebtsClosed,
            TradeOp::AssertSolvent => Opcode::TradeAssertSolvent,
        }
    }
}

/// The trading section of a format-1.1 module.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TradingSection {
    pub version: u16,
    pub chain_id: u64,
    pub mode: CompilationModeId,
    pub source_hash: [u8; 32],
    pub compiler_version: String,
    pub trade_id: String,
    pub policy: TradingPolicy,
    pub capabilities: Vec<CapabilityRequirement>,
    pub assets: Vec<AssetRef>,
    /// Debt slot names, by slot.
    pub debts: Vec<String>,
    /// Binding slot names, by slot.
    pub bindings: Vec<String>,
    pub ops: Vec<TradeOp>,
}

impl TradingSection {
    /// The asset of the profit floor (the first `AssertMinProfit`), which is the asset a receipt
    /// reports profit in.
    pub fn profit_asset(&self) -> Option<u32> {
        self.ops.iter().find_map(|op| match op {
            TradeOp::AssertMinProfit { asset, .. } => {
                self.assets.get(*asset as usize).map(|a| a.id)
            }
            _ => None,
        })
    }
}

// ---------------------------------------------------------------- encoding

fn write_amount(w: &mut Writer, amount: &Amount) {
    match amount {
        Amount::Literal(value) => {
            w.u8(0);
            w.u128(*value);
        }
        Amount::Binding(slot) => {
            w.u8(1);
            w.u8(*slot);
        }
        Amount::DebtPrincipal(slot) => {
            w.u8(2);
            w.u8(*slot);
        }
    }
}

fn read_amount(r: &mut Reader<'_>) -> Result<Amount, CodecError> {
    Ok(match r.u8()? {
        0 => Amount::Literal(r.u128()?),
        1 => Amount::Binding(r.u8()?),
        2 => Amount::DebtPrincipal(r.u8()?),
        other => return Err(CodecError::InvalidTag(other)),
    })
}

/// Canonical bytes of a trading section.
pub fn encode_section(section: &TradingSection) -> Vec<u8> {
    let mut w = Writer::new();
    let s = section;
    w.u16(s.version);
    w.u64(s.chain_id);
    w.u8(s.mode as u8);
    w.array32(&s.source_hash);
    w.str(&s.compiler_version);
    w.str(&s.trade_id);
    let p = &s.policy;
    w.str(&p.policy_id);
    w.u16(p.policy_version);
    w.u16(p.max_slippage_bps);
    w.u128(p.max_cost);
    w.u16(p.max_cost_asset);
    w.u16(p.max_flash_fee_bps);
    w.u64(p.deadline_blocks);
    w.bool(p.private_submission_required);
    w.option(&p.quote_freshness_blocks, |w, v| w.u64(*v));
    w.option(&p.max_oracle_deviation_bps, |w, v| w.u16(*v));
    w.option(&p.max_price_impact_bps, |w, v| w.u16(*v));
    w.u16(p.allowed_cost_kinds);
    w.seq(&s.capabilities, |w, cap| {
        w.u8(cap.kind as u8);
        w.str(&cap.name);
        w.u16(cap.version);
    });
    w.seq(&s.assets, |w, asset| {
        w.u32(asset.id);
        w.u8(asset.decimals);
        w.str(&asset.symbol);
    });
    w.seq(&s.debts, |w, name| w.str(name));
    w.seq(&s.bindings, |w, name| w.str(name));
    w.seq(&s.ops, |w, op| match op {
        TradeOp::Borrow {
            debt,
            provider,
            asset,
            principal,
        } => {
            w.u8(0);
            w.u8(*debt);
            w.u16(*provider);
            w.u16(*asset);
            w.u128(*principal);
        }
        TradeOp::Swap {
            binding,
            venue,
            from,
            to,
            input,
            min_out,
        } => {
            w.u8(1);
            w.u8(*binding);
            w.u16(*venue);
            w.u16(*from);
            w.u16(*to);
            write_amount(w, input);
            w.u128(*min_out);
        }
        TradeOp::Repay { debt } => {
            w.u8(2);
            w.u8(*debt);
        }
        TradeOp::ChargeCosts => w.u8(3),
        TradeOp::AssertMinProfit { asset, minimum } => {
            w.u8(4);
            w.u16(*asset);
            w.u128(*minimum);
        }
        TradeOp::AssertAllDebtsClosed => w.u8(5),
        TradeOp::AssertSolvent => w.u8(6),
    });
    w.finish()
}

/// Read a trading section from its canonical bytes (structure only; [`verify_section`] judges it).
pub fn decode_section(bytes: &[u8]) -> Result<TradingSection, CodecError> {
    let mut r = Reader::new(bytes);
    let version = r.u16()?;
    let chain_id = r.u64()?;
    let mode_byte = r.u8()?;
    let mode = CompilationModeId::from_u8(mode_byte).ok_or(CodecError::InvalidTag(mode_byte))?;
    let source_hash = r.array32()?;
    let compiler_version = r.string()?;
    let trade_id = r.string()?;
    let policy = TradingPolicy {
        policy_id: r.string()?,
        policy_version: r.u16()?,
        max_slippage_bps: r.u16()?,
        max_cost: r.u128()?,
        max_cost_asset: r.u16()?,
        max_flash_fee_bps: r.u16()?,
        deadline_blocks: r.u64()?,
        private_submission_required: r.bool()?,
        quote_freshness_blocks: r.option(|r| r.u64())?,
        max_oracle_deviation_bps: r.option(|r| r.u16())?,
        max_price_impact_bps: r.option(|r| r.u16())?,
        allowed_cost_kinds: r.u16()?,
    };
    let capabilities = r.seq(|r| {
        let kind = r.u8()?;
        Ok(CapabilityRequirement {
            kind: CapabilityKind::from_u8(kind).ok_or(CodecError::InvalidTag(kind))?,
            name: r.string()?,
            version: r.u16()?,
        })
    })?;
    let assets = r.seq(|r| {
        Ok(AssetRef {
            id: r.u32()?,
            decimals: r.u8()?,
            symbol: r.string()?,
        })
    })?;
    let debts = r.seq(|r| r.string())?;
    let bindings = r.seq(|r| r.string())?;
    let ops = r.seq(|r| {
        Ok(match r.u8()? {
            0 => TradeOp::Borrow {
                debt: r.u8()?,
                provider: r.u16()?,
                asset: r.u16()?,
                principal: r.u128()?,
            },
            1 => TradeOp::Swap {
                binding: r.u8()?,
                venue: r.u16()?,
                from: r.u16()?,
                to: r.u16()?,
                input: read_amount(r)?,
                min_out: r.u128()?,
            },
            2 => TradeOp::Repay { debt: r.u8()? },
            3 => TradeOp::ChargeCosts,
            4 => TradeOp::AssertMinProfit {
                asset: r.u16()?,
                minimum: r.u128()?,
            },
            5 => TradeOp::AssertAllDebtsClosed,
            6 => TradeOp::AssertSolvent,
            other => return Err(CodecError::InvalidTag(other)),
        })
    })?;
    r.finish()?;
    Ok(TradingSection {
        version,
        chain_id,
        mode,
        source_hash,
        compiler_version,
        trade_id,
        policy,
        capabilities,
        assets,
        debts,
        bindings,
        ops,
    })
}

// ---------------------------------------------------------------- static verification

/// Why a trading section is not a sound program.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SectionError {
    /// The operation at fault, when there is one.
    pub op_index: Option<u16>,
    pub reason: &'static str,
}

fn fail(op_index: Option<usize>, reason: &'static str) -> SectionError {
    SectionError {
        op_index: op_index.and_then(|i| u16::try_from(i).ok()),
        reason,
    }
}

const BPS_MAX: u16 = 10_000;

/// The stateful static verifier: bindings and debts resolve, every index is in range, every
/// capability exists at the version this engine implements, costs are charged exactly once and
/// before every profit floor, every debt opened is repaid, and the program has a profit floor.
///
/// The interpreter re-checks the dynamic half (amounts, balances, prices); this is the half that
/// does not depend on the chain's state, refused before anything runs.
pub fn verify_section(s: &TradingSection) -> Result<(), SectionError> {
    if s.version != TRADING_SECTION_VERSION {
        return Err(fail(None, "unsupported trading section version"));
    }
    if s.ops.is_empty() || s.ops.len() > MAX_OPS {
        return Err(fail(None, "operation count out of range"));
    }
    if s.assets.is_empty() || s.assets.len() > MAX_ASSETS {
        return Err(fail(None, "asset count out of range"));
    }
    if s.capabilities.len() > MAX_CAPABILITIES
        || s.debts.len() > MAX_SLOTS
        || s.bindings.len() > MAX_SLOTS
    {
        return Err(fail(None, "table size out of range"));
    }
    if s.policy.policy_version != 1 {
        return Err(fail(None, "unsupported policy version"));
    }
    let p = &s.policy;
    for bps in [
        Some(p.max_slippage_bps),
        Some(p.max_flash_fee_bps),
        p.max_oracle_deviation_bps,
        p.max_price_impact_bps,
    ]
    .into_iter()
    .flatten()
    {
        if bps > BPS_MAX {
            return Err(fail(None, "basis points above 10000"));
        }
    }
    if p.deadline_blocks == 0 {
        return Err(fail(None, "deadline must be at least one block"));
    }
    if p.quote_freshness_blocks == Some(0) {
        return Err(fail(None, "quote freshness must be at least one block"));
    }
    if usize::from(p.max_cost_asset) >= s.assets.len() {
        return Err(fail(None, "cost ceiling asset out of range"));
    }
    let known_cost_bits = CostKind::ALL
        .iter()
        .fold(0u16, |mask, kind| mask | kind.bit());
    if p.allowed_cost_kinds & !known_cost_bits != 0 {
        return Err(fail(None, "unknown cost kind admitted"));
    }

    for (i, a) in s.assets.iter().enumerate() {
        if s.assets[..i].iter().any(|b| b.id == a.id) {
            return Err(fail(None, "duplicate asset"));
        }
    }
    let mut has_oracle = false;
    for (i, cap) in s.capabilities.iter().enumerate() {
        if s.capabilities[..i]
            .iter()
            .any(|c| c.kind == cap.kind && c.name == cap.name)
        {
            return Err(fail(None, "duplicate capability"));
        }
        if cap.version != CAPABILITY_VERSION {
            return Err(fail(None, "unsupported capability version"));
        }
        let known = match cap.kind {
            CapabilityKind::FlashProvider => cap.name == FLASH_PROVIDER,
            CapabilityKind::Venue => venue_pool_id(&cap.name).is_some(),
            CapabilityKind::Oracle => {
                has_oracle = true;
                cap.name == ORACLE
            }
        };
        if !known {
            return Err(fail(None, "capability not in the chain's catalog"));
        }
    }
    if has_oracle != p.max_oracle_deviation_bps.is_some() {
        return Err(fail(
            None,
            "an oracle capability exactly when the policy bounds oracle deviation",
        ));
    }

    // Dataflow over the operations.
    let mut debt_open = vec![false; s.debts.len()];
    let mut debt_used = vec![false; s.debts.len()];
    let mut debt_asset = vec![0u16; s.debts.len()];
    let mut binding_asset: Vec<Option<u16>> = vec![None; s.bindings.len()];
    let mut charged_at: Option<usize> = None;
    let mut last_movement: Option<usize> = None;
    let mut profit_floors = 0usize;
    let cap_kind = |index: u16| s.capabilities.get(usize::from(index)).map(|c| c.kind);
    let asset_ok = |index: u16| usize::from(index) < s.assets.len();

    for (i, op) in s.ops.iter().enumerate() {
        let at = Some(i);
        match op {
            TradeOp::Borrow {
                debt,
                provider,
                asset,
                principal,
            } => {
                let d = usize::from(*debt);
                if d >= s.debts.len() {
                    return Err(fail(at, "debt slot out of range"));
                }
                if debt_used[d] {
                    return Err(fail(at, "debt slot opened twice"));
                }
                if cap_kind(*provider) != Some(CapabilityKind::FlashProvider) {
                    return Err(fail(at, "borrow names no flash provider"));
                }
                if !asset_ok(*asset) {
                    return Err(fail(at, "asset out of range"));
                }
                if *principal == 0 {
                    return Err(fail(at, "zero principal"));
                }
                if charged_at.is_some() {
                    return Err(fail(at, "borrow after costs were charged"));
                }
                debt_used[d] = true;
                debt_open[d] = true;
                debt_asset[d] = *asset;
                last_movement = Some(i);
            }
            TradeOp::Swap {
                binding,
                venue,
                from,
                to,
                input,
                min_out,
            } => {
                let b = usize::from(*binding);
                if b >= s.bindings.len() {
                    return Err(fail(at, "binding slot out of range"));
                }
                if binding_asset[b].is_some() {
                    return Err(fail(at, "binding written twice"));
                }
                if cap_kind(*venue) != Some(CapabilityKind::Venue) {
                    return Err(fail(at, "swap names no venue"));
                }
                if !asset_ok(*from) || !asset_ok(*to) || from == to {
                    return Err(fail(at, "swap assets out of range or identical"));
                }
                if *min_out == 0 {
                    return Err(fail(at, "swap without a minimum output"));
                }
                match input {
                    Amount::Literal(value) => {
                        if *value == 0 {
                            return Err(fail(at, "zero swap input"));
                        }
                    }
                    Amount::Binding(slot) => match binding_asset.get(usize::from(*slot)) {
                        Some(Some(asset)) if asset == from => {}
                        Some(Some(_)) => return Err(fail(at, "binding holds a different asset")),
                        _ => return Err(fail(at, "unresolved binding")),
                    },
                    Amount::DebtPrincipal(slot) => {
                        let d = usize::from(*slot);
                        if d >= s.debts.len() || !debt_open[d] {
                            return Err(fail(at, "unresolved debt"));
                        }
                        if debt_asset[d] != *from {
                            return Err(fail(at, "debt holds a different asset"));
                        }
                    }
                }
                if charged_at.is_some() {
                    return Err(fail(at, "swap after costs were charged"));
                }
                binding_asset[b] = Some(*to);
                last_movement = Some(i);
            }
            TradeOp::Repay { debt } => {
                let d = usize::from(*debt);
                if d >= s.debts.len() || !debt_open[d] {
                    return Err(fail(at, "repay of a debt that is not open"));
                }
                if charged_at.is_some() {
                    return Err(fail(at, "repay after costs were charged"));
                }
                debt_open[d] = false;
                last_movement = Some(i);
            }
            TradeOp::ChargeCosts => {
                if charged_at.is_some() {
                    return Err(fail(at, "costs charged twice"));
                }
                charged_at = Some(i);
            }
            TradeOp::AssertMinProfit { asset, .. } => {
                if !asset_ok(*asset) {
                    return Err(fail(at, "asset out of range"));
                }
                if charged_at.is_none() {
                    return Err(fail(at, "profit evaluated before every cost was charged"));
                }
                profit_floors += 1;
            }
            TradeOp::AssertAllDebtsClosed | TradeOp::AssertSolvent => {}
        }
    }
    if debt_open.iter().any(|open| *open) {
        return Err(fail(None, "unresolved debt: a borrow is never repaid"));
    }
    if profit_floors == 0 {
        return Err(fail(None, "no profit floor"));
    }
    match (charged_at, last_movement) {
        (Some(charged), Some(moved)) if charged > moved => {}
        _ => {
            return Err(fail(
                None,
                "costs must be charged after the last balance movement",
            ))
        }
    }
    // Every declared slot is used, so a receipt never names a debt or binding that does not exist.
    if debt_used.iter().any(|used| !used) || binding_asset.iter().any(Option::is_none) {
        return Err(fail(None, "declared slot never used"));
    }
    Ok(())
}

// ---------------------------------------------------------------- module assembly

/// Write a trading module around `code`: one entry function (`main`, no parameters), format 1.1,
/// the trading feature bit, the private-submission bit when the policy demands it, and the section.
///
/// The canonical compiler and test assemblers both use this, so the container a trade travels in is
/// written in exactly one place.
pub fn assemble_trading_module(section: TradingSection, code: Vec<u8>) -> Vec<u8> {
    let mut module = BytecodeModule::new();
    module.version = VersionInfo::from_packed(TRADING_VERSION);
    module.min_version = VersionInfo::from_packed(TRADING_VERSION);
    let mut features = FeatureFlags::new();
    features.set(FEATURE_TRADING_EXTENSION);
    if section.policy.private_submission_required {
        features.set(FEATURE_PRIVATE_SUBMISSION_REQUIRED);
    }
    module.features = features;
    module.functions.push(FunctionEntry {
        name: "main".into(),
        entry_point: 0,
        param_count: 0,
        local_count: 16,
        max_stack: 0,
        return_type_tag: 1,
    });
    module.code = code;
    module.trading = Some(section);
    module.to_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn flagship() -> TradingSection {
        TradingSection {
            version: TRADING_SECTION_VERSION,
            chain_id: X3_CHAIN_ID,
            mode: CompilationModeId::Testnet,
            source_hash: [1; 32],
            compiler_version: "0.1.0".into(),
            trade_id: "Arb".into(),
            policy: TradingPolicy {
                policy_id: "P".into(),
                policy_version: 1,
                max_slippage_bps: 50,
                max_cost: 20_000_000,
                max_cost_asset: 0,
                max_flash_fee_bps: 10,
                deadline_blocks: 5,
                private_submission_required: true,
                quote_freshness_blocks: Some(3),
                max_oracle_deviation_bps: Some(500),
                max_price_impact_bps: None,
                allowed_cost_kinds: CostKind::FlashFee.bit() | CostKind::VenueFee.bit(),
            },
            capabilities: vec![
                CapabilityRequirement {
                    kind: CapabilityKind::FlashProvider,
                    name: FLASH_PROVIDER.into(),
                    version: 1,
                },
                CapabilityRequirement {
                    kind: CapabilityKind::Venue,
                    name: "x3dex_pool_1".into(),
                    version: 1,
                },
                CapabilityRequirement {
                    kind: CapabilityKind::Venue,
                    name: "x3dex_pool_2".into(),
                    version: 1,
                },
                CapabilityRequirement {
                    kind: CapabilityKind::Oracle,
                    name: ORACLE.into(),
                    version: 1,
                },
            ],
            assets: vec![
                AssetRef {
                    id: 1,
                    decimals: 6,
                    symbol: "USDX".into(),
                },
                AssetRef {
                    id: 2,
                    decimals: 6,
                    symbol: "WETX".into(),
                },
            ],
            debts: vec!["debt".into()],
            bindings: vec!["wetx".into(), "usdx_back".into()],
            ops: vec![
                TradeOp::Borrow {
                    debt: 0,
                    provider: 0,
                    asset: 0,
                    principal: 1_000_000_000,
                },
                TradeOp::Swap {
                    binding: 0,
                    venue: 1,
                    from: 0,
                    to: 1,
                    input: Amount::DebtPrincipal(0),
                    min_out: 490_000_000,
                },
                TradeOp::Swap {
                    binding: 1,
                    venue: 2,
                    from: 1,
                    to: 0,
                    input: Amount::Binding(0),
                    min_out: 1_005_000_000,
                },
                TradeOp::Repay { debt: 0 },
                TradeOp::ChargeCosts,
                TradeOp::AssertMinProfit {
                    asset: 0,
                    minimum: 1_000_000,
                },
                TradeOp::AssertAllDebtsClosed,
            ],
        }
    }

    #[test]
    fn the_flagship_section_is_sound_and_round_trips() {
        let section = flagship();
        verify_section(&section).expect("sound");
        let bytes = encode_section(&section);
        assert_eq!(decode_section(&bytes), Ok(section));
        for cut in [0, 1, bytes.len() / 2, bytes.len() - 1] {
            assert!(decode_section(&bytes[..cut]).is_err());
        }
    }

    fn refused(mutate: impl FnOnce(&mut TradingSection), reason: &str) {
        let mut section = flagship();
        mutate(&mut section);
        let error = verify_section(&section).expect_err(reason);
        assert_eq!(error.reason, reason);
    }

    #[test]
    fn unresolved_and_misordered_programs_are_refused() {
        refused(
            |s| {
                s.ops.remove(3);
            },
            "unresolved debt: a borrow is never repaid",
        );
        refused(
            |s| {
                s.ops[2] = TradeOp::Swap {
                    binding: 1,
                    venue: 2,
                    from: 1,
                    to: 0,
                    input: Amount::Binding(1),
                    min_out: 1,
                }
            },
            "unresolved binding",
        );
        refused(
            |s| s.ops.swap(4, 5),
            "profit evaluated before every cost was charged",
        );
        refused(
            |s| {
                s.ops
                    .retain(|op| !matches!(op, TradeOp::AssertMinProfit { .. }))
            },
            "no profit floor",
        );
        refused(|s| s.ops.push(TradeOp::ChargeCosts), "costs charged twice");
        refused(
            |s| s.ops.insert(4, TradeOp::ChargeCosts),
            "costs charged twice",
        );
        refused(
            |s| s.capabilities[1].version = 2,
            "unsupported capability version",
        );
        refused(
            |s| s.capabilities[1].name = "uniswap".into(),
            "capability not in the chain's catalog",
        );
        refused(
            |s| s.policy.max_slippage_bps = 10_001,
            "basis points above 10000",
        );
        refused(
            |s| s.policy.deadline_blocks = 0,
            "deadline must be at least one block",
        );
        refused(
            |s| s.policy.max_oracle_deviation_bps = None,
            "an oracle capability exactly when the policy bounds oracle deviation",
        );
        refused(
            |s| {
                if let TradeOp::Swap { from, .. } = &mut s.ops[1] {
                    *from = 1;
                }
            },
            "swap assets out of range or identical",
        );
    }

    #[test]
    fn a_trading_module_round_trips_through_the_container() {
        let section = flagship();
        let bytes = assemble_trading_module(section.clone(), vec![Opcode::RetVoid as u8]);
        assert_eq!(crate::bytecode::min_version(&bytes), Some(TRADING_VERSION));
        assert!(crate::bytecode::requires_private_submission(&bytes));
        let module = BytecodeModule::from_bytes(&bytes).expect("reads");
        assert_eq!(module.trading, Some(section));
        assert!(module.features.has(FEATURE_TRADING_EXTENSION));
    }

    #[test]
    fn a_damaged_trading_module_is_refused() {
        let bytes = assemble_trading_module(flagship(), vec![Opcode::RetVoid as u8]);
        // Every truncation, resealed so the section reader (not the checksum) must refuse it.
        for cut in crate::bytecode::HEADER_LEN..bytes.len() {
            let mut short = bytes[..cut].to_vec();
            crate::bytecode::reseal(&mut short);
            assert!(BytecodeModule::from_bytes(&short).is_err(), "cut at {cut}");
        }
        // Trailing bytes after the section.
        let mut long = bytes.clone();
        long.push(0);
        crate::bytecode::reseal(&mut long);
        assert!(BytecodeModule::from_bytes(&long).is_err());
        // The feature bit without the section's demand of format 1.1.
        let mut downgraded = bytes.clone();
        downgraded[crate::bytecode::MIN_VERSION_OFFSET..crate::bytecode::MIN_VERSION_OFFSET + 4]
            .copy_from_slice(&crate::bytecode::VERSION.to_le_bytes());
        crate::bytecode::reseal(&mut downgraded);
        assert!(BytecodeModule::from_bytes(&downgraded).is_err());
    }

    #[test]
    fn catalog_lookups_are_checked() {
        assert_eq!(asset_id_for("asset_7"), Some(7));
        assert_eq!(asset_id_for("asset_4294967295"), Some(u32::MAX));
        assert_eq!(asset_id_for("asset_4294967296"), None);
        assert_eq!(asset_id_for("asset_"), None);
        assert_eq!(asset_id_for("asset_-1"), None);
        assert_eq!(asset_id_for("0xA0b8"), None);
        assert_eq!(venue_pool_id("x3dex_pool_12"), Some(12));
        assert_eq!(venue_pool_id("x3dex_pool_012"), None);
        assert_eq!(venue_pool_id("uniswap_v3"), None);
        assert_eq!(chain_id_for("x3chain"), Some(650_000));
        assert_eq!(chain_id_for("ethereum"), None);
    }
}
