//! Dotx3 HACKATHON ONLY COIN (DX3HACK)
//!
//! A demo credit for NEAR HACKATHON 2026 judges. It has no value and no market, by contract:
//!
//! * Only allowlisted accounts can hold it. The faucet operator may only allowlist direct
//!   subaccounts of one parent account (judge vaults), so no wallet, DEX pool or bridge can
//!   ever receive it. The owner allowlists the few contracts that need it (Guard, merchant).
//! * The faucet mints a fixed amount per claim, with a per-holder cooldown, a daily cap and
//!   a hard maximum supply.
//! * Every sender is limited to a fixed number of transfers per UTC day.
//! * At `expires_at` every balance and the total supply read as zero and every state-changing
//!   method except `purge` is refused. The time can be moved earlier (`end_now`), never later.
//! * The storage cleanup after expiry (`purge`) is permissionless but only runs when the caller
//!   presents the exact `.x3` burn program whose SHA-256 was bound at deploy. The contract
//!   checks the hash on-chain.

// Same rule as x3-guard: without `--cfg near`, near-sdk 5.x discards every refusal message
// and traps with a bare `unreachable`, so judges would see crashes instead of reasons.
#[cfg(all(target_arch = "wasm32", not(near)))]
compile_error!(
    "dx3-hackathon-coin: WASM must be built with `--cfg near` (cargo-near, or this crate's \
     .cargo/config.toml). Without it near-sdk drops every refusal reason."
);

use near_contract_standards::fungible_token::core::FungibleTokenCore;
use near_contract_standards::fungible_token::events::{FtBurn, FtMint};
use near_contract_standards::fungible_token::metadata::{
    FungibleTokenMetadata, FungibleTokenMetadataProvider, FT_METADATA_SPEC,
};
use near_contract_standards::fungible_token::resolver::FungibleTokenResolver;
use near_contract_standards::fungible_token::FungibleToken;
use near_contract_standards::storage_management::{StorageBalance, StorageBalanceBounds, StorageManagement};
use near_sdk::json_types::{U128, U64};
use near_sdk::store::{LookupMap, LookupSet};
use near_sdk::{env, near, require, AccountId, BorshStorageKey, NearToken, PanicOnDefault, PromiseOrValue};

pub const NAME: &str = "Dotx3 HACKATHON ONLY COIN";
pub const SYMBOL: &str = "DX3HACK";
pub const DECIMALS: u8 = 6;
const NS_PER_SEC: u64 = 1_000_000_000;
const NS_PER_DAY: u64 = 86_400 * NS_PER_SEC;
/// Deploy-time guard against a typo setting a far-future expiry.
const MAX_LIFETIME_NS: u64 = 90 * NS_PER_DAY;
const MAX_PURGE_BATCH: usize = 100;
const ICON: &str = "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 64 64'%3E%3Crect width='64' height='64' rx='14' fill='%233247c9'/%3E%3Ctext x='32' y='41' font-family='monospace' font-size='22' font-weight='700' text-anchor='middle' fill='white'%3E.x3%3C/text%3E%3C/svg%3E";

#[derive(BorshStorageKey)]
#[near(serializers = [borsh])]
enum StorageKey {
    Token,
    Holders,
    LastClaim,
    TransferCounts,
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DayCount {
    pub day: u64,
    pub count: u32,
}

#[near(serializers = [json])]
#[derive(Debug)]
pub struct HackathonConfig {
    pub owner_id: AccountId,
    pub faucet_operator_id: AccountId,
    pub holder_parent_id: AccountId,
    pub now: U64,
    pub expires_at: U64,
    pub active: bool,
    pub paused: bool,
    pub burn_program_sha256: String,
    pub max_supply: U128,
    pub minted_total: U128,
    pub claim_amount: U128,
    pub claim_cooldown_sec: u64,
    pub faucet_daily_cap: U128,
    pub faucet_minted_today: U128,
    pub max_transfers_per_day: u32,
    pub holders: u32,
    pub max_holders: u32,
}

#[near(contract_state)]
#[derive(PanicOnDefault)]
pub struct Contract {
    token: FungibleToken,
    metadata: FungibleTokenMetadata,
    owner_id: AccountId,
    faucet_operator_id: AccountId,
    holder_parent_id: AccountId,
    holders: LookupSet<AccountId>,
    holder_count: u32,
    max_holders: u32,
    max_supply: u128,
    minted_total: u128,
    claim_amount: u128,
    claim_cooldown_ns: u64,
    faucet_daily_cap: u128,
    faucet_day: u64,
    faucet_day_minted: u128,
    last_claim: LookupMap<AccountId, u64>,
    max_transfers_per_day: u32,
    transfer_counts: LookupMap<AccountId, DayCount>,
    expires_at_ns: u64,
    paused: bool,
    burn_program_sha256: [u8; 32],
}

fn hex_to_32(s: &str) -> [u8; 32] {
    let s = s.trim().to_ascii_lowercase();
    require!(s.len() == 64, "burn_program_sha256 must be 64 hex characters");
    let mut out = [0u8; 32];
    for (i, chunk) in s.as_bytes().chunks(2).enumerate() {
        let hi = (chunk[0] as char).to_digit(16);
        let lo = (chunk[1] as char).to_digit(16);
        match (hi, lo) {
            (Some(h), Some(l)) => out[i] = (h * 16 + l) as u8,
            _ => env::panic_str("burn_program_sha256 must be hex"),
        }
    }
    out
}

fn to_hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{:02x}", x)).collect()
}

/// True only for `<label>.<parent>` where `<label>` has no dots.
fn is_direct_subaccount(child: &AccountId, parent: &AccountId) -> bool {
    let c = child.as_str();
    let p = parent.as_str();
    match c.strip_suffix(p).and_then(|rest| rest.strip_suffix('.')) {
        Some(label) => !label.is_empty() && !label.contains('.'),
        None => false,
    }
}

#[near]
impl Contract {
    #[init]
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        owner_id: AccountId,
        faucet_operator_id: AccountId,
        holder_parent_id: AccountId,
        expires_at: U64,
        burn_program_sha256: String,
        max_supply: U128,
        claim_amount: U128,
        claim_cooldown_sec: u64,
        faucet_daily_cap: U128,
        max_transfers_per_day: u32,
        max_holders: u32,
    ) -> Self {
        let now = env::block_timestamp();
        require!(expires_at.0 > now, "expires_at must be in the future");
        require!(
            expires_at.0 <= now + MAX_LIFETIME_NS,
            "expires_at must be within 90 days"
        );
        require!(claim_amount.0 > 0, "claim_amount must be positive");
        require!(
            claim_amount.0 <= faucet_daily_cap.0,
            "claim_amount can't exceed the daily cap"
        );
        require!(faucet_daily_cap.0 <= max_supply.0, "daily cap can't exceed max supply");
        require!(max_transfers_per_day > 0, "max_transfers_per_day must be positive");
        require!(max_holders > 0, "max_holders must be positive");
        require!(owner_id != faucet_operator_id, "owner and faucet operator must differ");
        let metadata = FungibleTokenMetadata {
            spec: FT_METADATA_SPEC.to_string(),
            name: NAME.to_string(),
            symbol: SYMBOL.to_string(),
            icon: Some(ICON.to_string()),
            reference: None,
            reference_hash: None,
            decimals: DECIMALS,
        };
        metadata.assert_valid();
        Self {
            token: FungibleToken::new(StorageKey::Token),
            metadata,
            owner_id,
            faucet_operator_id,
            holder_parent_id,
            holders: LookupSet::new(StorageKey::Holders),
            holder_count: 0,
            max_holders,
            max_supply: max_supply.0,
            minted_total: 0,
            claim_amount: claim_amount.0,
            claim_cooldown_ns: claim_cooldown_sec * NS_PER_SEC,
            faucet_daily_cap: faucet_daily_cap.0,
            faucet_day: now / NS_PER_DAY,
            faucet_day_minted: 0,
            last_claim: LookupMap::new(StorageKey::LastClaim),
            max_transfers_per_day,
            transfer_counts: LookupMap::new(StorageKey::TransferCounts),
            expires_at_ns: expires_at.0,
            paused: false,
            burn_program_sha256: hex_to_32(&burn_program_sha256),
        }
    }

    /* ---------- internal checks ---------- */

    fn is_active(&self) -> bool {
        env::block_timestamp() < self.expires_at_ns
    }

    fn assert_live(&self) {
        require!(self.is_active(), "The hackathon is over. DX3HACK has burned.");
        require!(!self.paused, "DX3HACK is paused by the owner");
    }

    fn assert_owner(&self) {
        require!(
            env::predecessor_account_id() == self.owner_id,
            "Only the owner can do this"
        );
    }

    fn count_transfer(&mut self, sender: &AccountId) {
        if *sender == self.owner_id {
            return;
        }
        let day = env::block_timestamp() / NS_PER_DAY;
        let current = match self.transfer_counts.get(sender) {
            Some(dc) if dc.day == day => *dc,
            _ => DayCount { day, count: 0 },
        };
        require!(
            current.count < self.max_transfers_per_day,
            "Daily transfer limit reached for this account"
        );
        self.transfer_counts.insert(
            sender.clone(),
            DayCount {
                day,
                count: current.count + 1,
            },
        );
    }

    fn check_transfer(&mut self, receiver_id: &AccountId) {
        self.assert_live();
        let sender = env::predecessor_account_id();
        require!(
            self.holders.contains(&sender),
            "Sender isn't an approved DX3HACK holder"
        );
        require!(
            self.holders.contains(receiver_id),
            "Receiver isn't an approved DX3HACK holder"
        );
        self.count_transfer(&sender);
    }

    fn mint_to(&mut self, receiver_id: &AccountId, amount: u128, memo: &str) {
        require!(
            self.holders.contains(receiver_id),
            "Receiver isn't an approved DX3HACK holder"
        );
        let total = self
            .minted_total
            .checked_add(amount)
            .unwrap_or_else(|| env::panic_str("Overflow"));
        require!(total <= self.max_supply, "Max supply reached");
        self.minted_total = total;
        self.token.internal_deposit(receiver_id, amount);
        FtMint {
            owner_id: receiver_id,
            amount: U128(amount),
            memo: Some(memo),
        }
        .emit();
    }

    /* ---------- holders ---------- */

    /// Owner: any account. Faucet operator: only direct subaccounts of `holder_parent_id`.
    /// Registers the account for storage, paid from the contract's own balance.
    pub fn add_holder(&mut self, account_id: AccountId) {
        self.assert_live();
        let caller = env::predecessor_account_id();
        if caller == self.faucet_operator_id {
            require!(
                is_direct_subaccount(&account_id, &self.holder_parent_id),
                "The faucet operator can only add direct subaccounts of the holder parent"
            );
        } else {
            self.assert_owner();
        }
        if self.holders.contains(&account_id) {
            return;
        }
        require!(self.holder_count < self.max_holders, "Holder limit reached");
        self.holders.insert(account_id.clone());
        self.holder_count += 1;
        if !self.token.accounts.contains_key(&account_id) {
            self.token.internal_register_account(&account_id);
        }
    }

    /// Owner: stop an account from sending or receiving. Its balance stays until the burn.
    pub fn remove_holder(&mut self, account_id: AccountId) {
        self.assert_owner();
        if self.holders.remove(&account_id) {
            self.holder_count -= 1;
        }
    }

    /* ---------- faucet and minting ---------- */

    /// Faucet operator only. Mints `claim_amount` to an approved holder.
    pub fn faucet_claim(&mut self, receiver_id: AccountId) -> U128 {
        self.assert_live();
        require!(
            env::predecessor_account_id() == self.faucet_operator_id,
            "Only the faucet operator can claim"
        );
        let now = env::block_timestamp();
        if let Some(last) = self.last_claim.get(&receiver_id) {
            require!(
                now >= last + self.claim_cooldown_ns,
                "This holder claimed recently. Try again later."
            );
        }
        let day = now / NS_PER_DAY;
        if day != self.faucet_day {
            self.faucet_day = day;
            self.faucet_day_minted = 0;
        }
        let today = self.faucet_day_minted + self.claim_amount;
        require!(today <= self.faucet_daily_cap, "The faucet's daily limit is spent");
        self.faucet_day_minted = today;
        self.last_claim.insert(receiver_id.clone(), now);
        self.mint_to(&receiver_id, self.claim_amount, "faucet");
        U128(self.claim_amount)
    }

    /// Owner only. Seeds approved contracts (Guard, merchant) within max supply.
    pub fn mint(&mut self, receiver_id: AccountId, amount: U128) {
        self.assert_live();
        self.assert_owner();
        require!(amount.0 > 0, "Amount must be positive");
        self.mint_to(&receiver_id, amount.0, "owner mint");
    }

    /* ---------- owner controls ---------- */

    pub fn set_paused(&mut self, paused: bool) {
        self.assert_owner();
        self.paused = paused;
    }

    pub fn set_faucet_operator(&mut self, faucet_operator_id: AccountId) {
        self.assert_owner();
        require!(
            faucet_operator_id != self.owner_id,
            "owner and faucet operator must differ"
        );
        self.faucet_operator_id = faucet_operator_id;
    }

    /// Owner only. Moves the burn earlier, to now. There is no way to move it later.
    pub fn end_now(&mut self) {
        self.assert_owner();
        let now = env::block_timestamp();
        if now < self.expires_at_ns {
            self.expires_at_ns = now;
        }
    }

    /* ---------- burn ---------- */

    /// After expiry, anyone may call this to delete balances and storage. It runs only with the
    /// exact `.x3` burn program bound at deploy; the contract checks its SHA-256.
    /// Returns the number of accounts purged.
    pub fn purge(&mut self, account_ids: Vec<AccountId>, x3_program: String) -> u32 {
        require!(!self.is_active(), "DX3HACK hasn't expired yet");
        require!(account_ids.len() <= MAX_PURGE_BATCH, "At most 100 accounts per call");
        let hash = env::sha256(x3_program.as_bytes());
        require!(
            hash.as_slice() == self.burn_program_sha256,
            "x3_program doesn't match the burn program bound at deploy"
        );
        let mut purged = 0u32;
        for acc in account_ids.iter() {
            if let Some(balance) = self.token.accounts.remove(acc) {
                self.token.total_supply = self.token.total_supply.saturating_sub(balance);
                if balance > 0 {
                    FtBurn {
                        owner_id: acc,
                        amount: U128(balance),
                        memo: Some("x3 burn program"),
                    }
                    .emit();
                }
                purged += 1;
            }
            if self.holders.remove(acc) {
                self.holder_count -= 1;
            }
            self.last_claim.remove(acc);
            self.transfer_counts.remove(acc);
        }
        purged
    }

    /* ---------- views ---------- */

    pub fn hackathon_config(&self) -> HackathonConfig {
        let now = env::block_timestamp();
        let today = now / NS_PER_DAY;
        HackathonConfig {
            owner_id: self.owner_id.clone(),
            faucet_operator_id: self.faucet_operator_id.clone(),
            holder_parent_id: self.holder_parent_id.clone(),
            now: U64(now),
            expires_at: U64(self.expires_at_ns),
            active: self.is_active(),
            paused: self.paused,
            burn_program_sha256: to_hex(&self.burn_program_sha256),
            max_supply: U128(self.max_supply),
            minted_total: U128(self.minted_total),
            claim_amount: U128(self.claim_amount),
            claim_cooldown_sec: self.claim_cooldown_ns / NS_PER_SEC,
            faucet_daily_cap: U128(self.faucet_daily_cap),
            faucet_minted_today: U128(if today == self.faucet_day {
                self.faucet_day_minted
            } else {
                0
            }),
            max_transfers_per_day: self.max_transfers_per_day,
            holders: self.holder_count,
            max_holders: self.max_holders,
        }
    }

    pub fn is_holder(&self, account_id: AccountId) -> bool {
        self.holders.contains(&account_id)
    }

    /// Nanosecond timestamp when this holder can claim again; 0 if now.
    pub fn claim_available_at(&self, account_id: AccountId) -> U64 {
        match self.last_claim.get(&account_id) {
            Some(last) if env::block_timestamp() < last + self.claim_cooldown_ns => U64(last + self.claim_cooldown_ns),
            _ => U64(0),
        }
    }

    pub fn transfers_left_today(&self, account_id: AccountId) -> u32 {
        let day = env::block_timestamp() / NS_PER_DAY;
        match self.transfer_counts.get(&account_id) {
            Some(dc) if dc.day == day => self.max_transfers_per_day.saturating_sub(dc.count),
            _ => self.max_transfers_per_day,
        }
    }
}

#[near]
impl FungibleTokenCore for Contract {
    #[payable]
    fn ft_transfer(&mut self, receiver_id: AccountId, amount: U128, memo: Option<String>) {
        self.check_transfer(&receiver_id);
        self.token.ft_transfer(receiver_id, amount, memo)
    }

    #[payable]
    fn ft_transfer_call(
        &mut self,
        receiver_id: AccountId,
        amount: U128,
        memo: Option<String>,
        msg: String,
    ) -> PromiseOrValue<U128> {
        self.check_transfer(&receiver_id);
        self.token.ft_transfer_call(receiver_id, amount, memo, msg)
    }

    fn ft_total_supply(&self) -> U128 {
        if self.is_active() {
            self.token.ft_total_supply()
        } else {
            U128(0)
        }
    }

    fn ft_balance_of(&self, account_id: AccountId) -> U128 {
        if self.is_active() {
            self.token.ft_balance_of(account_id)
        } else {
            U128(0)
        }
    }
}

#[near]
impl FungibleTokenResolver for Contract {
    #[private]
    fn ft_resolve_transfer(&mut self, sender_id: AccountId, receiver_id: AccountId, amount: U128) -> U128 {
        let (used, burned) = self.token.internal_ft_resolve_transfer(&sender_id, receiver_id, amount);
        if burned > 0 {
            FtBurn {
                owner_id: &sender_id,
                amount: U128(burned),
                memo: Some("refund to removed account"),
            }
            .emit();
        }
        used.into()
    }
}

#[near]
impl StorageManagement for Contract {
    #[payable]
    fn storage_deposit(&mut self, account_id: Option<AccountId>, registration_only: Option<bool>) -> StorageBalance {
        let acc = account_id.clone().unwrap_or_else(env::predecessor_account_id);
        require!(
            self.holders.contains(&acc),
            "Only approved DX3HACK holders can register"
        );
        self.token.storage_deposit(account_id, registration_only)
    }

    #[payable]
    fn storage_withdraw(&mut self, amount: Option<NearToken>) -> StorageBalance {
        self.token.storage_withdraw(amount)
    }

    #[payable]
    fn storage_unregister(&mut self, force: Option<bool>) -> bool {
        let _ = force;
        env::panic_str("Holders are removed by the owner or by the post-expiry purge")
    }

    fn storage_balance_bounds(&self) -> StorageBalanceBounds {
        self.token.storage_balance_bounds()
    }

    fn storage_balance_of(&self, account_id: AccountId) -> Option<StorageBalance> {
        self.token.storage_balance_of(account_id)
    }
}

#[near]
impl FungibleTokenMetadataProvider for Contract {
    fn ft_metadata(&self) -> FungibleTokenMetadata {
        self.metadata.clone()
    }
}

#[cfg(test)]
mod tests;
