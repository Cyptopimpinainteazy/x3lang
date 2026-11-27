//! # EIP-1559 (type 2) transactions: canonical encoding and the exact signing hash
//!
//! EIP-1559 (<https://eips.ethereum.org/EIPS/eip-1559>) defines
//!
//! ```text
//! unsigned = 0x02 || rlp([chain_id, nonce, max_priority_fee_per_gas, max_fee_per_gas,
//!                         gas_limit, destination, amount, data, access_list])
//! signing hash = keccak256(unsigned)
//! signed   = 0x02 || rlp([... the nine fields ..., signature_y_parity, signature_r, signature_s])
//! ```
//!
//! RLP is the yellow paper's appendix B encoding: integers are big-endian with no leading zero
//! bytes (zero is the empty string), so every transaction has exactly one encoding.
//! The encoder is checked against the Besu-sourced EIP-1559 vectors that ethereumjs ships
//! (`packages/tx/test/testData/eip1559.ts`) and its access-list vector (`eip-1559.spec.ts`).

use crate::derivation::keccak256;

pub const EIP1559_TX_TYPE: u8 = 0x02;
/// `keccak256("transfer(address,uint256)")[..4]`, checked by a test.
pub const ERC20_TRANSFER_SELECTOR: [u8; 4] = [0xa9, 0x05, 0x9c, 0xbb];
/// Intrinsic gas of any transaction (yellow paper `G_transaction`).
pub const MIN_GAS_LIMIT: u64 = 21_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Eip1559Tx {
    pub chain_id: u64,
    pub nonce: u64,
    pub max_priority_fee_per_gas: u128,
    pub max_fee_per_gas: u128,
    pub gas_limit: u64,
    pub to: [u8; 20],
    pub value: u128,
    pub data: Vec<u8>,
    pub access_list: Vec<([u8; 20], Vec<[u8; 32]>)>,
}

/// An ECDSA signature in Ethereum's form: `y_parity` is 0 or 1, `r` and `s` are big-endian.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvmSignature {
    pub y_parity: u8,
    pub r: [u8; 32],
    pub s: [u8; 32],
}

fn length_prefix(out: &mut Vec<u8>, len: usize, short: u8, long: u8) {
    if len <= 55 {
        out.push(short + len as u8);
    } else {
        let be = (len as u64).to_be_bytes();
        let skip = be.iter().take_while(|b| **b == 0).count();
        out.push(long + (be.len() - skip) as u8);
        out.extend_from_slice(&be[skip..]);
    }
}

pub fn rlp_bytes(out: &mut Vec<u8>, bytes: &[u8]) {
    if bytes.len() == 1 && bytes[0] < 0x80 {
        out.push(bytes[0]);
    } else {
        length_prefix(out, bytes.len(), 0x80, 0xb7);
        out.extend_from_slice(bytes);
    }
}

/// A big-endian unsigned integer with leading zero bytes removed.
pub fn rlp_uint_be(out: &mut Vec<u8>, be: &[u8]) {
    let skip = be.iter().take_while(|b| **b == 0).count();
    rlp_bytes(out, &be[skip..]);
}

pub fn rlp_u64(out: &mut Vec<u8>, value: u64) {
    rlp_uint_be(out, &value.to_be_bytes());
}

pub fn rlp_u128(out: &mut Vec<u8>, value: u128) {
    rlp_uint_be(out, &value.to_be_bytes());
}

pub fn rlp_list(out: &mut Vec<u8>, payload: &[u8]) {
    length_prefix(out, payload.len(), 0xc0, 0xf7);
    out.extend_from_slice(payload);
}

impl Eip1559Tx {
    fn fields(&self) -> Vec<u8> {
        let mut p = Vec::new();
        rlp_u64(&mut p, self.chain_id);
        rlp_u64(&mut p, self.nonce);
        rlp_u128(&mut p, self.max_priority_fee_per_gas);
        rlp_u128(&mut p, self.max_fee_per_gas);
        rlp_u64(&mut p, self.gas_limit);
        rlp_bytes(&mut p, &self.to);
        rlp_u128(&mut p, self.value);
        rlp_bytes(&mut p, &self.data);
        let mut entries = Vec::new();
        for (address, keys) in &self.access_list {
            let mut entry = Vec::new();
            rlp_bytes(&mut entry, address);
            let mut slots = Vec::new();
            for key in keys {
                rlp_bytes(&mut slots, key);
            }
            rlp_list(&mut entry, &slots);
            rlp_list(&mut entries, &entry);
        }
        rlp_list(&mut p, &entries);
        p
    }

    /// The canonical unsigned transaction: the exact signing preimage.
    pub fn unsigned_bytes(&self) -> Vec<u8> {
        let mut out = vec![EIP1559_TX_TYPE];
        rlp_list(&mut out, &self.fields());
        out
    }

    /// `keccak256(unsigned_bytes)`: the only 32 bytes the MPC network is asked to sign.
    pub fn signing_hash(&self) -> [u8; 32] {
        keccak256(&self.unsigned_bytes())
    }

    /// The broadcastable signed transaction.
    pub fn signed_bytes(&self, sig: &EvmSignature) -> Vec<u8> {
        let mut p = self.fields();
        rlp_u64(&mut p, sig.y_parity as u64);
        rlp_uint_be(&mut p, &sig.r);
        rlp_uint_be(&mut p, &sig.s);
        let mut out = vec![EIP1559_TX_TYPE];
        rlp_list(&mut out, &p);
        out
    }
}

/// ABI calldata for `transfer(recipient, amount)` on an ERC-20 token.
pub fn erc20_transfer_data(recipient: &[u8; 20], amount: u128) -> Vec<u8> {
    let mut data = Vec::with_capacity(68);
    data.extend_from_slice(&ERC20_TRANSFER_SELECTOR);
    data.extend_from_slice(&[0u8; 12]);
    data.extend_from_slice(recipient);
    data.extend_from_slice(&[0u8; 16]);
    data.extend_from_slice(&amount.to_be_bytes());
    data
}
