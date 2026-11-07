//! TESTNET ONLY: a custody-bound contract, never an advisory EOA Guard.
//! A scoped agent key cannot withdraw native NEAR directly from this vault.
use near_sdk::json_types::U128;
use near_sdk::{env, near, AccountId, Gas, NearToken, PanicOnDefault, Promise};
use std::collections::{BTreeMap, BTreeSet};

// BTreeMap/BTreeSet entries are serialized into root contract state and
// permanently retain every used nonce. Bound per-vault lifetime storage
// until an audited persistent-store and recovery design is implemented.
const MAX_PAYMENT_RECORDS: usize = 64;
// Additional margin beyond the live node's storage stake; never consume
// the reserved balance needed to execute callbacks and maintain contract state.
const MIN_VAULT_GAS_RESERVE: u128 = 1_000_000_000_000_000_000_000_000; // 1 NEAR
const STORAGE_HEADROOM_BYTES: u128 = 4096;

#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PaymentState {
    Pending,
    Paid,
    Failed,
}
#[near(serializers = [borsh, json])]
#[derive(Clone)]
pub struct Payment {
    pub amount: U128,
    pub recipient: AccountId,
    pub state: PaymentState,
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecoveryState {
    Pending,
    Paid,
    Failed,
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Debug)]
pub struct Recovery {
    pub sequence: u64,
    pub amount: U128,
    pub state: RecoveryState,
}

#[near(contract_state)]
#[derive(PanicOnDefault)]
pub struct AgentVault {
    owner: AccountId,
    agent: AccountId,
    recipient: AccountId,
    cap: u128,
    budget: u128,
    deadline_ms: u64,
    revoked: bool,
    available: u128,
    spent: u128,
    reserved: u128,
    used: BTreeSet<String>,
    payments: BTreeMap<String, Payment>,
    // Single bounded recovery record; monotonically increasing counter is
    // never rolled back even after a failed transfer.
    recovery_sequence: u64,
    last_recovery: Option<Recovery>,
}
#[near]
impl AgentVault {
    #[init]
    pub fn new(
        owner: AccountId,
        agent: AccountId,
        recipient: AccountId,
        cap: U128,
        budget: U128,
        deadline_ms: u64,
    ) -> Self {
        assert!(env::current_account_id().as_str().ends_with(".testnet"), "testnet only");
        assert!(
            owner != agent && recipient != agent && owner != recipient,
            "distinct principal roles required"
        );
        assert!(recipient != env::current_account_id(), "vault cannot pay itself");
        assert!(cap.0 > 0 && budget.0 >= cap.0, "invalid cap or budget");
        assert!(deadline_ms > env::block_timestamp_ms(), "expired owner mandate");
        assert_eq!(
            env::predecessor_account_id(),
            env::current_account_id(),
            "vault initializer must be called by deployment account"
        );
        Self {
            owner,
            agent,
            recipient,
            cap: cap.0,
            budget: budget.0,
            deadline_ms,
            revoked: false,
            available: 0,
            spent: 0,
            reserved: 0,
            used: BTreeSet::new(),
            payments: BTreeMap::new(),
            recovery_sequence: 0,
            last_recovery: None,
        }
    }

    /// Record separately funded spendable escrow. The contract's initial gas/storage
    /// reserve is NOT spendable by agents and cannot inflate this ledger.
    #[payable]
    pub fn fund(&mut self) {
        assert_eq!(env::predecessor_account_id(), self.owner, "owner only");
        let amount = env::attached_deposit().as_yoctonear();
        assert!(amount > 0, "zero escrow");
        self.available = self.available.checked_add(amount).expect("escrow overflow");
    }

    /// Agent may only request recipient-approved payments from the vault's own balance.
    /// A NEAR FunctionCall-only key scoped to this method cannot directly transfer
    /// the agent's account balance or call unauthorized contract functions.
    pub fn request_payment(&mut self, nonce: String, recipient: AccountId, amount: U128) -> Promise {
        assert_eq!(env::predecessor_account_id(), self.agent, "unregistered agent");
        assert!(!self.revoked, "owner revoked mandate");
        assert!(env::block_timestamp_ms() < self.deadline_ms, "owner mandate expired");
        assert_eq!(recipient, self.recipient, "recipient not allowlisted");
        assert!(
            !nonce.is_empty()
                && nonce.len() <= 64
                && nonce
                    .bytes()
                    .all(|x| x.is_ascii_alphanumeric() || x == b'_' || x == b'-'),
            "invalid nonce"
        );
        assert!(!self.used.contains(&nonce), "nonce already used");
        assert!(
            self.used.len() < MAX_PAYMENT_RECORDS && self.payments.len() < MAX_PAYMENT_RECORDS,
            "payment history capacity reached"
        );
        assert!(amount.0 > 0 && amount.0 <= self.cap, "per-payment cap exceeded");
        let committed = self.spent.checked_add(self.reserved).expect("spent overflow");
        assert!(
            committed <= self.budget && amount.0 <= self.budget - committed,
            "cumulative budget exceeded"
        );
        assert!(amount.0 <= self.available, "insufficient escrow");
        self.available -= amount.0;
        self.reserved = self.reserved.checked_add(amount.0).expect("reserved overflow");
        self.used.insert(nonce.clone());
        self.payments.insert(
            nonce.clone(),
            Payment {
                amount,
                recipient: recipient.clone(),
                state: PaymentState::Pending,
            },
        );
        Promise::new(recipient)
            .transfer(NearToken::from_yoctonear(amount.0))
            .then(
                Self::ext(env::current_account_id())
                    .with_static_gas(Gas::from_tgas(15))
                    .on_payout(nonce),
            )
    }

    /// State is terminal only when the underlying asynchronous transfer
    /// receipt is available. A failed receipt returns escrow but not nonce.
    #[private]
    pub fn on_payout(&mut self, nonce: String) -> bool {
        let payment = self.payments.get_mut(&nonce).expect("unknown payment");
        assert_eq!(payment.state, PaymentState::Pending, "terminal payout");
        let amount = payment.amount.0;
        self.reserved = self.reserved.checked_sub(amount).expect("reserved underflow");
        if env::promise_result_checked(0, 32).is_ok() {
            self.spent = self.spent.checked_add(amount).expect("spent overflow");
            payment.state = PaymentState::Paid;
            true
        } else {
            self.available = self.available.checked_add(amount).expect("escrow overflow");
            payment.state = PaymentState::Failed;
            false
        }
    }
    /// Reclaim *only* owner-funded available escrow once an agent mandate
    /// cannot accept new payments. The owner never gains access to reserved
    /// in-flight payout funds and cannot drain vault gas/storage reserves.
    /// An asynchronous failure restores available escrow but burns sequence.
    pub fn recover_unused(&mut self, amount: U128) -> Promise {
        assert_eq!(env::predecessor_account_id(), self.owner, "owner only recovery");
        assert!(
            self.revoked || env::block_timestamp_ms() >= self.deadline_ms || self.used.len() >= MAX_PAYMENT_RECORDS,
            "recovery requires revoked, expired, or exhausted mandate"
        );
        assert_eq!(self.reserved, 0, "pending agent payouts must settle first");
        assert!(
            self.last_recovery
                .as_ref()
                .is_none_or(|r| r.state != RecoveryState::Pending),
            "owner recovery already pending"
        );
        assert!(
            amount.0 > 0 && amount.0 <= self.available,
            "recovery amount exceeds available escrow"
        );

        // Account balance and storage usage come from the runtime. The byte
        // cost comes from near-sdk's fixed protocol parameter. Leave a fixed
        // buffer for future state growth and callbacks; this conservative
        // estimate has NOT been audited across all deployment profiles.
        let storage_bytes = u128::from(env::storage_usage())
            .checked_add(STORAGE_HEADROOM_BYTES)
            .expect("storage size overflow");
        let storage_stake = storage_bytes
            .checked_mul(env::storage_byte_cost().as_yoctonear())
            .expect("storage stake overflow");
        let reserve = storage_stake
            .checked_add(MIN_VAULT_GAS_RESERVE)
            .expect("gas/storage reserve overflow");
        let balance = env::account_balance().as_yoctonear();
        assert!(
            amount.0 <= balance.saturating_sub(reserve),
            "insufficient vault balance above gas and storage reserve"
        );
        let next = self
            .recovery_sequence
            .checked_add(1)
            .expect("recovery sequence overflow");

        // Effects before transfer. A duplicate call cannot double debit or
        // double transfer the same available amount.
        self.available -= amount.0;
        self.recovery_sequence = next;
        self.last_recovery = Some(Recovery {
            sequence: next,
            amount,
            state: RecoveryState::Pending,
        });
        Promise::new(self.owner.clone())
            .transfer(NearToken::from_yoctonear(amount.0))
            .then(
                Self::ext(env::current_account_id())
                    .with_static_gas(Gas::from_tgas(15))
                    .on_recovery(next),
            )
    }

    #[private]
    pub fn on_recovery(&mut self, sequence: u64) -> bool {
        assert_eq!(
            env::predecessor_account_id(),
            env::current_account_id(),
            "vault callback only"
        );
        assert_eq!(env::promise_results_count(), 1, "exactly one recovery receipt required");
        let recovery = self.last_recovery.as_mut().expect("recovery missing");
        assert_eq!(recovery.sequence, sequence, "stale recovery callback");
        assert_eq!(recovery.state, RecoveryState::Pending, "recovery already terminal");
        if env::promise_result_checked(0, 32).is_ok() {
            recovery.state = RecoveryState::Paid;
            true
        } else {
            self.available = self
                .available
                .checked_add(recovery.amount.0)
                .expect("failed recovery refund overflow");
            recovery.state = RecoveryState::Failed;
            false
        }
    }

    pub fn latest_recovery(&self) -> Option<Recovery> {
        self.last_recovery.clone()
    }

    pub fn revoke(&mut self) {
        assert_eq!(env::predecessor_account_id(), self.owner, "owner only");
        self.revoked = true;
    }
    pub fn payment(&self, nonce: String) -> Option<Payment> {
        self.payments.get(&nonce).cloned()
    }
    pub fn available(&self) -> U128 {
        U128(self.available)
    }
    pub fn spent(&self) -> U128 {
        U128(self.spent)
    }
    pub fn reserved(&self) -> U128 {
        U128(self.reserved)
    }
    pub fn is_revoked(&self) -> bool {
        self.revoked
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use near_sdk::{test_utils::VMContextBuilder, testing_env};
    const NOW: u64 = 1_700_000_000_000;
    fn set(actor: &str, deposit: u128, t: u64) {
        let mut c = VMContextBuilder::new();
        c.current_account_id("vault.testnet".parse().unwrap())
            .predecessor_account_id(actor.parse().unwrap())
            .attached_deposit(NearToken::from_yoctonear(deposit))
            .account_balance(NearToken::from_near(100))
            .block_timestamp(t * 1_000_000);
        testing_env!(c.build());
    }
    fn vault() -> AgentVault {
        set("vault.testnet", 0, NOW);
        AgentVault::new(
            "owner.testnet".parse().unwrap(),
            "agent.testnet".parse().unwrap(),
            "allowed.testnet".parse().unwrap(),
            U128(2 * 10u128.pow(23)),
            U128(5 * 10u128.pow(23)),
            NOW + 60_000,
        )
    }
    fn fund(v: &mut AgentVault) {
        set("owner.testnet", 10u128.pow(24), NOW);
        v.fund();
    }
    #[test]
    fn compromised_agent_cannot_redirect_payment() {
        let mut v = vault();
        fund(&mut v);
        set("agent.testnet", 0, NOW + 1);
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| v.request_payment(
                "a".into(),
                "attacker.testnet".parse().unwrap(),
                U128(1)
            )))
            .is_err()
        );
        assert_eq!(v.available().0, 10u128.pow(24));
    }
    #[test]
    fn compromised_agent_cannot_exceed_payment_cap() {
        let mut v = vault();
        fund(&mut v);
        set("agent.testnet", 0, NOW + 1);
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| v.request_payment(
                "a".into(),
                "allowed.testnet".parse().unwrap(),
                U128(10u128.pow(24))
            )))
            .is_err()
        );
    }
    #[test]
    fn one_payment_then_nonce_replay_refused() {
        let mut v = vault();
        fund(&mut v);
        set("agent.testnet", 0, NOW + 1);
        let _p = v.request_payment("once".into(), "allowed.testnet".parse().unwrap(), U128(10u128.pow(23)));
        assert_eq!(v.payment("once".into()).unwrap().state, PaymentState::Pending);
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| v.request_payment(
                "once".into(),
                "allowed.testnet".parse().unwrap(),
                U128(10u128.pow(23))
            )))
            .is_err()
        );
    }
    #[test]
    fn revoked_or_expired_cannot_pay() {
        let mut v = vault();
        fund(&mut v);
        set("owner.testnet", 0, NOW + 1);
        v.revoke();
        set("agent.testnet", 0, NOW + 2);
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| v.request_payment(
                "late".into(),
                "allowed.testnet".parse().unwrap(),
                U128(1)
            )))
            .is_err()
        );
        let mut v = vault();
        fund(&mut v);
        set("agent.testnet", 0, NOW + 60_000);
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| v.request_payment(
                "expired".into(),
                "allowed.testnet".parse().unwrap(),
                U128(1)
            )))
            .is_err()
        );
    }
    #[test]
    fn failed_async_transfer_restores_escrow_but_never_reopens_nonce() {
        let mut v = vault();
        fund(&mut v);
        set("agent.testnet", 0, NOW + 1);
        let _p = v.request_payment(
            "failed".into(),
            "allowed.testnet".parse().unwrap(),
            U128(10u128.pow(23)),
        );
        assert_eq!(v.reserved().0, 10u128.pow(23));
        let mut ctx = VMContextBuilder::new();
        ctx.current_account_id("vault.testnet".parse().unwrap())
            .predecessor_account_id("vault.testnet".parse().unwrap())
            .block_timestamp((NOW + 2) * 1_000_000);
        testing_env!(
            ctx.build(),
            near_sdk::test_vm_config(),
            near_sdk::RuntimeFeesConfig::test(),
            std::collections::HashMap::new(),
            vec![near_sdk::PromiseResult::Failed]
        );
        assert!(!v.on_payout("failed".into()));
        assert_eq!(v.available().0, 10u128.pow(24));
        assert_eq!(v.reserved().0, 0);
        assert_eq!(v.payment("failed".into()).unwrap().state, PaymentState::Failed);
        set("agent.testnet", 0, NOW + 3);
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| v.request_payment(
                "failed".into(),
                "allowed.testnet".parse().unwrap(),
                U128(10u128.pow(23))
            )))
            .is_err()
        );
    }
    #[test]
    fn forged_agent_and_owner_operations_fail_closed() {
        let mut v = vault();
        set("agent.testnet", 10u128.pow(24), NOW);
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| v.fund())).is_err());
        assert_eq!(v.available().0, 0);
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| v.revoke())).is_err());
        assert!(!v.is_revoked());
        fund(&mut v);
        set("attacker.testnet", 0, NOW + 1);
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| v.request_payment(
                "forge".into(),
                "allowed.testnet".parse().unwrap(),
                U128(1)
            )))
            .is_err()
        );
        assert_eq!(v.available().0, 10u128.pow(24));
    }
    #[test]
    fn concurrent_pending_payments_cannot_exceed_cumulative_owner_budget() {
        let mut v = vault();
        fund(&mut v);
        set("agent.testnet", 0, NOW + 1);
        let _ = v.request_payment(
            "one".into(),
            "allowed.testnet".parse().unwrap(),
            U128(2 * 10u128.pow(23)),
        );
        let _ = v.request_payment(
            "two".into(),
            "allowed.testnet".parse().unwrap(),
            U128(2 * 10u128.pow(23)),
        );
        assert_eq!(v.reserved().0, 4 * 10u128.pow(23));
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| v.request_payment(
                "three".into(),
                "allowed.testnet".parse().unwrap(),
                U128(2 * 10u128.pow(23))
            )))
            .is_err()
        );
    }
    #[test]
    fn dust_payment_nonce_spam_must_not_grow_vault_storage_without_bound() {
        let mut v = vault();
        fund(&mut v);
        set("agent.testnet", 0, NOW + 1);
        let recipient: AccountId = "allowed.testnet".parse().unwrap();

        // An agent can submit tiny positive payments without consuming any
        // meaningful amount of the owner's spending budget. Each distinct
        // nonce permanently expands both BTreeSet and BTreeMap state.
        for i in 0..64 {
            // Separate actual transactions each receive a fresh gas allotment.
            set("agent.testnet", 0, NOW + 1);
            let nonce = format!("dust_{i}");
            let _promise = v.request_payment(nonce, recipient.clone(), U128(1));
        }
        let available = v.available().0;
        let reserved = v.reserved().0;
        let count = v.payments.len();
        assert_eq!(count, 64);
        assert_eq!(v.used.len(), 64);

        set("agent.testnet", 0, NOW + 1);
        let denied = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            v.request_payment("dust_64".to_string(), recipient, U128(1))
        }));
        assert!(
            denied.is_err(),
            "65th distinct dust payment must not expand storage forever"
        );
        assert_eq!(v.payments.len(), count, "denied payment wrote a record");
        assert_eq!(v.used.len(), count, "denied payment added a nonce");
        assert_eq!(v.available().0, available, "denied payment debited escrow");
        assert_eq!(v.reserved().0, reserved, "denied payment reserved funds");
    }

    fn callback_env(result: near_sdk::PromiseResult) {
        let mut ctx = VMContextBuilder::new();
        ctx.current_account_id("vault.testnet".parse().unwrap())
            .predecessor_account_id("vault.testnet".parse().unwrap())
            .account_balance(NearToken::from_near(100))
            .block_timestamp((NOW + 2) * 1_000_000);
        testing_env!(
            ctx.build(),
            near_sdk::test_vm_config(),
            near_sdk::RuntimeFeesConfig::test(),
            std::collections::HashMap::new(),
            vec![result]
        );
    }

    #[test]
    fn recovery_requires_owner_and_revocation_and_burns_old_receipt_sequence() {
        let mut v = vault();
        fund(&mut v);
        set("agent.testnet", 0, NOW + 1);
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            v.recover_unused(U128(10u128.pow(23)))
        }))
        .is_err());
        set("owner.testnet", 0, NOW + 1);
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            v.recover_unused(U128(10u128.pow(23)))
        }))
        .is_err());
        v.revoke();
        set("attacker.testnet", 0, NOW + 2);
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            v.recover_unused(U128(10u128.pow(23)))
        }))
        .is_err());
        set("owner.testnet", 0, NOW + 2);
        let _promise = v.recover_unused(U128(10u128.pow(23)));
        assert_eq!(v.available().0, 9 * 10u128.pow(23));
        let pending = v.latest_recovery().unwrap();
        assert_eq!(pending.sequence, 1);
        assert_eq!(pending.state, RecoveryState::Pending);
        assert_eq!(pending.amount.0, 10u128.pow(23));
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                v.recover_unused(U128(10u128.pow(23)))
            }))
            .is_err(),
            "duplicate owner recovery while pending"
        );
        callback_env(near_sdk::PromiseResult::Successful(vec![]));
        assert!(v.on_recovery(1));
        assert_eq!(v.latest_recovery().unwrap().state, RecoveryState::Paid);
        assert_eq!(v.available().0, 9 * 10u128.pow(23));
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| { v.on_recovery(1) })).is_err(),
            "duplicate callback cannot release twice"
        );
    }

    #[test]
    fn recovery_blocks_pending_payout_and_callback_failure_restores_escrow() {
        let mut v = vault();
        fund(&mut v);
        set("agent.testnet", 0, NOW + 1);
        let _ = v.request_payment(
            "pending".into(),
            "allowed.testnet".parse().unwrap(),
            U128(10u128.pow(23)),
        );
        set("owner.testnet", 0, NOW + 1);
        v.revoke();
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| { v.recover_unused(U128(1)) })).is_err(),
            "must not recover while payout pending"
        );
        callback_env(near_sdk::PromiseResult::Failed);
        assert!(!v.on_payout("pending".into()));
        set("owner.testnet", 0, NOW + 3);
        let _ = v.recover_unused(U128(10u128.pow(23)));
        assert_eq!(v.available().0, 9 * 10u128.pow(23));
        callback_env(near_sdk::PromiseResult::Failed);
        assert!(!v.on_recovery(1));
        assert_eq!(v.latest_recovery().unwrap().state, RecoveryState::Failed);
        assert_eq!(v.available().0, 10u128.pow(24));
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| { v.on_recovery(1) })).is_err(),
            "failed withdrawal cannot be credited twice"
        );
        set("owner.testnet", 0, NOW + 4);
        let _ = v.recover_unused(U128(10u128.pow(23)));
        assert_eq!(v.latest_recovery().unwrap().sequence, 2);
        callback_env(near_sdk::PromiseResult::Successful(vec![]));
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| { v.on_recovery(1) })).is_err(),
            "stale failed callback cannot finalize newer recovery"
        );
        assert_eq!(v.latest_recovery().unwrap().state, RecoveryState::Pending);
        assert!(v.on_recovery(2));
        assert_eq!(v.latest_recovery().unwrap().state, RecoveryState::Paid);
        assert_eq!(v.available().0, 9 * 10u128.pow(23));
    }

    #[test]
    fn forged_or_stale_recovery_callback_cannot_finalize_transfer() {
        let mut v = vault();
        fund(&mut v);
        set("owner.testnet", 0, NOW + 1);
        v.revoke();
        let _ = v.recover_unused(U128(10u128.pow(23)));
        let available_before = v.available().0;

        let mut ctx = VMContextBuilder::new();
        ctx.current_account_id("vault.testnet".parse().unwrap())
            .predecessor_account_id("attacker.testnet".parse().unwrap())
            .account_balance(NearToken::from_near(100))
            .block_timestamp((NOW + 2) * 1_000_000);
        testing_env!(
            ctx.build(),
            near_sdk::test_vm_config(),
            near_sdk::RuntimeFeesConfig::test(),
            std::collections::HashMap::new(),
            vec![near_sdk::PromiseResult::Successful(vec![])]
        );
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| { v.on_recovery(1) })).is_err(),
            "unauthorized callback must fail"
        );

        callback_env(near_sdk::PromiseResult::Successful(vec![]));
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| { v.on_recovery(2) })).is_err(),
            "wrong sequence cannot finalize recovery"
        );
        assert_eq!(v.available().0, available_before);
        assert_eq!(v.latest_recovery().unwrap().state, RecoveryState::Pending);
        assert!(v.on_recovery(1));
        assert_eq!(v.latest_recovery().unwrap().state, RecoveryState::Paid);
    }

    #[test]
    fn recovery_rejects_insufficient_onchain_storage_and_gas_reserve() {
        let mut v = vault();
        fund(&mut v);
        set("owner.testnet", 0, NOW + 1);
        v.revoke();
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| { v.recover_unused(U128(0)) })).is_err(),
            "zero recovery denied"
        );
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                v.recover_unused(U128(2 * 10u128.pow(24)))
            }))
            .is_err(),
            "recovery above available escrow denied"
        );
        let mut ctx = VMContextBuilder::new();
        ctx.current_account_id("vault.testnet".parse().unwrap())
            .predecessor_account_id("owner.testnet".parse().unwrap())
            .account_balance(NearToken::from_near(1))
            .block_timestamp((NOW + 2) * 1_000_000);
        testing_env!(ctx.build());
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                v.recover_unused(U128(10u128.pow(24)))
            }))
            .is_err(),
            "do not drain gas/storage reserve"
        );
        assert_eq!(v.available().0, 10u128.pow(24));
        assert!(v.latest_recovery().is_none());
    }

    #[test]
    fn owner_can_recover_after_expiry_or_history_exhaustion() {
        let mut v = vault();
        fund(&mut v);
        set("owner.testnet", 0, NOW + 60_000);
        let _ = v.recover_unused(U128(1));
        assert_eq!(v.latest_recovery().unwrap().sequence, 1);
        let mut full = vault();
        fund(&mut full);
        for i in 0..MAX_PAYMENT_RECORDS {
            full.used.insert(format!("spent_{i}"));
        }
        set("owner.testnet", 0, NOW + 1);
        let _ = full.recover_unused(U128(1));
        assert_eq!(full.latest_recovery().unwrap().state, RecoveryState::Pending);
    }

    #[test]
    fn vault_rejects_mainnet_account_at_initialization() {
        let mut ctx = VMContextBuilder::new();
        ctx.current_account_id("vault.near".parse().unwrap())
            .predecessor_account_id("vault.near".parse().unwrap())
            .block_timestamp(NOW * 1_000_000);
        testing_env!(ctx.build());
        assert!(std::panic::catch_unwind(|| AgentVault::new(
            "owner.near".parse().unwrap(),
            "agent.near".parse().unwrap(),
            "recipient.near".parse().unwrap(),
            U128(1),
            U128(10),
            NOW + 60000
        ))
        .is_err());
    }
}
