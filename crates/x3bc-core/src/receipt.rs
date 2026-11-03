//! The trading receipt: what exactly ran, against what, at what cost, and how it ended.
//!
//! A receipt binds the source, the compiler, the compilation mode, the artifact, the chain and its
//! finality, every capability version, the gas, every cost by kind and asset, the oracle readings,
//! the host-call transcript, the route, the balances before and after, the debts, the profit, the
//! outcome and the state commitments. It also carries the pre-state the trade was seeded with, so a
//! replay re-executes the artifact and compares — it never takes a total from the receipt on trust.
//!
//! The encoding is canonical (`crate::codec`): one receipt, one byte string, one digest.

use alloc::string::String;
use alloc::vec::Vec;

use crate::codec::{CodecError, Reader, Writer};
use crate::hash::sha256_domain;
use crate::state::{
    read_anchor, read_observation, read_pre_state, write_anchor, write_observation,
    write_pre_state, ChainAnchor, OracleObservation, TradingPreState,
};
use crate::trading::{CapabilityKind, CompilationModeId};
use crate::trap::TrapId;

/// Receipt format version.
pub const RECEIPT_FORMAT_VERSION: u16 = 1;

/// Domain tag for receipt digests and attestor signatures.
pub const RECEIPT_DOMAIN: &[u8] = b"x3-trading-receipt-v1";

/// Every cost category a trade can be charged.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum CostKind {
    /// VM gas, priced by the chain's gas price.
    Gas = 0,
    /// The flash provider's fee (paid inside the repayment).
    FlashFee = 1,
    /// The venue's liquidity fee (taken from the swap input).
    VenueFee = 2,
    ProtocolFee = 3,
    SettlementFee = 4,
    SolverFee = 5,
    /// The private-orderflow relay's fee.
    RelayFee = 6,
    /// Per oracle reading.
    OracleFee = 7,
    CrossDomainFee = 8,
}

impl CostKind {
    pub const ALL: [CostKind; 9] = [
        CostKind::Gas,
        CostKind::FlashFee,
        CostKind::VenueFee,
        CostKind::ProtocolFee,
        CostKind::SettlementFee,
        CostKind::SolverFee,
        CostKind::RelayFee,
        CostKind::OracleFee,
        CostKind::CrossDomainFee,
    ];

    /// The bit this kind occupies in a policy's allowed-cost mask.
    pub const fn bit(self) -> u16 {
        1 << (self as u8)
    }

    /// Kinds the chain itself levies on every trade it runs; a policy cannot opt out of them, only
    /// bound them with its cost ceiling.
    pub const fn chain_mandated(self) -> bool {
        matches!(
            self,
            CostKind::Gas
                | CostKind::ProtocolFee
                | CostKind::SettlementFee
                | CostKind::RelayFee
                | CostKind::OracleFee
        )
    }

    pub fn from_u8(value: u8) -> Option<CostKind> {
        CostKind::ALL
            .iter()
            .copied()
            .find(|kind| *kind as u8 == value)
    }

    pub const fn name(self) -> &'static str {
        match self {
            CostKind::Gas => "gas",
            CostKind::FlashFee => "flash_fee",
            CostKind::VenueFee => "venue_fee",
            CostKind::ProtocolFee => "protocol_fee",
            CostKind::SettlementFee => "settlement_fee",
            CostKind::SolverFee => "solver_fee",
            CostKind::RelayFee => "relay_fee",
            CostKind::OracleFee => "oracle_fee",
            CostKind::CrossDomainFee => "cross_domain_fee",
        }
    }
}

/// One cost, by kind and asset.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CostEntry {
    pub kind: CostKind,
    pub asset: u32,
    pub amount: u128,
    /// `true` when the amount is already inside another movement (a venue fee inside a swap's
    /// input, a flash fee inside a repayment); `false` when it was debited on its own.
    pub embedded: bool,
}

/// What the engine did, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum HostCallKind {
    Begin = 0,
    Borrow = 1,
    OracleRead = 2,
    Swap = 3,
    Repay = 4,
    ChargeCosts = 5,
    AssertProfit = 6,
    AssertDebtsClosed = 7,
    AssertSolvent = 8,
    Commit = 9,
    LoadProfit = 10,
}

impl HostCallKind {
    const ALL: [HostCallKind; 11] = [
        HostCallKind::Begin,
        HostCallKind::Borrow,
        HostCallKind::OracleRead,
        HostCallKind::Swap,
        HostCallKind::Repay,
        HostCallKind::ChargeCosts,
        HostCallKind::AssertProfit,
        HostCallKind::AssertDebtsClosed,
        HostCallKind::AssertSolvent,
        HostCallKind::Commit,
        HostCallKind::LoadProfit,
    ];

    pub fn from_u8(value: u8) -> Option<HostCallKind> {
        HostCallKind::ALL
            .iter()
            .copied()
            .find(|kind| *kind as u8 == value)
    }
}

/// `op_index` for transcript entries that belong to no section operation (begin, commit).
pub const NO_OP: u16 = u16::MAX;

/// One transcript entry: the kind, the section operation it served, and the figures it moved
/// (for a swap: input, output, fee, reserves before and after; for a borrow: principal, fee).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostCall {
    pub kind: HostCallKind,
    pub op_index: u16,
    pub figures: Vec<u128>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DebtRecord {
    pub debt: String,
    pub asset: u32,
    pub principal: u128,
    pub fee: u128,
    pub repaid: u128,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfitRecord {
    pub asset: u32,
    /// Net change of the profit asset's trader balance, after every cost.
    pub net: i128,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Committed,
    RolledBack { trap: TrapId, op_index: u16 },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TradeEvent {
    pub name: String,
    pub values: Vec<u128>,
}

/// An account a committed trade moves value between.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Account {
    Trader,
    FlashPool(String),
    Venue(String),
    FeeCollector,
}

/// A value movement staged inside the VM and released to the chain only on commit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LedgerIntent {
    pub asset: u32,
    pub from: Account,
    pub to: Account,
    pub amount: u128,
}

/// How the route was chosen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum RouteSelection {
    /// The route is the one the source program states; no solver auction ran. Solver markets are
    /// refused at compile time (`X3BC-L…`), so this is the only selection a receipt can record.
    Compiled = 0,
}

/// One leg as requested by the program and as executed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RouteLeg {
    pub venue: String,
    pub from: u32,
    pub to: u32,
    pub min_out: u128,
    pub input: u128,
    pub output: u128,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RouteRecord {
    pub selection: RouteSelection,
    pub legs: Vec<RouteLeg>,
}

/// A capability the artifact required and the version the host offered for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapabilityUse {
    pub kind: CapabilityKind,
    pub name: String,
    pub required_version: u16,
    pub offered_version: Option<u16>,
}

/// The complete record of one trading execution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TradingReceipt {
    pub format_version: u16,
    pub source_hash: [u8; 32],
    pub compiler_version: String,
    pub mode: CompilationModeId,
    pub bytecode_hash: [u8; 32],
    pub bytecode_version: u32,
    pub chain_id: u64,
    pub anchor: ChainAnchor,
    pub trade_id: String,
    pub policy_id: String,
    pub capabilities: Vec<CapabilityUse>,
    pub gas_limit: u64,
    pub gas_used: u64,
    /// Gas the trade paid for: everything through the end of its function, fixed when costs were
    /// committed (before the profit floor). Zero for a trade that never reached that point.
    pub gas_charged: u64,
    pub costs: Vec<CostEntry>,
    /// The oracle observations the trade read.
    pub oracle: Vec<OracleObservation>,
    pub transcript: Vec<HostCall>,
    pub route: RouteRecord,
    pub pre_state: TradingPreState,
    pub pre_balances: Vec<(u32, u128)>,
    pub post_balances: Vec<(u32, u128)>,
    pub debts: Vec<DebtRecord>,
    pub profit: ProfitRecord,
    pub outcome: Outcome,
    pub events: Vec<TradeEvent>,
    pub intents: Vec<LedgerIntent>,
    pub pre_state_commitment: [u8; 32],
    pub post_state_commitment: [u8; 32],
}

fn write_account(w: &mut Writer, account: &Account) {
    match account {
        Account::Trader => w.u8(0),
        Account::FlashPool(name) => {
            w.u8(1);
            w.str(name);
        }
        Account::Venue(name) => {
            w.u8(2);
            w.str(name);
        }
        Account::FeeCollector => w.u8(3),
    }
}

fn read_account(r: &mut Reader<'_>) -> Result<Account, CodecError> {
    Ok(match r.u8()? {
        0 => Account::Trader,
        1 => Account::FlashPool(r.string()?),
        2 => Account::Venue(r.string()?),
        3 => Account::FeeCollector,
        other => return Err(CodecError::InvalidTag(other)),
    })
}

fn write_balances(w: &mut Writer, balances: &[(u32, u128)]) {
    w.seq(balances, |w, (asset, amount)| {
        w.u32(*asset);
        w.u128(*amount);
    });
}

fn read_balances(r: &mut Reader<'_>) -> Result<Vec<(u32, u128)>, CodecError> {
    r.seq(|r| Ok((r.u32()?, r.u128()?)))
}

/// Canonical bytes of a receipt.
pub fn encode(receipt: &TradingReceipt) -> Vec<u8> {
    let mut w = Writer::new();
    let r = receipt;
    w.u16(r.format_version);
    w.array32(&r.source_hash);
    w.str(&r.compiler_version);
    w.u8(r.mode as u8);
    w.array32(&r.bytecode_hash);
    w.u32(r.bytecode_version);
    w.u64(r.chain_id);
    write_anchor(&mut w, &r.anchor);
    w.str(&r.trade_id);
    w.str(&r.policy_id);
    w.seq(&r.capabilities, |w, cap| {
        w.u8(cap.kind as u8);
        w.str(&cap.name);
        w.u16(cap.required_version);
        w.option(&cap.offered_version, |w, v| w.u16(*v));
    });
    w.u64(r.gas_limit);
    w.u64(r.gas_used);
    w.u64(r.gas_charged);
    w.seq(&r.costs, |w, cost| {
        w.u8(cost.kind as u8);
        w.u32(cost.asset);
        w.u128(cost.amount);
        w.bool(cost.embedded);
    });
    w.seq(&r.oracle, write_observation);
    w.seq(&r.transcript, |w, call| {
        w.u8(call.kind as u8);
        w.u16(call.op_index);
        w.seq(&call.figures, |w, figure| w.u128(*figure));
    });
    w.u8(r.route.selection as u8);
    w.seq(&r.route.legs, |w, leg| {
        w.str(&leg.venue);
        w.u32(leg.from);
        w.u32(leg.to);
        w.u128(leg.min_out);
        w.u128(leg.input);
        w.u128(leg.output);
    });
    write_pre_state(&mut w, &r.pre_state);
    write_balances(&mut w, &r.pre_balances);
    write_balances(&mut w, &r.post_balances);
    w.seq(&r.debts, |w, debt| {
        w.str(&debt.debt);
        w.u32(debt.asset);
        w.u128(debt.principal);
        w.u128(debt.fee);
        w.u128(debt.repaid);
    });
    w.u32(r.profit.asset);
    w.i128(r.profit.net);
    match &r.outcome {
        Outcome::Committed => w.u8(0),
        Outcome::RolledBack { trap, op_index } => {
            w.u8(1);
            w.u16(trap.code());
            w.u16(*op_index);
        }
    }
    w.seq(&r.events, |w, event| {
        w.str(&event.name);
        w.seq(&event.values, |w, value| w.u128(*value));
    });
    w.seq(&r.intents, |w, intent| {
        w.u32(intent.asset);
        write_account(w, &intent.from);
        write_account(w, &intent.to);
        w.u128(intent.amount);
    });
    w.array32(&r.pre_state_commitment);
    w.array32(&r.post_state_commitment);
    w.finish()
}

/// Read a receipt from its canonical bytes. Truncated, padded or malformed bytes are refused.
pub fn decode(bytes: &[u8]) -> Result<TradingReceipt, CodecError> {
    let mut r = Reader::new(bytes);
    let format_version = r.u16()?;
    if format_version != RECEIPT_FORMAT_VERSION {
        return Err(CodecError::InvalidTag(format_version as u8));
    }
    let source_hash = r.array32()?;
    let compiler_version = r.string()?;
    let mode_byte = r.u8()?;
    let mode = CompilationModeId::from_u8(mode_byte).ok_or(CodecError::InvalidTag(mode_byte))?;
    let bytecode_hash = r.array32()?;
    let bytecode_version = r.u32()?;
    let chain_id = r.u64()?;
    let anchor = read_anchor(&mut r)?;
    let trade_id = r.string()?;
    let policy_id = r.string()?;
    let capabilities = r.seq(|r| {
        let kind_byte = r.u8()?;
        Ok(CapabilityUse {
            kind: CapabilityKind::from_u8(kind_byte).ok_or(CodecError::InvalidTag(kind_byte))?,
            name: r.string()?,
            required_version: r.u16()?,
            offered_version: r.option(|r| r.u16())?,
        })
    })?;
    let gas_limit = r.u64()?;
    let gas_used = r.u64()?;
    let gas_charged = r.u64()?;
    let costs = r.seq(|r| {
        let kind_byte = r.u8()?;
        Ok(CostEntry {
            kind: CostKind::from_u8(kind_byte).ok_or(CodecError::InvalidTag(kind_byte))?,
            asset: r.u32()?,
            amount: r.u128()?,
            embedded: r.bool()?,
        })
    })?;
    let oracle = r.seq(read_observation)?;
    let transcript = r.seq(|r| {
        let kind_byte = r.u8()?;
        Ok(HostCall {
            kind: HostCallKind::from_u8(kind_byte).ok_or(CodecError::InvalidTag(kind_byte))?,
            op_index: r.u16()?,
            figures: r.seq(|r| r.u128())?,
        })
    })?;
    let selection = match r.u8()? {
        0 => RouteSelection::Compiled,
        other => return Err(CodecError::InvalidTag(other)),
    };
    let legs = r.seq(|r| {
        Ok(RouteLeg {
            venue: r.string()?,
            from: r.u32()?,
            to: r.u32()?,
            min_out: r.u128()?,
            input: r.u128()?,
            output: r.u128()?,
        })
    })?;
    let pre_state = read_pre_state(&mut r)?;
    let pre_balances = read_balances(&mut r)?;
    let post_balances = read_balances(&mut r)?;
    let debts = r.seq(|r| {
        Ok(DebtRecord {
            debt: r.string()?,
            asset: r.u32()?,
            principal: r.u128()?,
            fee: r.u128()?,
            repaid: r.u128()?,
        })
    })?;
    let profit = ProfitRecord {
        asset: r.u32()?,
        net: r.i128()?,
    };
    let outcome = match r.u8()? {
        0 => Outcome::Committed,
        1 => {
            let code = r.u16()?;
            Outcome::RolledBack {
                trap: TrapId::from_code(code).ok_or(CodecError::InvalidTag(code as u8))?,
                op_index: r.u16()?,
            }
        }
        other => return Err(CodecError::InvalidTag(other)),
    };
    let events = r.seq(|r| {
        Ok(TradeEvent {
            name: r.string()?,
            values: r.seq(|r| r.u128())?,
        })
    })?;
    let intents = r.seq(|r| {
        Ok(LedgerIntent {
            asset: r.u32()?,
            from: read_account(r)?,
            to: read_account(r)?,
            amount: r.u128()?,
        })
    })?;
    let pre_state_commitment = r.array32()?;
    let post_state_commitment = r.array32()?;
    r.finish()?;
    Ok(TradingReceipt {
        format_version,
        source_hash,
        compiler_version,
        mode,
        bytecode_hash,
        bytecode_version,
        chain_id,
        anchor,
        trade_id,
        policy_id,
        capabilities,
        gas_limit,
        gas_used,
        gas_charged,
        costs,
        oracle,
        transcript,
        route: RouteRecord { selection, legs },
        pre_state,
        pre_balances,
        post_balances,
        debts,
        profit,
        outcome,
        events,
        intents,
        pre_state_commitment,
        post_state_commitment,
    })
}

/// The digest an attestor signs: the receipt's canonical bytes under [`RECEIPT_DOMAIN`].
pub fn digest(receipt: &TradingReceipt) -> [u8; 32] {
    sha256_domain(RECEIPT_DOMAIN, &encode(receipt))
}

/// The name of the first field in which two receipts differ, in the order a replay reports them.
pub fn first_divergence(
    expected: &TradingReceipt,
    claimed: &TradingReceipt,
) -> Option<&'static str> {
    macro_rules! field {
        ($name:literal, $($path:tt)+) => {
            if expected.$($path)+ != claimed.$($path)+ {
                return Some($name);
            }
        };
    }
    field!("outcome", outcome);
    field!("costs", costs);
    field!("profit", profit);
    field!("debts", debts);
    field!("post_balances", post_balances);
    field!("pre_balances", pre_balances);
    field!("route", route);
    field!("transcript", transcript);
    field!("intents", intents);
    field!("events", events);
    field!("oracle", oracle);
    field!("gas_used", gas_used);
    field!("gas_charged", gas_charged);
    field!("gas_limit", gas_limit);
    field!("capabilities", capabilities);
    field!("pre_state_commitment", pre_state_commitment);
    field!("post_state_commitment", post_state_commitment);
    field!("pre_state", pre_state);
    field!("anchor", anchor);
    field!("chain_id", chain_id);
    field!("trade_id", trade_id);
    field!("policy_id", policy_id);
    field!("source_hash", source_hash);
    field!("compiler_version", compiler_version);
    field!("mode", mode);
    field!("bytecode_hash", bytecode_hash);
    field!("bytecode_version", bytecode_version);
    field!("format_version", format_version);
    None
}
