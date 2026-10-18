//! The chain state a trading execution is seeded with, and where it ran.
//!
//! A block reads these from storage (`CanonicalLedger`, `pallet_x3_dex` pools, the flash pools,
//! `pallet_x3_oracle`, the runtime's cost schedule) and hands them to the engine; the engine never
//! reads chain storage itself. The same values travel in the receipt, which is what lets an
//! independent replay re-execute the trade from the artifact alone.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use crate::codec::{CodecError, Reader, Writer};
use crate::hash::sha256_domain;

/// A flash-liquidity pool a provider offers for one asset.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FlashPoolState {
    pub provider: String,
    /// Interface version the provider implements.
    pub version: u16,
    pub asset: u32,
    pub liquidity: u128,
    pub fee_bps: u16,
}

/// A constant-product venue.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VenueState {
    pub name: String,
    pub version: u16,
    pub asset_a: u32,
    pub asset_b: u32,
    pub reserve_a: u128,
    pub reserve_b: u128,
    pub fee_bps: u16,
}

/// One oracle reading: `price_num / price_den` units of `quote` per unit of `base`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OracleObservation {
    pub base: u32,
    pub quote: u32,
    pub price_num: u128,
    pub price_den: u128,
    pub observed_at_block: u64,
    pub source: String,
}

/// What the chain charges a trade, in `fee_asset` base units.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CostSchedule {
    pub fee_asset: u32,
    /// Per unit of gas charged.
    pub gas_price: u128,
    pub settlement_fee: u128,
    /// Charged when the trade is submitted through the private channel.
    pub relay_fee: u128,
    pub protocol_fee: u128,
    /// Per oracle observation read.
    pub oracle_fee: u128,
    pub solver_fee: u128,
    pub cross_domain_fee: u128,
}

/// Everything the engine is told about the world before the trade runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TradingPreState {
    pub chain_id: u64,
    /// The block the trade executes in.
    pub block_number: u64,
    /// The block the trader signed against; the compiled deadline counts from here.
    pub submitted_at_block: u64,
    pub trader: [u8; 32],
    pub trader_balances: BTreeMap<u32, u128>,
    pub flash_pools: Vec<FlashPoolState>,
    pub venues: Vec<VenueState>,
    pub oracle: Vec<OracleObservation>,
    pub costs: CostSchedule,
    /// Whether this submission arrived through a private channel.
    pub private_channel: bool,
}

/// The finalized header an attestor binds a receipt to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finality {
    pub block_number: u64,
    pub block_hash: [u8; 32],
    pub state_root: [u8; 32],
    pub parent_state_root: [u8; 32],
}

/// Where a trade ran: known inside the block, plus the finality an attestor adds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChainAnchor {
    pub spec_version: u32,
    pub transaction_version: u32,
    pub block_number: u64,
    pub parent_hash: [u8; 32],
    pub extrinsic_index: u32,
    pub finality: Option<Finality>,
}

pub(crate) fn write_pre_state(w: &mut Writer, pre: &TradingPreState) {
    w.u64(pre.chain_id);
    w.u64(pre.block_number);
    w.u64(pre.submitted_at_block);
    w.array32(&pre.trader);
    w.length(pre.trader_balances.len());
    for (asset, balance) in &pre.trader_balances {
        w.u32(*asset);
        w.u128(*balance);
    }
    w.seq(&pre.flash_pools, |w, pool| {
        w.str(&pool.provider);
        w.u16(pool.version);
        w.u32(pool.asset);
        w.u128(pool.liquidity);
        w.u16(pool.fee_bps);
    });
    w.seq(&pre.venues, |w, venue| {
        w.str(&venue.name);
        w.u16(venue.version);
        w.u32(venue.asset_a);
        w.u32(venue.asset_b);
        w.u128(venue.reserve_a);
        w.u128(venue.reserve_b);
        w.u16(venue.fee_bps);
    });
    w.seq(&pre.oracle, write_observation);
    let c = &pre.costs;
    w.u32(c.fee_asset);
    for amount in [
        c.gas_price,
        c.settlement_fee,
        c.relay_fee,
        c.protocol_fee,
        c.oracle_fee,
        c.solver_fee,
        c.cross_domain_fee,
    ] {
        w.u128(amount);
    }
    w.bool(pre.private_channel);
}

pub(crate) fn write_observation(w: &mut Writer, o: &OracleObservation) {
    w.u32(o.base);
    w.u32(o.quote);
    w.u128(o.price_num);
    w.u128(o.price_den);
    w.u64(o.observed_at_block);
    w.str(&o.source);
}

pub(crate) fn read_observation(r: &mut Reader<'_>) -> Result<OracleObservation, CodecError> {
    Ok(OracleObservation {
        base: r.u32()?,
        quote: r.u32()?,
        price_num: r.u128()?,
        price_den: r.u128()?,
        observed_at_block: r.u64()?,
        source: r.string()?,
    })
}

pub(crate) fn read_pre_state(r: &mut Reader<'_>) -> Result<TradingPreState, CodecError> {
    let chain_id = r.u64()?;
    let block_number = r.u64()?;
    let submitted_at_block = r.u64()?;
    let trader = r.array32()?;
    let count = r.length()?;
    let mut trader_balances = BTreeMap::new();
    let mut previous: Option<u32> = None;
    for _ in 0..count {
        let asset = r.u32()?;
        // Canonical: strictly ascending keys, so one map has one encoding.
        if previous.is_some_and(|p| p >= asset) {
            return Err(CodecError::InvalidTag(0));
        }
        previous = Some(asset);
        trader_balances.insert(asset, r.u128()?);
    }
    let flash_pools = r.seq(|r| {
        Ok(FlashPoolState {
            provider: r.string()?,
            version: r.u16()?,
            asset: r.u32()?,
            liquidity: r.u128()?,
            fee_bps: r.u16()?,
        })
    })?;
    let venues = r.seq(|r| {
        Ok(VenueState {
            name: r.string()?,
            version: r.u16()?,
            asset_a: r.u32()?,
            asset_b: r.u32()?,
            reserve_a: r.u128()?,
            reserve_b: r.u128()?,
            fee_bps: r.u16()?,
        })
    })?;
    let oracle = r.seq(read_observation)?;
    let costs = CostSchedule {
        fee_asset: r.u32()?,
        gas_price: r.u128()?,
        settlement_fee: r.u128()?,
        relay_fee: r.u128()?,
        protocol_fee: r.u128()?,
        oracle_fee: r.u128()?,
        solver_fee: r.u128()?,
        cross_domain_fee: r.u128()?,
    };
    let private_channel = r.bool()?;
    Ok(TradingPreState {
        chain_id,
        block_number,
        submitted_at_block,
        trader,
        trader_balances,
        flash_pools,
        venues,
        oracle,
        costs,
        private_channel,
    })
}

pub(crate) fn write_anchor(w: &mut Writer, a: &ChainAnchor) {
    w.u32(a.spec_version);
    w.u32(a.transaction_version);
    w.u64(a.block_number);
    w.array32(&a.parent_hash);
    w.u32(a.extrinsic_index);
    w.option(&a.finality, |w, f| {
        w.u64(f.block_number);
        w.array32(&f.block_hash);
        w.array32(&f.state_root);
        w.array32(&f.parent_state_root);
    });
}

pub(crate) fn read_anchor(r: &mut Reader<'_>) -> Result<ChainAnchor, CodecError> {
    Ok(ChainAnchor {
        spec_version: r.u32()?,
        transaction_version: r.u32()?,
        block_number: r.u64()?,
        parent_hash: r.array32()?,
        extrinsic_index: r.u32()?,
        finality: r.option(|r| {
            Ok(Finality {
                block_number: r.u64()?,
                block_hash: r.array32()?,
                state_root: r.array32()?,
                parent_state_root: r.array32()?,
            })
        })?,
    })
}

/// Canonical bytes of a pre-state.
pub fn encode_pre_state(pre: &TradingPreState) -> Vec<u8> {
    let mut w = Writer::new();
    write_pre_state(&mut w, pre);
    w.finish()
}

/// Read a pre-state from its canonical bytes, refusing trailing bytes.
pub fn decode_pre_state(bytes: &[u8]) -> Result<TradingPreState, CodecError> {
    let mut r = Reader::new(bytes);
    let pre = read_pre_state(&mut r)?;
    r.finish()?;
    Ok(pre)
}

/// The commitment a receipt carries for a trading state: the digest of its canonical bytes.
pub fn state_commitment(state: &TradingPreState) -> [u8; 32] {
    sha256_domain(b"x3-trading-state-v1", &encode_pre_state(state))
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn sample() -> TradingPreState {
        let mut trader_balances = BTreeMap::new();
        trader_balances.insert(1, 50);
        trader_balances.insert(2, 7);
        TradingPreState {
            chain_id: 650_000,
            block_number: 10,
            submitted_at_block: 9,
            trader: [3; 32],
            trader_balances,
            flash_pools: vec![FlashPoolState {
                provider: "x3_flashloan".into(),
                version: 1,
                asset: 1,
                liquidity: 1_000,
                fee_bps: 5,
            }],
            venues: vec![VenueState {
                name: "x3dex_pool_1".into(),
                version: 1,
                asset_a: 1,
                asset_b: 2,
                reserve_a: 10,
                reserve_b: 20,
                fee_bps: 30,
            }],
            oracle: vec![OracleObservation {
                base: 2,
                quote: 1,
                price_num: 2,
                price_den: 1,
                observed_at_block: 9,
                source: "x3_oracle".into(),
            }],
            costs: CostSchedule {
                fee_asset: 1,
                gas_price: 10,
                settlement_fee: 1,
                relay_fee: 2,
                protocol_fee: 3,
                oracle_fee: 4,
                solver_fee: 0,
                cross_domain_fee: 0,
            },
            private_channel: true,
        }
    }

    #[test]
    fn pre_state_round_trips_and_commits_to_every_field() {
        let pre = sample();
        let bytes = encode_pre_state(&pre);
        assert_eq!(decode_pre_state(&bytes), Ok(pre.clone()));
        let mut moved = pre.clone();
        moved.venues[0].reserve_b += 1;
        assert_ne!(state_commitment(&pre), state_commitment(&moved));
        for cut in [0, bytes.len() / 2, bytes.len() - 1] {
            assert!(decode_pre_state(&bytes[..cut]).is_err());
        }
        let mut long = bytes.clone();
        long.push(0);
        assert_eq!(decode_pre_state(&long), Err(CodecError::TrailingBytes));
    }
}
