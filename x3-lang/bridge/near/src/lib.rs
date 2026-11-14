//! Experimental native NEAR HTLC. Claims assign enforceable internal credits synchronously.
//! Native wallet withdrawals are asynchronous and NEVER count as completed at preimage reveal.
//!
//! Storage economics (issue #71): every record lives in its own trie key
//! (`store::LookupMap`), so the root state has a fixed size. Whoever creates a
//! record pays its storage stake up front from the attached deposit, separately
//! from escrow principal, and any excess is returned. Terminal swap and window
//! records are kept as prepaid anti-replay tombstones and historic views.
#![allow(clippy::too_many_arguments)]

use near_sdk::json_types::{Base64VecU8, U128};
use near_sdk::store::LookupMap;
use near_sdk::{env, near, AccountId, BorshStorageKey, Gas, NearToken, PanicOnDefault, Promise};

// JSON must encode NEAR yocto amounts as decimal strings. Bare u128
// serializes unreliably for values above the serde_json numeric range.
mod u128_decimal {
    use near_sdk::serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(value: &u128, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&value.to_string())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u128, D::Error> {
        let raw = String::deserialize(deserializer)?;
        raw.parse::<u128>().map_err(near_sdk::serde::de::Error::custom)
    }
}

const MIN_LOCK_NS: u64 = 300_000_000_000;
const MAX_LOCK_NS: u64 = 7 * 24 * 60 * 60 * 1_000_000_000;
const MIN_NEAR_DEPOSIT: u128 = 1_000_000_000_000_000_000_000; // 0.001 NEAR principal.
const CALLBACK_GAS: Gas = Gas::from_tgas(15);
/// Bounds on concurrent live (non-terminal) records. Counted with O(1)
/// counters, never by iterating storage.
const MAX_LIVE_WINDOWS: u32 = 256;
const MAX_LIVE_SWAPS: u32 = 256;
/// Bytes NEAR charges per trie record on top of key and value
/// (`storage_num_extra_bytes_record`). If the protocol ever charges more, the
/// post-write check in `charge_and_verify` fails closed.
const RECORD_OVERHEAD_BYTES: u64 = 40;
/// Swap IDs are exactly 32 bytes in hexadecimal.
const SWAP_ID_LEN: u64 = 64;
/// Every `LookupMap` key is a one-byte prefix followed by the borsh key.
const PREFIX_LEN: u64 = 1;

#[near(serializers = [borsh])]
#[derive(BorshStorageKey)]
enum StorageKey {
    Windows,
    Swaps,
    Credits,
    Inflight,
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SwapState {
    Locked,
    Claimed,
    Refunded,
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FundingWindowState {
    Pending,
    Funded,
    NotFunded,
}

#[near(serializers = [borsh, json])]
#[derive(Clone)]
pub struct FundingWindow {
    pub initiator: AccountId,
    pub recipient: AccountId,
    pub hashlock: Vec<u8>,
    #[serde(with = "u128_decimal")]
    pub amount: u128,
    pub deadline_ns: u64,
    pub expiry_ns: u64,
    pub state: FundingWindowState,
    /// Storage stake the registrar paid for this record (yoctoNEAR).
    #[serde(with = "u128_decimal")]
    pub storage_deposit: u128,
}

#[near(serializers = [borsh, json])]
#[derive(Clone)]
pub struct Swap {
    pub initiator: AccountId,
    pub recipient: AccountId,
    pub hashlock: Vec<u8>,
    #[serde(with = "u128_decimal")]
    pub amount: u128,
    pub expires_at_ns: u64,
    pub state: SwapState,
    /// Storage stake the funder paid for this record and its payout entries
    /// (yoctoNEAR). Never part of `amount` and never counted as liability.
    #[serde(with = "u128_decimal")]
    pub storage_deposit: u128,
}

#[near(contract_state)]
#[derive(PanicOnDefault)]
pub struct NativeNearHtlc {
    registrar: AccountId,
    windows: LookupMap<String, FundingWindow>,
    swaps: LookupMap<String, Swap>,
    credits: LookupMap<AccountId, u128>,
    withdrawals_inflight: LookupMap<AccountId, u128>,
    total_liability: u128,
    live_windows: u32,
    live_swaps: u32,
    /// Total storage stake collected from record creators (yoctoNEAR).
    storage_deposits: u128,
}

fn byte_cost() -> u128 {
    env::storage_byte_cost().as_yoctonear()
}

fn borsh_len<T: near_sdk::borsh::BorshSerialize>(value: &T) -> u64 {
    near_sdk::borsh::to_vec(value).expect("borsh").len() as u64
}

/// One credit or in-flight entry for `account`: key prefix + borsh(AccountId) + u128.
fn payout_entry_bytes(account: &AccountId) -> u64 {
    RECORD_OVERHEAD_BYTES + PREFIX_LEN + 4 + account.as_str().len() as u64 + 16
}

/// Storage for one swap record, plus a reserve for the payee's credit entry and
/// a concurrent in-flight withdrawal entry. The payee is the recipient on claim
/// or the initiator on refund, so reserve for the longer of the two names.
fn swap_storage_bytes(swap: &Swap) -> (u64, u64) {
    let record = RECORD_OVERHEAD_BYTES + PREFIX_LEN + 4 + SWAP_ID_LEN + borsh_len(swap);
    let payee = if swap.recipient.as_str().len() >= swap.initiator.as_str().len() {
        &swap.recipient
    } else {
        &swap.initiator
    };
    (record, 2 * payout_entry_bytes(payee))
}

fn window_storage_bytes(window: &FundingWindow) -> u64 {
    RECORD_OVERHEAD_BYTES + PREFIX_LEN + 4 + SWAP_ID_LEN + borsh_len(window)
}

/// Fails before any state changes unless the attached deposit covers
/// `principal + storage`; returns the excess to refund to the caller.
fn require_deposit(principal: u128, storage_bytes: u64) -> (u128, u128) {
    let storage = byte_cost()
        .checked_mul(storage_bytes as u128)
        .expect("storage cost overflow");
    let required = principal.checked_add(storage).expect("deposit overflow");
    let attached = env::attached_deposit().as_yoctonear();
    assert!(
        attached >= required,
        "deposit must cover principal plus storage: {required} yoctoNEAR required"
    );
    (storage, attached - required)
}

fn refund_excess(excess: u128) {
    if excess > 0 {
        Promise::new(env::predecessor_account_id())
            .transfer(NearToken::from_yoctonear(excess))
            .detach();
    }
}

/// Issue #42: NEAR-finality-bound BNB settlement. A funding window's id is not
/// chosen freely: it MUST be the SHA-256 commitment to every swap term plus the
/// opaque BSC source binding, so any record stored under a window id has
/// exactly the terms the BSC escrow derived that id from. Byte layout is pinned
/// by the cross-language vector shared with `settlement.py` and
/// `NearSettledBnbEscrow.sol`.
const WINDOW_DOMAIN: &[u8] = b"X3-NEAR-BSC-WINDOW-V1";
const SETTLEMENT_DOMAIN: &[u8] = b"X3-NEAR-SETTLEMENT-V1";
/// Exact log line prefix a BSC-side verifier must match, byte for byte.
pub const SETTLEMENT_LOG_PREFIX: &str = "X3_NEAR_SETTLEMENT_V1 ";

/// Settlement outcome codes. `Registered` is evidence a window exists with
/// exact terms; the other three are mutually exclusive terminal outcomes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettlementOutcome {
    Registered = 1,
    Claimed = 2,
    Refunded = 3,
    NotFunded = 4,
}

fn push_account(buf: &mut Vec<u8>, account: &AccountId) {
    let raw = account.as_str().as_bytes();
    buf.extend_from_slice(&(raw.len() as u32).to_be_bytes());
    buf.extend_from_slice(raw);
}

fn lower_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(DIGITS[(b >> 4) as usize] as char);
        out.push(DIGITS[(b & 0x0f) as usize] as char);
    }
    out
}

/// Lowercase-hex window id committing to vault, parties, hashlock, amount,
/// funding deadline, expiry and the 32-byte BSC source binding.
pub fn derive_window_id(
    vault: &AccountId,
    initiator: &AccountId,
    recipient: &AccountId,
    hashlock: &[u8],
    amount: u128,
    deadline_ns: u64,
    expiry_ns: u64,
    source_binding: &[u8],
) -> String {
    assert_eq!(hashlock.len(), 32, "hashlock length");
    assert_eq!(source_binding.len(), 32, "source binding must be 32 bytes");
    let mut buf = Vec::with_capacity(256);
    buf.extend_from_slice(WINDOW_DOMAIN);
    push_account(&mut buf, vault);
    push_account(&mut buf, initiator);
    push_account(&mut buf, recipient);
    buf.extend_from_slice(hashlock);
    buf.extend_from_slice(&amount.to_be_bytes());
    buf.extend_from_slice(&deadline_ns.to_be_bytes());
    buf.extend_from_slice(&expiry_ns.to_be_bytes());
    buf.extend_from_slice(source_binding);
    lower_hex(&env::sha256(&buf))
}

/// The exact log line emitted for `outcome` of window `window_id_hex`.
pub fn settlement_log_line(window_id_hex: &str, outcome: SettlementOutcome) -> String {
    assert_eq!(window_id_hex.len(), 64, "window id length");
    let mut raw = Vec::with_capacity(32);
    for pair in window_id_hex.as_bytes().chunks(2) {
        let hi = (pair[0] as char).to_digit(16).expect("window id hex");
        let lo = (pair[1] as char).to_digit(16).expect("window id hex");
        raw.push((hi * 16 + lo) as u8);
    }
    let mut buf = Vec::with_capacity(SETTLEMENT_DOMAIN.len() + 33);
    buf.extend_from_slice(SETTLEMENT_DOMAIN);
    buf.extend_from_slice(&raw);
    buf.push(outcome as u8);
    format!("{SETTLEMENT_LOG_PREFIX}{}", lower_hex(&env::sha256(&buf)))
}

/// Emitted as the LAST statement of a successful state transition, so no
/// assertion can fail after it inside the same function call. A BSC verifier
/// must still require the receipt's execution status to be successful: NEAR
/// keeps logs of failed receipts in their outcomes.
fn emit_settlement(window_id_hex: &str, outcome: SettlementOutcome) {
    env::log_str(&settlement_log_line(window_id_hex, outcome));
}

fn check_swap_id(id: &str) {
    assert!(
        id.len() as u64 == SWAP_ID_LEN && id.bytes().all(|b| b.is_ascii_hexdigit()),
        "swap id must be 32 bytes in hexadecimal"
    );
}

impl NativeNearHtlc {
    /// Writes are buffered by `store::LookupMap`; flush and confirm the trie
    /// grew by no more than the bytes that were paid for.
    fn verify_storage(&mut self, before: u64, paid_bytes: u64) {
        self.windows.flush();
        self.swaps.flush();
        let used = env::storage_usage().saturating_sub(before);
        assert!(
            used <= paid_bytes,
            "storage under-charged: used {used} paid {paid_bytes}"
        );
    }
}

#[near]
impl NativeNearHtlc {
    #[init]
    pub fn new() -> Self {
        // Testnet pilot only. Name suffix is a necessary deployment precaution,
        // NOT cryptographic network validation: the deployer must independently
        // verify NEAR testnet genesis over the intended RPC before signing.
        assert!(
            env::current_account_id().as_str().ends_with(".testnet"),
            "testnet deployment only"
        );
        Self {
            registrar: env::predecessor_account_id(),
            windows: LookupMap::new(StorageKey::Windows),
            swaps: LookupMap::new(StorageKey::Swaps),
            credits: LookupMap::new(StorageKey::Credits),
            withdrawals_inflight: LookupMap::new(StorageKey::Inflight),
            total_liability: 0,
            live_windows: 0,
            live_swaps: 0,
            storage_deposits: 0,
        }
    }

    /// Testnet pilot: trusted registrar reserves a unique swap identity and a
    /// last funding deadline on-chain BEFORE the liquidity provider funds it.
    /// The registrar attaches the window record's storage stake.
    ///
    /// Issue #42: `id` must equal `derive_window_id` over the exact terms and
    /// the BSC `source_binding`. The registrar therefore admits windows but
    /// cannot bind an id to terms other than the ones it commits to. A
    /// `Registered` settlement log is emitted for the BSC escrow's lock gate.
    #[payable]
    pub fn register_window(
        &mut self,
        id: String,
        initiator: AccountId,
        recipient: AccountId,
        hashlock: Base64VecU8,
        amount: U128,
        deadline_ns: u64,
        expiry_ns: u64,
        source_binding: Base64VecU8,
    ) {
        assert_eq!(env::predecessor_account_id(), self.registrar, "registrar only");
        check_swap_id(&id);
        assert_eq!(hashlock.0.len(), 32, "hashlock length");
        assert_eq!(source_binding.0.len(), 32, "source binding must be 32 bytes");
        assert!(source_binding.0.iter().any(|b| *b != 0), "zero source binding");
        assert_eq!(
            id,
            derive_window_id(
                &env::current_account_id(),
                &initiator,
                &recipient,
                &hashlock.0,
                amount.0,
                deadline_ns,
                expiry_ns,
                &source_binding.0,
            ),
            "window id must commit to its terms"
        );
        assert!(
            !self.swaps.contains_key(&id) && !self.windows.contains_key(&id),
            "id reserved"
        );
        assert_eq!(hashlock.0.len(), 32, "hashlock length");
        assert!(hashlock.0.iter().any(|b| *b != 0), "zero hashlock");
        assert!(amount.0 >= MIN_NEAR_DEPOSIT, "amount too low");
        assert_ne!(recipient, env::current_account_id(), "bad recipient");
        let now = env::block_timestamp();
        assert!(self.live_windows < MAX_LIVE_WINDOWS, "live window limit reached");
        assert!(deadline_ns >= now.saturating_add(MIN_LOCK_NS), "deadline too close");
        assert!(expiry_ns > deadline_ns.saturating_add(MIN_LOCK_NS), "unsafe expiry");
        assert!(expiry_ns <= now.saturating_add(MAX_LOCK_NS), "expiry too far");
        let mut window = FundingWindow {
            initiator,
            recipient,
            hashlock: hashlock.0,
            amount: amount.0,
            deadline_ns,
            expiry_ns,
            state: FundingWindowState::Pending,
            storage_deposit: 0,
        };
        let bytes = window_storage_bytes(&window);
        let (storage, excess) = require_deposit(0, bytes);
        window.storage_deposit = storage;
        let before = env::storage_usage();
        self.windows.insert(id.clone(), window);
        self.verify_storage(before, bytes);
        self.live_windows += 1;
        self.storage_deposits = self.storage_deposits.checked_add(storage).expect("overflow");
        refund_excess(excess);
        emit_settlement(&id, SettlementOutcome::Registered);
    }

    /// The provider can only fund the exact reserved terms before deadline. It
    /// attaches the exact principal plus the swap record's storage stake.
    #[payable]
    pub fn fund_window(&mut self, id: String) {
        assert!(self.live_swaps < MAX_LIVE_SWAPS, "live swap limit reached");
        let window = self.windows.get(&id).expect("unknown funding window");
        assert_eq!(window.state, FundingWindowState::Pending, "not pending");
        assert!(env::block_timestamp() < window.deadline_ns, "funding deadline passed");
        assert_eq!(
            env::predecessor_account_id(),
            window.initiator,
            "wrong funding provider"
        );
        let mut swap = Swap {
            initiator: window.initiator.clone(),
            recipient: window.recipient.clone(),
            hashlock: window.hashlock.clone(),
            amount: window.amount,
            expires_at_ns: window.expiry_ns,
            state: SwapState::Locked,
            storage_deposit: 0,
        };
        let (record, reserve) = swap_storage_bytes(&swap);
        let (storage, excess) = require_deposit(swap.amount, record + reserve);
        swap.storage_deposit = storage;
        let amount = swap.amount;
        let before = env::storage_usage();
        self.swaps.insert(id.clone(), swap);
        self.windows.get_mut(&id).expect("window").state = FundingWindowState::Funded;
        self.verify_storage(before, record);
        self.total_liability = self.total_liability.checked_add(amount).expect("liability overflow");
        self.live_windows -= 1;
        self.live_swaps += 1;
        self.storage_deposits = self.storage_deposits.checked_add(storage).expect("overflow");
        refund_excess(excess);
    }

    /// Never-funding can be finalized once the contract itself forbids late funding.
    /// This is an on-chain terminal state; RPC observations alone do NOT prove it.
    pub fn close_unfunded(&mut self, id: String) {
        let window = self.windows.get_mut(&id).expect("unknown funding window");
        assert_eq!(window.state, FundingWindowState::Pending, "window already terminal");
        assert!(
            env::block_timestamp() >= window.deadline_ns,
            "funding window still open"
        );
        assert!(!self.swaps.contains_key(&id), "already funded");
        window.state = FundingWindowState::NotFunded;
        self.live_windows -= 1;
        emit_settlement(&id, SettlementOutcome::NotFunded);
    }

    pub fn window_state(&self, id: String) -> Option<FundingWindowState> {
        self.windows.get(&id).map(|w| w.state.clone())
    }

    pub fn get_window(&self, id: String) -> Option<FundingWindow> {
        self.windows.get(&id).cloned()
    }

    /// Permissionless lock. `amount` is the escrow principal; the attached
    /// deposit must cover `amount` plus storage (see `lock_deposit_required`).
    #[payable]
    pub fn lock(&mut self, id: String, recipient: AccountId, hashlock: Base64VecU8, expires_at_ns: u64, amount: U128) {
        check_swap_id(&id);
        assert!(
            !self.swaps.contains_key(&id) && !self.windows.contains_key(&id),
            "swap id already used"
        );
        assert_eq!(hashlock.0.len(), 32, "hashlock must be 32 bytes");
        assert!(hashlock.0.iter().any(|b| *b != 0), "zero hashlock prohibited");
        let current = env::block_timestamp();
        assert!(expires_at_ns > current.saturating_add(MIN_LOCK_NS), "expiry too short");
        assert!(expires_at_ns <= current.saturating_add(MAX_LOCK_NS), "expiry too far");
        assert!(self.live_swaps < MAX_LIVE_SWAPS, "live swap limit reached");
        assert!(amount.0 >= MIN_NEAR_DEPOSIT, "minimum 0.001 NEAR");
        assert_ne!(recipient, env::current_account_id(), "cannot credit vault itself");
        let mut swap = Swap {
            initiator: env::predecessor_account_id(),
            recipient,
            hashlock: hashlock.0,
            amount: amount.0,
            expires_at_ns,
            state: SwapState::Locked,
            storage_deposit: 0,
        };
        let (record, reserve) = swap_storage_bytes(&swap);
        // Fails here, before any record or liability exists, if underfunded.
        let (storage, excess) = require_deposit(amount.0, record + reserve);
        swap.storage_deposit = storage;
        let before = env::storage_usage();
        self.swaps.insert(id, swap);
        self.verify_storage(before, record);
        self.total_liability = self.total_liability.checked_add(amount.0).expect("liability overflow");
        self.live_swaps += 1;
        self.storage_deposits = self.storage_deposits.checked_add(storage).expect("overflow");
        refund_excess(excess);
    }

    /// Exact deposit `lock` needs from `initiator` for `amount` to `recipient`.
    pub fn lock_deposit_required(&self, initiator: AccountId, recipient: AccountId, amount: U128) -> U128 {
        let swap = Swap {
            initiator,
            recipient,
            hashlock: vec![0; 32],
            amount: amount.0,
            expires_at_ns: 0,
            state: SwapState::Locked,
            storage_deposit: 0,
        };
        let (record, reserve) = swap_storage_bytes(&swap);
        U128(amount.0 + byte_cost() * (record + reserve) as u128)
    }

    /// Storage stake the registrar must attach to `register_window`.
    pub fn window_deposit_required(&self, initiator: AccountId, recipient: AccountId) -> U128 {
        let window = FundingWindow {
            initiator,
            recipient,
            hashlock: vec![0; 32],
            amount: 0,
            deadline_ns: 0,
            expiry_ns: 0,
            state: FundingWindowState::Pending,
            storage_deposit: 0,
        };
        U128(byte_cost() * window_storage_bytes(&window) as u128)
    }

    /// The source-chain observer must wait for THIS finalized credit state,
    /// not merely for the preimage to appear in a pending NEAR transaction.
    ///
    /// Issue #42: for a window-bound swap the `Claimed` settlement log is the
    /// only NEAR fact that can pay the BSC liquidity provider. A failed claim
    /// still discloses `preimage` in its arguments; that disclosure confers no
    /// BSC payout because the BSC escrow accepts no preimage at all.
    pub fn claim(&mut self, id: String, preimage: Base64VecU8) {
        assert_eq!(preimage.0.len(), 32, "preimage must be 32 raw bytes");
        let bound = self.windows.contains_key(&id);
        let swap = self.swaps.get_mut(&id).expect("unknown swap");
        assert_eq!(swap.state, SwapState::Locked, "not locked");
        assert_eq!(
            swap.recipient,
            env::predecessor_account_id(),
            "not designated recipient"
        );
        assert!(env::block_timestamp() < swap.expires_at_ns, "swap expired");
        assert_eq!(env::sha256(&preimage.0), swap.hashlock, "invalid preimage");
        let recipient = swap.recipient.clone();
        let amount = swap.amount;
        swap.state = SwapState::Claimed;
        self.live_swaps -= 1;
        self.add_credit(recipient, amount);
        if bound {
            emit_settlement(&id, SettlementOutcome::Claimed);
        }
    }

    /// Expiration assigns the initiator an on-chain withdrawal credit.
    ///
    /// Issue #42: callable by anyone once expired, and the credit still goes
    /// only to the initiator. The BSC depositor must be able to drive an
    /// unclaimed, funded window to its `Refunded` terminal state without the
    /// liquidity provider's cooperation, or BNB could stay locked forever.
    pub fn refund(&mut self, id: String) {
        let bound = self.windows.contains_key(&id);
        let swap = self.swaps.get_mut(&id).expect("unknown swap");
        assert_eq!(swap.state, SwapState::Locked, "not locked");
        assert!(env::block_timestamp() >= swap.expires_at_ns, "not expired");
        let initiator = swap.initiator.clone();
        let amount = swap.amount;
        swap.state = SwapState::Refunded;
        self.live_swaps -= 1;
        self.add_credit(initiator, amount);
        if bound {
            emit_settlement(&id, SettlementOutcome::Refunded);
        }
    }

    /// Only owner of credit can withdraw. Reject concurrent withdrawals for the same owner.
    pub fn withdraw(&mut self) -> Promise {
        let account = env::predecessor_account_id();
        assert!(
            !self.withdrawals_inflight.contains_key(&account),
            "payout already pending"
        );
        let amount = self.credits.remove(&account).unwrap_or(0);
        assert!(amount > 0, "no credit to withdraw");
        self.withdrawals_inflight.insert(account.clone(), amount);
        Promise::new(account.clone())
            .transfer(NearToken::from_yoctonear(amount))
            .then(
                Self::ext(env::current_account_id())
                    .with_static_gas(CALLBACK_GAS)
                    .on_withdraw(account),
            )
    }

    #[private]
    pub fn on_withdraw(&mut self, account: AccountId) -> bool {
        let amount = self.withdrawals_inflight.remove(&account).expect("no pending payout");
        match env::promise_result_checked(0, 32) {
            Ok(_) => {
                self.total_liability = self.total_liability.checked_sub(amount).expect("underflow");
                true
            }
            Err(_) => {
                self.add_credit(account, amount);
                false
            }
        }
    }

    pub fn get(&self, id: String) -> Option<Swap> {
        self.swaps.get(&id).cloned()
    }
    pub fn credit_of(&self, owner: AccountId) -> U128 {
        U128(*self.credits.get(&owner).unwrap_or(&0))
    }
    pub fn pending_withdrawal_of(&self, owner: AccountId) -> U128 {
        U128(*self.withdrawals_inflight.get(&owner).unwrap_or(&0))
    }
    pub fn liability(&self) -> U128 {
        U128(self.total_liability)
    }
    /// Total storage stake collected from record creators. Kept separate from
    /// `liability`: it pays for state and is never withdrawable as principal.
    pub fn storage_deposits(&self) -> U128 {
        U128(self.storage_deposits)
    }

    fn add_credit(&mut self, account: AccountId, amount: u128) {
        let existing = self.credits.get(&account).copied().unwrap_or(0);
        self.credits
            .insert(account, existing.checked_add(amount).expect("credit overflow"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use near_sdk::test_utils::VMContextBuilder;
    use near_sdk::testing_env;
    fn set(actor: &str, deposit: u128, time: u64) {
        let ctx = VMContextBuilder::new()
            .current_account_id("x3-vault.testnet".parse().unwrap())
            .predecessor_account_id(actor.parse().unwrap())
            .attached_deposit(NearToken::from_yoctonear(deposit))
            .account_balance(NearToken::from_near(100))
            .storage_usage(env::storage_usage())
            .block_timestamp(time)
            .build();
        testing_env!(ctx);
    }
    const START: u64 = 1_700_000_000_000_000_000;
    fn secret() -> Vec<u8> {
        vec![3u8; 32]
    }
    fn swap_id() -> String {
        "a".repeat(64)
    }
    fn swap_id_at(index: usize) -> String {
        format!("{index:064x}")
    }
    const ONE: u128 = 1_000_000_000_000_000_000_000_000;
    fn lock_deposit(c: &NativeNearHtlc, initiator: &str, recipient: &str, amount: u128) -> u128 {
        c.lock_deposit_required(initiator.parse().unwrap(), recipient.parse().unwrap(), U128(amount))
            .0
    }
    fn lock_as(c: &mut NativeNearHtlc, id: String, initiator: &str, recipient: &str, amount: u128, time: u64) {
        let deposit = lock_deposit(c, initiator, recipient, amount);
        set(initiator, deposit, time);
        c.lock(
            id,
            recipient.parse().unwrap(),
            Base64VecU8(env::sha256(secret())),
            time + 600_000_000_000,
            U128(amount),
        );
    }
    fn lock(c: &mut NativeNearHtlc) {
        lock_as(c, swap_id(), "lp.testnet", "alice.testnet", ONE, START);
    }
    fn try_lock(c: &mut NativeNearHtlc, id: String, deposit: u128, expiry: u64) -> bool {
        set("lp.testnet", deposit, START);
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            c.lock(
                id,
                "alice.testnet".parse().unwrap(),
                Base64VecU8(env::sha256(secret())),
                expiry,
                U128(ONE),
            )
        }))
        .is_ok()
    }
    fn window_deposit(c: &NativeNearHtlc) -> u128 {
        c.window_deposit_required("lp.testnet".parse().unwrap(), "alice.testnet".parse().unwrap())
            .0
    }
    /// Distinct non-zero BSC source binding per index (stands in for distinct
    /// BSC swaps; the NEAR contract treats it as opaque bytes).
    fn binding(index: usize) -> Vec<u8> {
        let mut b = vec![0xEEu8; 32];
        b[..8].copy_from_slice(&(index as u64).to_be_bytes());
        b
    }
    fn bound_id(index: usize, deadline: u64, expiry: u64) -> String {
        derive_window_id(
            &"x3-vault.testnet".parse().unwrap(),
            &"lp.testnet".parse().unwrap(),
            &"alice.testnet".parse().unwrap(),
            &env::sha256(secret()),
            ONE,
            deadline,
            expiry,
            &binding(index),
        )
    }
    /// Registers the window for source binding `index` and returns its id.
    fn register(c: &mut NativeNearHtlc, index: usize, deadline: u64, expiry: u64) -> String {
        let id = bound_id(index, deadline, expiry);
        let deposit = window_deposit(c);
        set("registrar.testnet", deposit, START);
        c.register_window(
            id.clone(),
            "lp.testnet".parse().unwrap(),
            "alice.testnet".parse().unwrap(),
            Base64VecU8(env::sha256(secret())),
            U128(ONE),
            deadline,
            expiry,
            Base64VecU8(binding(index)),
        );
        id
    }
    fn fund_deposit(c: &NativeNearHtlc) -> u128 {
        lock_deposit(c, "lp.testnet", "alice.testnet", ONE)
    }
    fn new_testnet_vault() -> NativeNearHtlc {
        set("registrar.testnet", 0, START);
        NativeNearHtlc::new()
    }

    #[test]
    fn native_near_htlc_allows_testnet_named_contract_with_registrar() {
        set("owner.testnet", 0, START);
        let vault = NativeNearHtlc::new();
        assert_eq!(vault.registrar.as_str(), "owner.testnet");
    }

    #[test]
    #[should_panic(expected = "testnet deployment only")]
    fn native_near_htlc_refuses_mainnet_named_contract_at_init() {
        let mut ctx = VMContextBuilder::new();
        ctx.current_account_id("native-x3-vault.near".parse().unwrap())
            .predecessor_account_id("owner.near".parse().unwrap())
            .block_timestamp(START);
        testing_env!(ctx.build());
        NativeNearHtlc::new();
    }

    #[test]
    #[should_panic(expected = "testnet deployment only")]
    fn native_near_htlc_refuses_implicit_account_at_init() {
        let mut ctx = VMContextBuilder::new();
        ctx.current_account_id("a".repeat(64).parse().unwrap())
            .predecessor_account_id("owner.near".parse().unwrap())
            .block_timestamp(START);
        testing_env!(ctx.build());
        NativeNearHtlc::new();
    }

    #[test]
    fn funding_window_json_view_has_expected_wire_fields() {
        let mut c = new_testnet_vault();
        let id = register(&mut c, 0, START + 600_000_000_000, START + 1_200_000_000_000);
        let json = near_sdk::serde_json::to_value(c.get_window(id.clone()).unwrap()).unwrap();
        assert_eq!(json["state"], "Pending");
        assert!(json["deadline_ns"].is_number());
        assert!(json["hashlock"].is_array());
        assert_eq!(json["amount"], ONE.to_string());
        assert_eq!(json["storage_deposit"], window_deposit(&c).to_string());
    }

    #[test]
    fn can_never_close_funding_window_early_or_fund_at_deadline() {
        let mut c = new_testnet_vault();
        let deadline = START + 600_000_000_000;
        let id = register(&mut c, 0, deadline, START + 1_200_000_000_000);
        set("anyone.testnet", 0, deadline - 1);
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| c.close_unfunded(id.clone()))).is_err());
        set("lp.testnet", fund_deposit(&c), deadline);
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| c.fund_window(id.clone()))).is_err());
        set("anyone.testnet", 0, deadline);
        c.close_unfunded(id.clone());
        assert_eq!(c.window_state(id.clone()), Some(FundingWindowState::NotFunded));
        assert_eq!(c.liability().0, 0);
        assert_eq!(c.live_windows, 0);
    }

    #[test]
    fn existing_escrow_json_amount_is_exact_decimal_string() {
        let mut c = new_testnet_vault();
        lock(&mut c);
        let json = near_sdk::serde_json::to_value(c.get(swap_id()).unwrap()).unwrap();
        assert_eq!(json["amount"], ONE.to_string());
        assert_eq!(json["storage_deposit"], (fund_deposit(&c) - ONE).to_string());
    }

    #[test]
    fn registered_window_can_be_irreversibly_closed_when_never_funded() {
        let mut c = new_testnet_vault();
        let id = register(&mut c, 0, START + 600_000_000_000, START + 1_200_000_000_000);
        assert_eq!(c.window_state(id.clone()), Some(FundingWindowState::Pending));
        set("anyone.testnet", 0, START + 600_000_000_000);
        c.close_unfunded(id.clone());
        assert_eq!(c.window_state(id.clone()), Some(FundingWindowState::NotFunded));
        assert!(c.get(id.clone()).is_none());
        set("lp.testnet", fund_deposit(&c), START + 600_000_000_001);
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| c.fund_window(id.clone()))).is_err());
        let deposit = fund_deposit(&c);
        assert!(!try_lock(&mut c, id.clone(), deposit, START + 1_200_000_000_000));
        assert_eq!(c.liability().0, 0);
    }

    #[test]
    fn funded_window_cannot_be_closed_as_unfunded() {
        let mut c = new_testnet_vault();
        let id = register(&mut c, 0, START + 600_000_000_000, START + 1_200_000_000_000);
        set("lp.testnet", fund_deposit(&c), START + 1);
        c.fund_window(id.clone());
        assert_eq!(c.window_state(id.clone()), Some(FundingWindowState::Funded));
        assert_eq!(c.get(id.clone()).unwrap().amount, ONE);
        assert_eq!(c.liability().0, ONE);
        set("anyone.testnet", 0, START + 600_000_000_000);
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| c.close_unfunded(id.clone()))).is_err());
    }

    #[test]
    fn unauthorized_registrar_and_wrong_lp_or_amount_rejected() {
        let mut c = new_testnet_vault();
        let deposit = window_deposit(&c);
        set("stranger.testnet", deposit, START);
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| c.register_window(
                bound_id(0, START + 600_000_000_000, START + 1_200_000_000_000),
                "lp.testnet".parse().unwrap(),
                "alice.testnet".parse().unwrap(),
                Base64VecU8(env::sha256(secret())),
                U128(ONE),
                START + 600_000_000_000,
                START + 1_200_000_000_000,
                Base64VecU8(binding(0))
            )))
            .is_err()
        );
        let id = register(&mut c, 0, START + 600_000_000_000, START + 1_200_000_000_000);
        set("attacker.testnet", fund_deposit(&c), START + 1);
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| c.fund_window(id.clone()))).is_err());
        set("lp.testnet", fund_deposit(&c) - 1, START + 2);
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| c.fund_window(id.clone()))).is_err());
        assert_eq!(c.window_state(id.clone()), Some(FundingWindowState::Pending));
        assert_eq!(c.liability().0, 0);
    }

    #[test]
    fn locks_native_near_and_records_payout_owner() {
        let mut c = new_testnet_vault();
        lock(&mut c);
        let swap = c.get(swap_id()).unwrap();
        assert_eq!(swap.amount, ONE);
        assert_eq!(swap.recipient.as_str(), "alice.testnet");
        assert_eq!(swap.state, SwapState::Locked);
        assert_eq!(c.liability().0, ONE);
    }
    #[test]
    fn claim_credits_on_chain_before_secret_becomes_useful() {
        let mut c = new_testnet_vault();
        lock(&mut c);
        set("alice.testnet", 0, START + 1);
        c.claim(swap_id(), Base64VecU8(secret()));
        assert_eq!(c.get(swap_id()).unwrap().state, SwapState::Claimed);
        assert_eq!(c.credit_of("alice.testnet".parse().unwrap()).0, ONE);
        assert_eq!(c.liability().0, ONE);
    }
    #[test]
    fn only_recipient_with_correct_secret_can_claim() {
        let mut c = new_testnet_vault();
        lock(&mut c);
        set("attacker.testnet", 0, START + 1);
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(
            || c.claim(swap_id(), Base64VecU8(secret()))
        ))
        .is_err());
        set("alice.testnet", 0, START + 1);
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(
            || c.claim(swap_id(), Base64VecU8(vec![4u8; 32]))
        ))
        .is_err());
        assert_eq!(c.get(swap_id()).unwrap().state, SwapState::Locked);
        assert_eq!(c.credit_of("alice.testnet".parse().unwrap()).0, 0);
    }
    #[test]
    fn duplicate_swap_id_is_rejected_after_claim() {
        let mut c = new_testnet_vault();
        lock(&mut c);
        set("alice.testnet", 0, START + 1);
        c.claim(swap_id(), Base64VecU8(secret()));
        let deposit = fund_deposit(&c);
        assert!(!try_lock(&mut c, swap_id(), deposit, START + 600_000_000_000));
    }
    #[test]
    /// Issue #42 changed refund from initiator-only to permissionless after
    /// expiry. Who triggers it never changes who is credited.
    fn refund_after_expiry_is_permissionless_but_credits_only_initiator() {
        let mut c = new_testnet_vault();
        lock(&mut c);
        set("lp.testnet", 0, START + 1);
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| c.refund(swap_id()))).is_err());
        set("other.testnet", 0, START + 600_000_000_000 - 1);
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| c.refund(swap_id()))).is_err());
        set("other.testnet", 0, START + 600_000_000_000);
        c.refund(swap_id());
        assert_eq!(c.get(swap_id()).unwrap().state, SwapState::Refunded);
        assert_eq!(c.credit_of("lp.testnet".parse().unwrap()).0, ONE);
        assert_eq!(c.credit_of("other.testnet".parse().unwrap()).0, 0);
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| c.refund(swap_id()))).is_err());
    }
    #[test]
    fn withdrawal_moves_credit_to_inflight_without_double_spend() {
        let mut c = new_testnet_vault();
        lock(&mut c);
        set("alice.testnet", 0, START + 1);
        c.claim(swap_id(), Base64VecU8(secret()));
        let _promise = c.withdraw();
        assert_eq!(c.credit_of("alice.testnet".parse().unwrap()).0, 0);
        assert_eq!(c.pending_withdrawal_of("alice.testnet".parse().unwrap()).0, ONE);
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = c.withdraw();
        }))
        .is_err());
    }
    fn callback_ctx(time: u64) -> near_sdk::VMContext {
        VMContextBuilder::new()
            .current_account_id("x3-vault.testnet".parse().unwrap())
            .predecessor_account_id("x3-vault.testnet".parse().unwrap())
            .account_balance(NearToken::from_near(100))
            .storage_usage(env::storage_usage())
            .block_timestamp(time)
            .build()
    }
    fn callback(time: u64, ok: bool) {
        let result = if ok {
            near_sdk::PromiseResult::Successful(vec![])
        } else {
            near_sdk::PromiseResult::Failed
        };
        testing_env!(
            callback_ctx(time),
            near_sdk::test_vm_config(),
            near_sdk::RuntimeFeesConfig::test(),
            std::collections::HashMap::new(),
            vec![result]
        );
    }

    #[test]
    fn failed_async_wallet_transfer_restores_claimant_credit() {
        let mut c = new_testnet_vault();
        lock(&mut c);
        set("alice.testnet", 0, START + 1);
        c.claim(swap_id(), Base64VecU8(secret()));
        let _ = c.withdraw();
        callback(START + 2, false);
        assert!(!c.on_withdraw("alice.testnet".parse().unwrap()));
        assert_eq!(c.credit_of("alice.testnet".parse().unwrap()).0, ONE);
        assert_eq!(c.pending_withdrawal_of("alice.testnet".parse().unwrap()).0, 0);
        assert_eq!(c.liability().0, ONE);
    }

    #[test]
    fn confirmed_async_wallet_transfer_reduces_liability_once() {
        let mut c = new_testnet_vault();
        lock(&mut c);
        set("alice.testnet", 0, START + 1);
        c.claim(swap_id(), Base64VecU8(secret()));
        let _ = c.withdraw();
        callback(START + 2, true);
        assert!(c.on_withdraw("alice.testnet".parse().unwrap()));
        assert_eq!(c.credit_of("alice.testnet".parse().unwrap()).0, 0);
        assert_eq!(c.pending_withdrawal_of("alice.testnet".parse().unwrap()).0, 0);
        assert_eq!(c.liability().0, 0);
    }
    #[test]
    fn rejects_invalid_lock_details_and_zero_amount() {
        let mut c = new_testnet_vault();
        assert!(!try_lock(&mut c, swap_id(), 0, START + 600_000_000_000));
        let deposit = fund_deposit(&c);
        assert!(!try_lock(&mut c, "not-hex".into(), deposit, START + 600_000_000_000));
        set("lp.testnet", deposit, START);
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| c.lock(
            swap_id(),
            "alice.testnet".parse().unwrap(),
            Base64VecU8(env::sha256(secret())),
            START + 600_000_000_000,
            U128(0)
        )))
        .is_err());
        assert_eq!(c.liability().0, 0);
    }

    #[test]
    fn live_swap_limit_is_fail_closed() {
        let mut c = new_testnet_vault();
        for i in 0..MAX_LIVE_SWAPS as usize {
            lock_as(&mut c, swap_id_at(i), "lp.testnet", "alice.testnet", ONE, START);
        }
        assert_eq!(c.live_swaps, MAX_LIVE_SWAPS);
        let deposit = fund_deposit(&c);
        assert!(!try_lock(
            &mut c,
            swap_id_at(MAX_LIVE_SWAPS as usize),
            deposit,
            START + 600_000_000_000
        ));
    }

    #[test]
    fn terminal_swap_does_not_consume_a_live_slot() {
        let mut c = new_testnet_vault();
        lock(&mut c);
        set("alice.testnet", 0, START + 1);
        c.claim(swap_id(), Base64VecU8(secret()));
        assert_eq!(c.live_swaps, 0);
        lock_as(&mut c, swap_id_at(1), "lp.testnet", "alice.testnet", ONE, START);
        assert_eq!(c.live_swaps, 1);
    }

    #[test]
    fn live_window_limit_is_fail_closed() {
        let mut c = new_testnet_vault();
        let (deadline, expiry) = (START + 600_000_000_000, START + 1_200_000_000_000);
        let mut first = String::new();
        for i in 0..MAX_LIVE_WINDOWS as usize {
            let id = register(&mut c, i, deadline, expiry);
            if i == 0 {
                first = id;
            }
        }
        assert_eq!(c.live_windows, MAX_LIVE_WINDOWS);
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| register(
            &mut c,
            MAX_LIVE_WINDOWS as usize,
            deadline,
            expiry
        )))
        .is_err());
        // Funding one window frees a window slot (counter, not a storage scan).
        set("lp.testnet", fund_deposit(&c), START + 1);
        c.fund_window(first);
        assert_eq!(c.live_windows, MAX_LIVE_WINDOWS - 1);
        register(&mut c, MAX_LIVE_WINDOWS as usize, deadline, expiry);
    }

    // ---- issue #71: storage economics -------------------------------------

    fn byte_cost_now() -> u128 {
        env::storage_byte_cost().as_yoctonear()
    }

    #[test]
    fn root_state_size_is_constant_per_lock() {
        // The old layout serialized every swap into the root value (160 bytes
        // per lock for these names, 0.0016 NEAR against a 0.001 NEAR minimum).
        let mut c = new_testnet_vault();
        let root = near_sdk::borsh::to_vec(&c).unwrap().len();
        for i in 0..8 {
            lock_as(&mut c, swap_id_at(i), "lp.testnet", "alice.testnet", ONE, START);
        }
        assert_eq!(near_sdk::borsh::to_vec(&c).unwrap().len(), root);
    }

    #[test]
    fn charged_record_bytes_match_runtime_storage_exactly() {
        let mut c = new_testnet_vault();
        let before = env::storage_usage();
        lock(&mut c);
        let used = env::storage_usage() - before;
        let swap = c.get(swap_id()).unwrap();
        let (record, reserve) = swap_storage_bytes(&swap);
        assert_eq!(used, record, "formula must equal the runtime's own accounting");
        assert_eq!(swap.storage_deposit, byte_cost_now() * (record + reserve) as u128);
        // Payout entries created later stay inside the prepaid reserve.
        let after_lock = env::storage_usage();
        set("alice.testnet", 0, START + 1);
        c.claim(swap_id(), Base64VecU8(secret()));
        let _ = c.withdraw();
        c.credits.flush();
        c.withdrawals_inflight.flush();
        assert!(env::storage_usage() - after_lock <= reserve);
    }

    #[test]
    fn underfunded_lock_fails_before_any_record_or_liability() {
        let mut c = new_testnet_vault();
        let before = env::storage_usage();
        // The old API accepted principal alone; now that is one storage stake short.
        assert!(!try_lock(&mut c, swap_id(), ONE, START + 600_000_000_000));
        let short = fund_deposit(&c) - 1;
        assert!(!try_lock(&mut c, swap_id(), short, START + 600_000_000_000));
        assert!(c.get(swap_id()).is_none());
        assert_eq!(c.liability().0, 0);
        assert_eq!(c.storage_deposits().0, 0);
        assert_eq!(c.live_swaps, 0);
        assert_eq!(env::storage_usage(), before);
    }

    #[test]
    fn minimum_principal_still_pays_storage_separately() {
        let mut c = new_testnet_vault();
        let required = lock_deposit(&c, "lp.testnet", "alice.testnet", MIN_NEAR_DEPOSIT);
        let storage = required - MIN_NEAR_DEPOSIT;
        assert!(
            storage > MIN_NEAR_DEPOSIT,
            "storage alone exceeds the old 0.001 NEAR minimum"
        );
        lock_as(
            &mut c,
            swap_id(),
            "lp.testnet",
            "alice.testnet",
            MIN_NEAR_DEPOSIT,
            START,
        );
        assert_eq!(c.liability().0, MIN_NEAR_DEPOSIT, "storage never becomes liability");
        assert_eq!(c.storage_deposits().0, storage);
        set("alice.testnet", 0, START + 1);
        c.claim(swap_id(), Base64VecU8(secret()));
        assert_eq!(c.credit_of("alice.testnet".parse().unwrap()).0, MIN_NEAR_DEPOSIT);
    }

    #[test]
    fn excess_deposit_is_refunded_to_the_caller() {
        let mut c = new_testnet_vault();
        let required = fund_deposit(&c);
        set("lp.testnet", required + 12_345, START);
        c.lock(
            swap_id(),
            "alice.testnet".parse().unwrap(),
            Base64VecU8(env::sha256(secret())),
            START + 600_000_000_000,
            U128(ONE),
        );
        let receipts = near_sdk::test_utils::get_created_receipts();
        let refunds: Vec<_> = receipts
            .iter()
            .filter(|r| r.receiver_id.as_str() == "lp.testnet")
            .flat_map(|r| r.actions.iter())
            .filter_map(|a| match a {
                near_sdk::mock::MockAction::Transfer { deposit, .. } => Some(deposit.as_yoctonear()),
                _ => None,
            })
            .collect();
        assert_eq!(refunds, vec![12_345]);
        assert_eq!(c.liability().0, ONE);
        assert_eq!(c.storage_deposits().0, required - ONE);
    }

    #[test]
    fn extreme_account_ids_are_charged_for_their_length() {
        let mut c = new_testnet_vault();
        let long_a = format!("{}.testnet", "a".repeat(56));
        let long_b = format!("{}.testnet", "b".repeat(56));
        assert_eq!(long_a.len(), 64);
        let short = lock_deposit(&c, "lp.testnet", "alice.testnet", ONE);
        let long = lock_deposit(&c, &long_a, &long_b, ONE);
        // Two names 51 bytes longer in the record, one in each of two reserved payout entries.
        let extra = (64 - "lp.testnet".len()) + (64 - "alice.testnet".len()) + 2 * (64 - "alice.testnet".len());
        assert_eq!(long - short, byte_cost_now() * extra as u128);
        let before = env::storage_usage();
        lock_as(&mut c, swap_id_at(7), &long_a, &long_b, ONE, START);
        let (record, _) = swap_storage_bytes(&c.get(swap_id_at(7)).unwrap());
        assert_eq!(env::storage_usage() - before, record);
    }

    #[test]
    fn under_charged_write_fails_closed() {
        let mut c = new_testnet_vault();
        let before = env::storage_usage();
        c.swaps.insert(
            swap_id(),
            Swap {
                initiator: "lp.testnet".parse().unwrap(),
                recipient: "alice.testnet".parse().unwrap(),
                hashlock: vec![1; 32],
                amount: ONE,
                expires_at_ns: START,
                state: SwapState::Locked,
                storage_deposit: 0,
            },
        );
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| c.verify_storage(before, 10))).is_err());
    }

    #[test]
    fn window_registration_and_funding_charge_storage() {
        let mut c = new_testnet_vault();
        set("registrar.testnet", 0, START);
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| c.register_window(
                bound_id(0, START + 600_000_000_000, START + 1_200_000_000_000),
                "lp.testnet".parse().unwrap(),
                "alice.testnet".parse().unwrap(),
                Base64VecU8(env::sha256(secret())),
                U128(ONE),
                START + 600_000_000_000,
                START + 1_200_000_000_000,
                Base64VecU8(binding(0))
            )))
            .is_err()
        );
        assert!(c
            .get_window(bound_id(0, START + 600_000_000_000, START + 1_200_000_000_000))
            .is_none());
        let id = register(&mut c, 0, START + 600_000_000_000, START + 1_200_000_000_000);
        let window_stake = window_deposit(&c);
        assert_eq!(c.storage_deposits().0, window_stake);
        // Old behaviour: exact principal. Now refused; principal plus storage accepted.
        set("lp.testnet", ONE, START + 1);
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| c.fund_window(id.clone()))).is_err());
        assert_eq!(c.liability().0, 0);
        set("lp.testnet", fund_deposit(&c), START + 1);
        c.fund_window(id.clone());
        assert_eq!(c.liability().0, ONE);
        assert_eq!(c.storage_deposits().0, window_stake + fund_deposit(&c) - ONE);
    }

    /// Permissionless lock spam with maximum-length names, concurrent logical
    /// swaps, claims, refunds and withdrawals. After every transition the
    /// vault holds at least liabilities plus the stake for every byte it added.
    #[test]
    fn lock_spam_and_full_lifecycle_keep_vault_solvent() {
        let mut c = new_testnet_vault();
        let baseline = env::storage_usage();
        let mut balance: u128 = 0; // NEAR received minus NEAR paid out
        let solvent = |c: &mut NativeNearHtlc, balance: u128| {
            c.windows.flush();
            c.swaps.flush();
            c.credits.flush();
            c.withdrawals_inflight.flush();
            let stake = byte_cost_now() * (env::storage_usage() - baseline) as u128;
            assert!(stake <= c.storage_deposits().0, "storage growth not prepaid");
            assert!(balance >= c.liability().0 + stake, "vault insolvent");
        };
        let n = MAX_LIVE_SWAPS as usize;
        let who = |k: char, i: usize| format!("{k}{i:0>55}.testnet");
        for i in 0..n {
            let (a, b) = (who('a', i), who('r', i));
            balance += lock_deposit(&c, &a, &b, MIN_NEAR_DEPOSIT);
            lock_as(&mut c, swap_id_at(i), &a, &b, MIN_NEAR_DEPOSIT, START);
            solvent(&mut c, balance);
        }
        let deposit = fund_deposit(&c);
        assert!(!try_lock(&mut c, swap_id_at(n), deposit, START + 600_000_000_000));
        for i in 0..n {
            let payee = if i % 2 == 0 {
                set(&who('r', i), 0, START + 1);
                c.claim(swap_id_at(i), Base64VecU8(secret()));
                who('r', i)
            } else {
                set(&who('a', i), 0, START + 600_000_000_000);
                c.refund(swap_id_at(i));
                who('a', i)
            };
            solvent(&mut c, balance);
            set(&payee, 0, START + 600_000_000_001);
            let _ = c.withdraw();
            solvent(&mut c, balance);
            callback(START + 600_000_000_002, i % 3 != 0);
            let ok = c.on_withdraw(payee.parse().unwrap());
            if ok {
                balance -= MIN_NEAR_DEPOSIT;
            }
            solvent(&mut c, balance);
        }
        assert_eq!(c.live_swaps, 0);
        // Failed payouts remain owed as credit; everything else was paid once.
        let owed = (0..n).filter(|i| i % 3 == 0).count() as u128 * MIN_NEAR_DEPOSIT;
        assert_eq!(c.liability().0, owed);
        // Terminal records remain as anti-replay tombstones.
        assert!(!try_lock(&mut c, swap_id_at(0), deposit, START + 600_000_000_000));
    }

    // ---- issue #42: NEAR-finality-bound BSC settlement --------------------

    const DEADLINE: u64 = START + 600_000_000_000;
    const EXPIRY: u64 = START + 1_200_000_000_000;

    fn settlement_logs() -> Vec<String> {
        near_sdk::test_utils::get_logs()
            .into_iter()
            .filter(|l| l.starts_with(SETTLEMENT_LOG_PREFIX))
            .collect()
    }
    fn fails<R>(f: impl FnOnce() -> R) -> bool {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).is_err()
    }
    fn funded_window(c: &mut NativeNearHtlc) -> String {
        let id = register(c, 0, DEADLINE, EXPIRY);
        set("lp.testnet", fund_deposit(c), START + 1);
        c.fund_window(id.clone());
        assert!(settlement_logs().is_empty(), "funding is not a settlement outcome");
        id
    }

    /// The same bytes are pinned in `test_settlement.py` (Python) and
    /// `NearSettledBnbEscrow.t.sol` (Solidity). Any layout drift breaks all three.
    #[test]
    fn settlement_encoding_matches_cross_language_vector() {
        set("anyone.testnet", 0, START);
        let hashlock = env::sha256(secret());
        assert_eq!(
            lower_hex(&hashlock),
            "648aa5c579fb30f38af744d97d6ec840c7a91277a499a0d780f3e7314eca090b"
        );
        let mut binding = [0u8; 32];
        let raw = "f9ceb35c05cbdd513aa7342a5ba33dd8538a9580668cea14017b5ef7f91d05aa";
        for (i, b) in binding.iter_mut().enumerate() {
            *b = u8::from_str_radix(&raw[2 * i..2 * i + 2], 16).unwrap();
        }
        let id = derive_window_id(
            &"x3-vault.testnet".parse().unwrap(),
            &"lp.testnet".parse().unwrap(),
            &"alice.testnet".parse().unwrap(),
            &hashlock,
            ONE,
            1_700_000_600_000_000_000,
            1_700_001_200_000_000_000,
            &binding,
        );
        assert_eq!(id, "9d66ddfe92e56e993dd87e2f7ca53b87e39ceb35325cc44b404c0a8eb87de110");
        let expected = [
            (
                SettlementOutcome::Registered,
                "4b7b258b6db8ae6b3979c0ef505ecf625c0ab59727439d980ce017b01fa04808",
            ),
            (
                SettlementOutcome::Claimed,
                "c956f53937ecff906f42672b636a1b8216fd7beade6dd7eeaaa2074e52d9177e",
            ),
            (
                SettlementOutcome::Refunded,
                "561cc941f2d1e81c86d2d428753acae16de924e836c2d9b589dbbc4a303bbb17",
            ),
            (
                SettlementOutcome::NotFunded,
                "e440e5dd802c852009e6e7315b05e404404d8f6f861d4a2f114ee9014dde9dff",
            ),
        ];
        for (outcome, commitment) in expected {
            assert_eq!(
                settlement_log_line(&id, outcome),
                format!("X3_NEAR_SETTLEMENT_V1 {commitment}")
            );
        }
    }

    #[test]
    fn window_id_must_commit_to_every_term_and_the_source_binding() {
        let mut c = new_testnet_vault();
        let good = bound_id(0, DEADLINE, EXPIRY);
        let attempt = |c: &mut NativeNearHtlc, id: String, amount: u128, recipient: &str, bind: Vec<u8>| {
            let deposit = window_deposit(c);
            set("registrar.testnet", deposit, START);
            fails(|| {
                c.register_window(
                    id,
                    "lp.testnet".parse().unwrap(),
                    recipient.parse().unwrap(),
                    Base64VecU8(env::sha256(secret())),
                    U128(amount),
                    DEADLINE,
                    EXPIRY,
                    Base64VecU8(bind),
                )
            })
        };
        // Same id, but terms that differ from what the id commits to.
        assert!(attempt(&mut c, good.clone(), ONE + 1, "alice.testnet", binding(0)));
        assert!(attempt(&mut c, good.clone(), ONE, "lp.testnet", binding(0)));
        assert!(attempt(&mut c, good.clone(), ONE, "alice.testnet", binding(1)));
        assert!(attempt(&mut c, good.to_uppercase(), ONE, "alice.testnet", binding(0)));
        assert!(attempt(&mut c, swap_id(), ONE, "alice.testnet", binding(0)));
        assert!(attempt(&mut c, good.clone(), ONE, "alice.testnet", vec![0u8; 32]));
        assert!(attempt(&mut c, good.clone(), ONE, "alice.testnet", vec![1u8; 31]));
        assert!(c.get_window(good.clone()).is_none());
        assert_eq!(c.live_windows, 0);
        assert!(!attempt(&mut c, good.clone(), ONE, "alice.testnet", binding(0)));
        assert_eq!(c.window_state(good), Some(FundingWindowState::Pending));
    }

    #[test]
    fn registration_emits_exactly_the_registered_settlement_log() {
        let mut c = new_testnet_vault();
        let id = register(&mut c, 0, DEADLINE, EXPIRY);
        assert_eq!(
            settlement_logs(),
            vec![settlement_log_line(&id, SettlementOutcome::Registered)]
        );
    }

    /// The issue #42 attack surface on NEAR: a claim that fails for any reason
    /// (wrong caller, wrong preimage, landing at expiry) must never emit the
    /// `Claimed` settlement log, and must leave the swap refundable.
    #[test]
    fn failed_claims_emit_no_settlement_log_and_success_emits_exactly_one() {
        let mut c = new_testnet_vault();
        let id = funded_window(&mut c);
        set("lp.testnet", 0, START + 2);
        assert!(fails(|| c.claim(id.clone(), Base64VecU8(secret()))));
        assert!(settlement_logs().is_empty());
        set("alice.testnet", 0, START + 2);
        assert!(fails(|| c.claim(id.clone(), Base64VecU8(vec![4u8; 32]))));
        assert!(settlement_logs().is_empty());
        set("alice.testnet", 0, EXPIRY);
        assert!(fails(|| c.claim(id.clone(), Base64VecU8(secret()))));
        assert!(settlement_logs().is_empty());
        assert_eq!(c.get(id.clone()).unwrap().state, SwapState::Locked);
        assert_eq!(c.credit_of("alice.testnet".parse().unwrap()).0, 0);
        // The preimage is now public, yet the swap is still only refundable.
        set("anyone.testnet", 0, EXPIRY);
        c.refund(id.clone());
        assert_eq!(
            settlement_logs(),
            vec![settlement_log_line(&id, SettlementOutcome::Refunded)]
        );
        assert_eq!(c.credit_of("lp.testnet".parse().unwrap()).0, ONE);
        set("alice.testnet", 0, EXPIRY + 1);
        assert!(fails(|| c.claim(id.clone(), Base64VecU8(secret()))));
        assert!(settlement_logs().is_empty());
    }

    #[test]
    fn claim_one_nanosecond_before_expiry_is_terminal_and_excludes_refund() {
        let mut c = new_testnet_vault();
        let id = funded_window(&mut c);
        set("alice.testnet", 0, EXPIRY - 1);
        c.claim(id.clone(), Base64VecU8(secret()));
        assert_eq!(
            settlement_logs(),
            vec![settlement_log_line(&id, SettlementOutcome::Claimed)]
        );
        assert_eq!(c.credit_of("alice.testnet".parse().unwrap()).0, ONE);
        set("lp.testnet", 0, EXPIRY);
        assert!(fails(|| c.refund(id.clone())));
        set("alice.testnet", 0, EXPIRY);
        assert!(fails(|| c.claim(id.clone(), Base64VecU8(secret()))));
        assert!(settlement_logs().is_empty());
        assert_eq!(c.credit_of("lp.testnet".parse().unwrap()).0, 0);
    }

    #[test]
    fn refund_one_nanosecond_before_expiry_fails_and_at_expiry_is_terminal() {
        let mut c = new_testnet_vault();
        let id = funded_window(&mut c);
        set("alice.testnet", 0, EXPIRY - 1);
        assert!(fails(|| c.refund(id.clone())));
        assert!(settlement_logs().is_empty());
        set("alice.testnet", 0, EXPIRY);
        c.refund(id.clone());
        assert_eq!(
            settlement_logs(),
            vec![settlement_log_line(&id, SettlementOutcome::Refunded)]
        );
    }

    #[test]
    fn omitted_funding_closes_with_not_funded_log_and_nothing_follows() {
        let mut c = new_testnet_vault();
        let id = register(&mut c, 0, DEADLINE, EXPIRY);
        set("alice.testnet", 0, DEADLINE - 1);
        assert!(fails(|| c.close_unfunded(id.clone())));
        assert!(settlement_logs().is_empty());
        set("alice.testnet", 0, DEADLINE);
        c.close_unfunded(id.clone());
        assert_eq!(
            settlement_logs(),
            vec![settlement_log_line(&id, SettlementOutcome::NotFunded)]
        );
        set("lp.testnet", fund_deposit(&c), DEADLINE + 1);
        assert!(fails(|| c.fund_window(id.clone())));
        set("alice.testnet", 0, DEADLINE + 2);
        assert!(fails(|| c.close_unfunded(id.clone())));
        assert!(fails(|| c.claim(id.clone(), Base64VecU8(secret()))));
        assert!(settlement_logs().is_empty());
    }

    /// A direct `lock()` can occupy a window id before registration (griefing
    /// only: the BSC escrow then never sees a `Registered` proof, so the
    /// depositor never locks BNB) but it can never emit a settlement log.
    #[test]
    fn direct_lock_squatting_a_window_id_cannot_forge_settlement() {
        let mut c = new_testnet_vault();
        let id = bound_id(0, DEADLINE, EXPIRY);
        // The attacker names itself recipient so it could claim its own lock.
        lock_as(&mut c, id.clone(), "lp.testnet", "lp.testnet", ONE, START);
        let deposit = window_deposit(&c);
        set("registrar.testnet", deposit, START);
        assert!(fails(|| c.register_window(
            id.clone(),
            "lp.testnet".parse().unwrap(),
            "alice.testnet".parse().unwrap(),
            Base64VecU8(env::sha256(secret())),
            U128(ONE),
            DEADLINE,
            EXPIRY,
            Base64VecU8(binding(0)),
        )));
        set("lp.testnet", 0, START + 1);
        c.claim(id.clone(), Base64VecU8(secret()));
        assert!(settlement_logs().is_empty(), "unbound swap emitted settlement");
        // Ordinary unbound swaps never emit settlement logs either.
        lock(&mut c);
        set("other.testnet", 0, START + 600_000_000_000);
        c.refund(swap_id());
        assert!(settlement_logs().is_empty());
    }

    #[test]
    fn same_hashlock_across_windows_settles_each_window_independently() {
        let mut c = new_testnet_vault();
        let first = register(&mut c, 0, DEADLINE, EXPIRY);
        let second = register(&mut c, 1, DEADLINE, EXPIRY);
        assert_ne!(first, second);
        for id in [&first, &second] {
            set("lp.testnet", fund_deposit(&c), START + 1);
            c.fund_window(id.clone());
        }
        set("alice.testnet", 0, START + 2);
        c.claim(first.clone(), Base64VecU8(secret()));
        assert_eq!(
            settlement_logs(),
            vec![settlement_log_line(&first, SettlementOutcome::Claimed)]
        );
        // The secret is public after the first claim; the second window's
        // outcome is still decided only by its own on-chain transition.
        set("anyone.testnet", 0, EXPIRY);
        c.refund(second.clone());
        assert_eq!(
            settlement_logs(),
            vec![settlement_log_line(&second, SettlementOutcome::Refunded)]
        );
        assert_ne!(
            settlement_log_line(&first, SettlementOutcome::Claimed),
            settlement_log_line(&second, SettlementOutcome::Claimed)
        );
    }
}
