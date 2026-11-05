//! Experimental native NEAR HTLC. Claims assign enforceable internal credits synchronously.
//! Native wallet withdrawals are asynchronous and NEVER count as completed at preimage reveal.
use near_sdk::json_types::{Base64VecU8, U128};
use near_sdk::{env, near, AccountId, Gas, NearToken, PanicOnDefault, Promise};
use std::collections::BTreeMap;

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
#[derive(Clone)]
pub struct Swap {
    pub initiator: AccountId,
    pub recipient: AccountId,
    pub hashlock: Vec<u8>,
    pub amount: u128,
    pub expires_at_ns: u64,
    pub state: SwapState,
}

#[near(contract_state)]
#[derive(PanicOnDefault)]
pub struct NativeNearHtlc {
    swaps: BTreeMap<String, Swap>,
    credits: BTreeMap<AccountId, u128>,
    withdrawals_inflight: BTreeMap<AccountId, u128>,
    total_liability: u128,
}

#[near]
impl NativeNearHtlc {
    #[init]
    pub fn new() -> Self {
        Self {
            swaps: BTreeMap::new(),
            credits: BTreeMap::new(),
            withdrawals_inflight: BTreeMap::new(),
            total_liability: 0,
        }
    }

    #[payable]
    pub fn lock(&mut self, id: String, recipient: AccountId, hashlock: Base64VecU8, expires_at_ns: u64) {
        assert!(
            id.len() == 64 && id.bytes().all(|b| b.is_ascii_hexdigit()),
            "swap id must be 32 bytes in hexadecimal"
        );
        assert!(!self.swaps.contains_key(&id), "swap id already used");
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
            Base64VecU8(env::sha256(&secret())),
            START + 600_000_000_000,
        );
    }
    #[test]
    fn locks_native_near_and_records_payout_owner() {
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
        let mut c = NativeNearHtlc::new();
        lock(&mut c);
        set("alice.testnet", 0, START + 1);
        c.claim(swap_id(), Base64VecU8(secret()));
        set("lp.testnet", 10u128.pow(24), START + 2);
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| c.lock(
            swap_id(),
            "alice.testnet".parse().unwrap(),
            Base64VecU8(env::sha256(&secret())),
            START + 600_000_000_000
        )))
        .is_err());
    }
    #[test]
    fn refund_credits_only_owner_after_expiry() {
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
        let mut c = NativeNearHtlc::new();
        set("lp.testnet", 0, START);
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| c.lock(
            swap_id(),
            "alice.testnet".parse().unwrap(),
            Base64VecU8(env::sha256(&secret())),
            START + 600_000_000_000
        )))
        .is_err());
        set("lp.testnet", 10u128.pow(24), START);
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| c.lock(
            "not-hex".into(),
            "alice.testnet".parse().unwrap(),
            Base64VecU8(env::sha256(&secret())),
            START + 600_000_000_000
        )))
        .is_err());
    }
}
