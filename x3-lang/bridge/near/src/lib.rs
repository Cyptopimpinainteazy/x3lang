//! Experimental native NEAR HTLC. Claims assign enforceable internal credits synchronously.
//! Native wallet withdrawals are asynchronous and NEVER count as completed at preimage reveal.
#![allow(clippy::too_many_arguments)]

use near_sdk::json_types::{Base64VecU8, U128};
use near_sdk::{env, near, AccountId, Gas, NearToken, PanicOnDefault, Promise};
use std::collections::BTreeMap;

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
const MIN_NEAR_DEPOSIT: u128 = 1_000_000_000_000_000_000_000; // 0.001 NEAR.
const CALLBACK_GAS: Gas = Gas::from_tgas(15);

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
}

#[near(contract_state)]
#[derive(PanicOnDefault)]
pub struct NativeNearHtlc {
    registrar: AccountId,
    windows: BTreeMap<String, FundingWindow>,
    swaps: BTreeMap<String, Swap>,
    credits: BTreeMap<AccountId, u128>,
    withdrawals_inflight: BTreeMap<AccountId, u128>,
    total_liability: u128,
}

#[near]
impl NativeNearHtlc {
    #[init]
    pub fn new() -> Self {
        // Restricted pilot deployment identity. NEAR contracts have no cryptographically
        // authenticated chain-id hostcall: this prevents ordinary `.near` mainnet
        // accounts from initializing, but a suffix is NOT consensus proof of testnet.
        assert!(
            env::current_account_id().as_str().ends_with(".testnet"),
            "X3_NEAR_TESTNET_ONLY"
        );
        Self {
            registrar: env::predecessor_account_id(),
            windows: BTreeMap::new(),
            swaps: BTreeMap::new(),
            credits: BTreeMap::new(),
            withdrawals_inflight: BTreeMap::new(),
            total_liability: 0,
        }
    }

    /// Testnet pilot: trusted registrar reserves a unique swap identity and a
    /// last funding deadline on-chain BEFORE the liquidity provider funds it.
    /// Registrar/contract storage economics and deployment code identity
    /// require independent review before any real value is permitted.
    pub fn register_window(
        &mut self,
        id: String,
        initiator: AccountId,
        recipient: AccountId,
        hashlock: Base64VecU8,
        amount: U128,
        deadline_ns: u64,
        expiry_ns: u64,
    ) {
        assert_eq!(env::predecessor_account_id(), self.registrar, "registrar only");
        assert!(
            id.len() == 64 && id.bytes().all(|b| b.is_ascii_hexdigit()),
            "bad swap id"
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
        assert!(deadline_ns >= now.saturating_add(MIN_LOCK_NS), "deadline too close");
        assert!(expiry_ns > deadline_ns.saturating_add(MIN_LOCK_NS), "unsafe expiry");
        assert!(expiry_ns <= now.saturating_add(MAX_LOCK_NS), "expiry too far");
        self.windows.insert(
            id,
            FundingWindow {
                initiator,
                recipient,
                hashlock: hashlock.0,
                amount: amount.0,
                deadline_ns,
                expiry_ns,
                state: FundingWindowState::Pending,
            },
        );
    }

    /// The provider can only fund the exact reserved terms before deadline.
    #[payable]
    pub fn fund_window(&mut self, id: String) {
        let window = self.windows.get_mut(&id).expect("unknown funding window");
        assert_eq!(window.state, FundingWindowState::Pending, "not pending");
        assert!(env::block_timestamp() < window.deadline_ns, "funding deadline passed");
        assert_eq!(
            env::predecessor_account_id(),
            window.initiator,
            "wrong funding provider"
        );
        let amount = env::attached_deposit().as_yoctonear();
        assert_eq!(amount, window.amount, "funding amount mismatch");
        self.total_liability = self.total_liability.checked_add(amount).expect("liability overflow");
        self.swaps.insert(
            id,
            Swap {
                initiator: window.initiator.clone(),
                recipient: window.recipient.clone(),
                hashlock: window.hashlock.clone(),
                amount,
                expires_at_ns: window.expiry_ns,
                state: SwapState::Locked,
            },
        );
        window.state = FundingWindowState::Funded;
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
    }

    pub fn window_state(&self, id: String) -> Option<FundingWindowState> {
        self.windows.get(&id).map(|w| w.state.clone())
    }

    pub fn get_window(&self, id: String) -> Option<FundingWindow> {
        self.windows.get(&id).cloned()
    }

    #[payable]
    pub fn lock(&mut self, id: String, recipient: AccountId, hashlock: Base64VecU8, expires_at_ns: u64) {
        assert!(
            id.len() == 64 && id.bytes().all(|b| b.is_ascii_hexdigit()),
            "swap id must be 32 bytes in hexadecimal"
        );
        assert!(
            !self.swaps.contains_key(&id) && !self.windows.contains_key(&id),
            "swap id already used"
        );
        assert_eq!(hashlock.0.len(), 32, "hashlock must be 32 bytes");
        assert!(hashlock.0.iter().any(|b| *b != 0), "zero hashlock prohibited");
        let current = env::block_timestamp();
        assert!(expires_at_ns > current.saturating_add(MIN_LOCK_NS), "expiry too short");
        assert!(expires_at_ns <= current.saturating_add(MAX_LOCK_NS), "expiry too far");
        let amount = env::attached_deposit().as_yoctonear();
        assert!(amount >= MIN_NEAR_DEPOSIT, "minimum 0.001 NEAR");
        assert_ne!(recipient, env::current_account_id(), "cannot credit vault itself");
        self.total_liability = self.total_liability.checked_add(amount).expect("liability overflow");
        self.swaps.insert(
            id,
            Swap {
                initiator: env::predecessor_account_id(),
                recipient,
                hashlock: hashlock.0,
                amount,
                expires_at_ns,
                state: SwapState::Locked,
            },
        );
    }

    /// The source-chain observer must wait for THIS finalized credit state,
    /// not merely for the preimage to appear in a pending NEAR transaction.
    pub fn claim(&mut self, id: String, preimage: Base64VecU8) {
        assert_eq!(preimage.0.len(), 32, "preimage must be 32 raw bytes");
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
        self.add_credit(recipient, amount);
    }

    /// Expiration assigns the initiator an on-chain withdrawal credit.
    pub fn refund(&mut self, id: String) {
        let swap = self.swaps.get_mut(&id).expect("unknown swap");
        assert_eq!(swap.state, SwapState::Locked, "not locked");
        assert_eq!(swap.initiator, env::predecessor_account_id(), "not initiator");
        assert!(env::block_timestamp() >= swap.expires_at_ns, "not expired");
        let initiator = swap.initiator.clone();
        let amount = swap.amount;
        swap.state = SwapState::Refunded;
        self.add_credit(initiator, amount);
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
            .block_timestamp(time)
            .build();
        testing_env!(ctx);
    }
    #[test]
    #[should_panic(expected = "X3_NEAR_TESTNET_ONLY")]
    fn initialization_rejects_mainnet_account_before_vault_state_exists() {
        let ctx = VMContextBuilder::new()
            .current_account_id("x3-htlc.near".parse().unwrap())
            .predecessor_account_id("alice.near".parse().unwrap())
            .build();
        testing_env!(ctx);
        let _ = NativeNearHtlc::new();
    }

    #[test]
    fn initialization_accepts_testnet_account() {
        set("registrar.testnet", 0, START);
        let state = NativeNearHtlc::new();
        assert_eq!(state.registrar.as_str(), "registrar.testnet");
    }

    const START: u64 = 1_700_000_000_000_000_000;
    fn secret() -> Vec<u8> {
        vec![3u8; 32]
    }
    fn swap_id() -> String {
        "a".repeat(64)
    }
    fn lock(c: &mut NativeNearHtlc) {
        set("lp.testnet", 10u128.pow(24), START);
        c.lock(
            swap_id(),
            "alice.testnet".parse().unwrap(),
            Base64VecU8(env::sha256(secret())),
            START + 600_000_000_000,
        );
    }
    #[test]
    fn funding_window_json_view_has_expected_wire_fields() {
        // Existing unit test used the implicit default near-sdk account,
        // which does not express a testnet deployment identity.
        set("deployer.testnet", 0, START);
        let mut c = NativeNearHtlc::new();
        set("registrar.testnet", 0, START);
        // Separate deployer from registrar is tested elsewhere; test the wire view.
        // Contract was created in the previous VM context, so this uses its
        // deployed registrar identity from construction, not an external key.
        let registrar = c.registrar.to_string();
        set(&registrar, 0, START);
        c.register_window(
            swap_id(),
            "lp.testnet".parse().unwrap(),
            "alice.testnet".parse().unwrap(),
            Base64VecU8(env::sha256(secret())),
            U128(10u128.pow(24)),
            START + 600_000_000_000,
            START + 1_200_000_000_000,
        );
        let json = near_sdk::serde_json::to_value(c.get_window(swap_id()).unwrap()).unwrap();
        assert_eq!(json["state"], "Pending");
        assert!(json["deadline_ns"].is_number());
        assert!(json["hashlock"].is_array());
        assert_eq!(json["amount"], 10u128.pow(24).to_string());
    }

    #[test]
    fn can_never_close_funding_window_early_or_fund_at_deadline() {
        set("registrar.testnet", 0, START);
        let mut c = NativeNearHtlc::new();
        let deadline = START + 600_000_000_000;
        c.register_window(
            swap_id(),
            "lp.testnet".parse().unwrap(),
            "alice.testnet".parse().unwrap(),
            Base64VecU8(env::sha256(secret())),
            U128(10u128.pow(24)),
            deadline,
            START + 1_200_000_000_000,
        );
        set("anyone.testnet", 0, deadline - 1);
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| c.close_unfunded(swap_id()))).is_err());
        set("lp.testnet", 10u128.pow(24), deadline);
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| c.fund_window(swap_id()))).is_err());
        set("anyone.testnet", 0, deadline);
        c.close_unfunded(swap_id());
        assert_eq!(c.window_state(swap_id()), Some(FundingWindowState::NotFunded));
        assert_eq!(c.liability().0, 0);
    }

    #[test]
    fn existing_escrow_json_amount_is_exact_decimal_string() {
        // Existing unit test used the implicit default near-sdk account,
        // which does not express a testnet deployment identity.
        set("deployer.testnet", 0, START);
        let mut c = NativeNearHtlc::new();
        lock(&mut c);
        let json = near_sdk::serde_json::to_value(c.get(swap_id()).unwrap()).unwrap();
        assert_eq!(json["amount"], 10u128.pow(24).to_string());
    }

    #[test]
    fn registered_window_can_be_irreversibly_closed_when_never_funded() {
        set("registrar.testnet", 0, START);
        let mut c = NativeNearHtlc::new();
        set("registrar.testnet", 0, START);
        c.register_window(
            swap_id(),
            "lp.testnet".parse().unwrap(),
            "alice.testnet".parse().unwrap(),
            Base64VecU8(env::sha256(secret())),
            U128(10u128.pow(24)),
            START + 600_000_000_000,
            START + 1_200_000_000_000,
        );
        assert_eq!(c.window_state(swap_id()), Some(FundingWindowState::Pending));
        set("anyone.testnet", 0, START + 600_000_000_000);
        c.close_unfunded(swap_id());
        assert_eq!(c.window_state(swap_id()), Some(FundingWindowState::NotFunded));
        assert!(c.get(swap_id()).is_none());
        set("lp.testnet", 10u128.pow(24), START + 600_000_000_001);
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| c.fund_window(swap_id()))).is_err());
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| c.lock(
            swap_id(),
            "alice.testnet".parse().unwrap(),
            Base64VecU8(env::sha256(secret())),
            START + 1_200_000_000_000
        )))
        .is_err());
        assert_eq!(c.liability().0, 0);
    }

    #[test]
    fn funded_window_cannot_be_closed_as_unfunded() {
        set("registrar.testnet", 0, START);
        let mut c = NativeNearHtlc::new();
        set("registrar.testnet", 0, START);
        c.register_window(
            swap_id(),
            "lp.testnet".parse().unwrap(),
            "alice.testnet".parse().unwrap(),
            Base64VecU8(env::sha256(secret())),
            U128(10u128.pow(24)),
            START + 600_000_000_000,
            START + 1_200_000_000_000,
        );
        set("lp.testnet", 10u128.pow(24), START + 1);
        c.fund_window(swap_id());
        assert_eq!(c.window_state(swap_id()), Some(FundingWindowState::Funded));
        assert_eq!(c.get(swap_id()).unwrap().amount, 10u128.pow(24));
        set("anyone.testnet", 0, START + 600_000_000_000);
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| c.close_unfunded(swap_id()))).is_err());
    }

    #[test]
    fn unauthorized_registrar_and_wrong_lp_or_amount_rejected() {
        set("registrar.testnet", 0, START);
        let mut c = NativeNearHtlc::new();
        set("stranger.testnet", 0, START);
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| c.register_window(
                swap_id(),
                "lp.testnet".parse().unwrap(),
                "alice.testnet".parse().unwrap(),
                Base64VecU8(env::sha256(secret())),
                U128(10u128.pow(24)),
                START + 600_000_000_000,
                START + 1_200_000_000_000
            )))
            .is_err()
        );
        set("registrar.testnet", 0, START);
        c.register_window(
            swap_id(),
            "lp.testnet".parse().unwrap(),
            "alice.testnet".parse().unwrap(),
            Base64VecU8(env::sha256(secret())),
            U128(10u128.pow(24)),
            START + 600_000_000_000,
            START + 1_200_000_000_000,
        );
        set("attacker.testnet", 10u128.pow(24), START + 1);
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| c.fund_window(swap_id()))).is_err());
        set("lp.testnet", 10u128.pow(24) - 1, START + 2);
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| c.fund_window(swap_id()))).is_err());
        assert_eq!(c.window_state(swap_id()), Some(FundingWindowState::Pending));
    }

    #[test]
    fn locks_native_near_and_records_payout_owner() {
        // Existing unit test used the implicit default near-sdk account,
        // which does not express a testnet deployment identity.
        set("deployer.testnet", 0, START);
        let mut c = NativeNearHtlc::new();
        lock(&mut c);
        let swap = c.get(swap_id()).unwrap();
        assert_eq!(swap.amount, 10u128.pow(24));
        assert_eq!(swap.recipient.as_str(), "alice.testnet");
        assert_eq!(swap.state, SwapState::Locked);
        assert_eq!(c.liability().0, 10u128.pow(24));
    }
    #[test]
    fn claim_credits_on_chain_before_secret_becomes_useful() {
        // Existing unit test used the implicit default near-sdk account,
        // which does not express a testnet deployment identity.
        set("deployer.testnet", 0, START);
        let mut c = NativeNearHtlc::new();
        lock(&mut c);
        set("alice.testnet", 0, START + 1);
        c.claim(swap_id(), Base64VecU8(secret()));
        assert_eq!(c.get(swap_id()).unwrap().state, SwapState::Claimed);
        assert_eq!(c.credit_of("alice.testnet".parse().unwrap()).0, 10u128.pow(24));
        assert_eq!(c.liability().0, 10u128.pow(24));
    }
    #[test]
    fn only_recipient_with_correct_secret_can_claim() {
        // Existing unit test used the implicit default near-sdk account,
        // which does not express a testnet deployment identity.
        set("deployer.testnet", 0, START);
        let mut c = NativeNearHtlc::new();
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
        // Existing unit test used the implicit default near-sdk account,
        // which does not express a testnet deployment identity.
        set("deployer.testnet", 0, START);
        let mut c = NativeNearHtlc::new();
        lock(&mut c);
        set("alice.testnet", 0, START + 1);
        c.claim(swap_id(), Base64VecU8(secret()));
        set("lp.testnet", 10u128.pow(24), START + 2);
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| c.lock(
            swap_id(),
            "alice.testnet".parse().unwrap(),
            Base64VecU8(env::sha256(secret())),
            START + 600_000_000_000
        )))
        .is_err());
    }
    #[test]
    fn refund_credits_only_owner_after_expiry() {
        // Existing unit test used the implicit default near-sdk account,
        // which does not express a testnet deployment identity.
        set("deployer.testnet", 0, START);
        let mut c = NativeNearHtlc::new();
        lock(&mut c);
        set("lp.testnet", 0, START + 1);
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| c.refund(swap_id()))).is_err());
        set("other.testnet", 0, START + 600_000_000_000);
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| c.refund(swap_id()))).is_err());
        set("lp.testnet", 0, START + 600_000_000_000);
        c.refund(swap_id());
        assert_eq!(c.get(swap_id()).unwrap().state, SwapState::Refunded);
        assert_eq!(c.credit_of("lp.testnet".parse().unwrap()).0, 10u128.pow(24));
    }
    #[test]
    fn withdrawal_moves_credit_to_inflight_without_double_spend() {
        // Existing unit test used the implicit default near-sdk account,
        // which does not express a testnet deployment identity.
        set("deployer.testnet", 0, START);
        let mut c = NativeNearHtlc::new();
        lock(&mut c);
        set("alice.testnet", 0, START + 1);
        c.claim(swap_id(), Base64VecU8(secret()));
        let _promise = c.withdraw();
        assert_eq!(c.credit_of("alice.testnet".parse().unwrap()).0, 0);
        assert_eq!(
            c.pending_withdrawal_of("alice.testnet".parse().unwrap()).0,
            10u128.pow(24)
        );
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
            .block_timestamp(time)
            .build()
    }

    #[test]
    fn failed_async_wallet_transfer_restores_claimant_credit() {
        // Existing unit test used the implicit default near-sdk account,
        // which does not express a testnet deployment identity.
        set("deployer.testnet", 0, START);
        let mut c = NativeNearHtlc::new();
        lock(&mut c);
        set("alice.testnet", 0, START + 1);
        c.claim(swap_id(), Base64VecU8(secret()));
        let _ = c.withdraw();
        testing_env!(
            callback_ctx(START + 2),
            near_sdk::test_vm_config(),
            near_sdk::RuntimeFeesConfig::test(),
            std::collections::HashMap::new(),
            vec![near_sdk::PromiseResult::Failed]
        );
        assert!(!c.on_withdraw("alice.testnet".parse().unwrap()));
        assert_eq!(c.credit_of("alice.testnet".parse().unwrap()).0, 10u128.pow(24));
        assert_eq!(c.pending_withdrawal_of("alice.testnet".parse().unwrap()).0, 0);
        assert_eq!(c.liability().0, 10u128.pow(24));
    }

    #[test]
    fn confirmed_async_wallet_transfer_reduces_liability_once() {
        // Existing unit test used the implicit default near-sdk account,
        // which does not express a testnet deployment identity.
        set("deployer.testnet", 0, START);
        let mut c = NativeNearHtlc::new();
        lock(&mut c);
        set("alice.testnet", 0, START + 1);
        c.claim(swap_id(), Base64VecU8(secret()));
        let _ = c.withdraw();
        testing_env!(
            callback_ctx(START + 2),
            near_sdk::test_vm_config(),
            near_sdk::RuntimeFeesConfig::test(),
            std::collections::HashMap::new(),
            vec![near_sdk::PromiseResult::Successful(vec![])]
        );
        assert!(c.on_withdraw("alice.testnet".parse().unwrap()));
        assert_eq!(c.credit_of("alice.testnet".parse().unwrap()).0, 0);
        assert_eq!(c.pending_withdrawal_of("alice.testnet".parse().unwrap()).0, 0);
        assert_eq!(c.liability().0, 0);
    }
    #[test]
    fn rejects_invalid_lock_details_and_zero_amount() {
        // Existing unit test used the implicit default near-sdk account,
        // which does not express a testnet deployment identity.
        set("deployer.testnet", 0, START);
        let mut c = NativeNearHtlc::new();
        set("lp.testnet", 0, START);
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| c.lock(
            swap_id(),
            "alice.testnet".parse().unwrap(),
            Base64VecU8(env::sha256(secret())),
            START + 600_000_000_000
        )))
        .is_err());
        set("lp.testnet", 10u128.pow(24), START);
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| c.lock(
            "not-hex".into(),
            "alice.testnet".parse().unwrap(),
            Base64VecU8(env::sha256(secret())),
            START + 600_000_000_000
        )))
        .is_err());
    }
}
