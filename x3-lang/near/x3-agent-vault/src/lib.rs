//! TESTNET ONLY: a custody-bound contract, never an advisory EOA Guard.
//! A scoped agent key cannot withdraw native NEAR directly from this vault.
//!
//! Ledger invariant: every yoctoNEAR the owner deposits with [`AgentVault::fund`] is
//! always in exactly one bucket: `available + reserved + spent + storage_charged + withdrawn`
//! equals the total funded. The storage a payment record adds to contract state is charged
//! to escrow and to the agent's budget, so a compromised agent cannot use dust payments to
//! lock the owner's escrow in storage stake beyond the budget the owner granted.
//!
//! On-chain WASM must be built with `--cfg near` (this crate's `.cargo/config.toml`, as
//! `cargo near build`); otherwise near-sdk discards every refusal reason and the explorer
//! shows a bare `unreachable` trap.

#[cfg(all(target_arch = "wasm32", not(near)))]
compile_error!("x3-agent-vault: WASM must be built with `--cfg near` (cargo-near, or this crate's .cargo/config.toml)");

use near_sdk::json_types::U128;
use near_sdk::{env, near, require, AccountId, Gas, NearToken, PanicOnDefault, Promise};
use std::collections::{BTreeMap, BTreeSet};

const MAX_NONCE_LEN: usize = 64;
const CALLBACK_GAS: Gas = Gas::from_tgas(15);

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

/// Bytes one payment adds to the borsh-serialized root state: the nonce in `used`
/// (u32 length prefix + bytes), the same key in `payments`, and the `Payment` value
/// (U128 = 16 bytes, AccountId = u32 length prefix + bytes, 1-byte state tag).
pub fn payment_record_bytes(nonce: &str, recipient: &AccountId) -> u64 {
    (2 * (4 + nonce.len()) + 16 + 4 + recipient.as_str().len() + 1) as u64
}

fn deny(reason: &str) -> ! {
    env::panic_str(&format!("X3_VAULT_DENIED: {reason}"))
}

fn checked(value: Option<u128>, what: &str) -> u128 {
    value.unwrap_or_else(|| env::panic_str(&format!("X3_VAULT_LEDGER: {what} overflow")))
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
    storage_charged: u128,
    withdrawn: u128,
    funded: u128,
    used: BTreeSet<String>,
    payments: BTreeMap<String, Payment>,
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
        require!(
            env::current_account_id().as_str().ends_with(".testnet"),
            "X3_VAULT_INIT: testnet only"
        );
        require!(
            owner != agent && recipient != agent && owner != recipient,
            "X3_VAULT_INIT: distinct principal roles required"
        );
        require!(
            recipient != env::current_account_id(),
            "X3_VAULT_INIT: vault cannot pay itself"
        );
        require!(cap.0 > 0 && budget.0 >= cap.0, "X3_VAULT_INIT: invalid cap or budget");
        require!(
            deadline_ms > env::block_timestamp_ms(),
            "X3_VAULT_INIT: expired owner mandate"
        );
        require!(
            env::predecessor_account_id() == env::current_account_id(),
            "X3_VAULT_INIT: vault initializer must be called by deployment account"
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
            storage_charged: 0,
            withdrawn: 0,
            funded: 0,
            used: BTreeSet::new(),
            payments: BTreeMap::new(),
        }
    }

    /// Record separately funded spendable escrow. The contract's initial gas/storage
    /// reserve is NOT spendable by agents and cannot inflate this ledger.
    #[payable]
    pub fn fund(&mut self) {
        if env::predecessor_account_id() != self.owner {
            deny("owner only");
        }
        let amount = env::attached_deposit().as_yoctonear();
        if amount == 0 {
            deny("zero escrow");
        }
        self.available = checked(self.available.checked_add(amount), "escrow");
        self.funded = checked(self.funded.checked_add(amount), "funded");
    }

    /// Agent may only request recipient-approved payments from the vault's own balance.
    /// A NEAR FunctionCall-only key scoped to this method cannot directly transfer
    /// the agent's account balance or call unauthorized contract functions.
    ///
    /// Each payment costs `amount` plus the storage its record adds; both come out of
    /// escrow and count against the owner's cumulative budget.
    pub fn request_payment(&mut self, nonce: String, recipient: AccountId, amount: U128) -> Promise {
        if env::predecessor_account_id() != self.agent {
            deny("unregistered agent");
        }
        if self.revoked {
            deny("owner revoked mandate");
        }
        if env::block_timestamp_ms() >= self.deadline_ms {
            deny("owner mandate expired");
        }
        if recipient != self.recipient {
            deny("recipient not allowlisted");
        }
        if nonce.is_empty()
            || nonce.len() > MAX_NONCE_LEN
            || !nonce
                .bytes()
                .all(|x| x.is_ascii_alphanumeric() || x == b'_' || x == b'-')
        {
            deny("invalid nonce");
        }
        if self.used.contains(&nonce) {
            deny("nonce already used");
        }
        if amount.0 == 0 || amount.0 > self.cap {
            deny("per-payment cap exceeded");
        }
        let storage = checked(
            env::storage_byte_cost()
                .as_yoctonear()
                .checked_mul(payment_record_bytes(&nonce, &recipient) as u128),
            "storage cost",
        );
        let total = checked(amount.0.checked_add(storage), "payment total");
        let committed = checked(
            self.spent
                .checked_add(self.reserved)
                .and_then(|v| v.checked_add(self.storage_charged)),
            "committed",
        );
        if committed > self.budget || total > self.budget - committed {
            deny("cumulative budget exceeded (payment plus its storage)");
        }
        if total > self.available {
            deny("insufficient escrow (payment plus its storage)");
        }
        self.available -= total;
        self.reserved = checked(self.reserved.checked_add(amount.0), "reserved");
        self.storage_charged = checked(self.storage_charged.checked_add(storage), "storage");
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
                    .with_static_gas(CALLBACK_GAS)
                    .on_payout(nonce),
            )
    }

    /// State is terminal only when the underlying asynchronous transfer
    /// receipt is available. A failed receipt returns escrow but not nonce.
    /// The record's storage stays charged: the record itself stays in state.
    #[private]
    pub fn on_payout(&mut self, nonce: String) -> bool {
        let payment = self
            .payments
            .get_mut(&nonce)
            .unwrap_or_else(|| env::panic_str("X3_VAULT_LEDGER: unknown payment"));
        require!(
            payment.state == PaymentState::Pending,
            "X3_VAULT_LEDGER: terminal payout"
        );
        let amount = payment.amount.0;
        self.reserved = self
            .reserved
            .checked_sub(amount)
            .unwrap_or_else(|| env::panic_str("X3_VAULT_LEDGER: reserved underflow"));
        if env::promise_result_checked(0, 32).is_ok() {
            self.spent = checked(self.spent.checked_add(amount), "spent");
            payment.state = PaymentState::Paid;
            true
        } else {
            self.available = checked(self.available.checked_add(amount), "escrow");
            payment.state = PaymentState::Failed;
            false
        }
    }

    /// The owner recovers unspent escrow at any time, including after revocation or
    /// expiry. Funds reserved for pending payouts and storage already charged stay put.
    pub fn withdraw(&mut self, amount: U128) -> Promise {
        if env::predecessor_account_id() != self.owner {
            deny("owner only");
        }
        if amount.0 == 0 || amount.0 > self.available {
            deny("withdrawal exceeds available escrow");
        }
        self.available -= amount.0;
        self.withdrawn = checked(self.withdrawn.checked_add(amount.0), "withdrawn");
        Promise::new(self.owner.clone())
            .transfer(NearToken::from_yoctonear(amount.0))
            .then(
                Self::ext(env::current_account_id())
                    .with_static_gas(CALLBACK_GAS)
                    .on_withdraw(amount),
            )
    }

    /// A failed withdrawal transfer returns its amount to available escrow.
    #[private]
    pub fn on_withdraw(&mut self, amount: U128) -> bool {
        if env::promise_result_checked(0, 32).is_ok() {
            true
        } else {
            self.withdrawn = self
                .withdrawn
                .checked_sub(amount.0)
                .unwrap_or_else(|| env::panic_str("X3_VAULT_LEDGER: withdrawn underflow"));
            self.available = checked(self.available.checked_add(amount.0), "escrow");
            false
        }
    }

    pub fn revoke(&mut self) {
        if env::predecessor_account_id() != self.owner {
            deny("owner only");
        }
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
    pub fn storage_charged(&self) -> U128 {
        U128(self.storage_charged)
    }
    pub fn withdrawn(&self) -> U128 {
        U128(self.withdrawn)
    }
    pub fn funded(&self) -> U128 {
        U128(self.funded)
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
    fn record_cost(nonce: &str) -> u128 {
        env::storage_byte_cost().as_yoctonear()
            * payment_record_bytes(nonce, &"allowed.testnet".parse().unwrap()) as u128
    }
    /// Every funded yoctoNEAR is in exactly one ledger bucket.
    fn assert_ledger(v: &AgentVault) {
        assert_eq!(
            v.available().0 + v.reserved().0 + v.spent().0 + v.storage_charged().0 + v.withdrawn().0,
            v.funded().0
        );
    }
    fn refused<R>(f: impl FnOnce() -> R) -> String {
        let hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let err = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f))
            .err()
            .expect("call was expected to be refused");
        std::panic::set_hook(hook);
        err.downcast_ref::<String>()
            .cloned()
            .or_else(|| err.downcast_ref::<&str>().map(|s| s.to_string()))
            .unwrap_or_default()
    }
    fn callback(result: near_sdk::PromiseResult, t: u64) {
        let mut ctx = VMContextBuilder::new();
        ctx.current_account_id("vault.testnet".parse().unwrap())
            .predecessor_account_id("vault.testnet".parse().unwrap())
            .block_timestamp(t * 1_000_000);
        testing_env!(
            ctx.build(),
            near_sdk::test_vm_config(),
            near_sdk::RuntimeFeesConfig::test(),
            std::collections::HashMap::new(),
            vec![result]
        );
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
        // The amount returns; the storage its record occupies stays charged.
        let storage = record_cost("failed");
        assert_eq!(v.storage_charged().0, storage);
        assert_eq!(v.available().0, 10u128.pow(24) - storage);
        assert_eq!(v.reserved().0, 0);
        assert_ledger(&v);
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

    #[test]
    fn record_bytes_match_borsh_growth_of_root_state() {
        use near_sdk::borsh::to_vec;
        let mut v = vault();
        fund(&mut v);
        let before = to_vec(&v).unwrap().len();
        set("agent.testnet", 0, NOW + 1);
        let nonce = "x".repeat(64);
        let _ = v.request_payment(nonce.clone(), "allowed.testnet".parse().unwrap(), U128(1));
        let after = to_vec(&v).unwrap().len();
        assert_eq!(
            (after - before) as u64,
            payment_record_bytes(&nonce, &"allowed.testnet".parse().unwrap())
        );
    }

    #[test]
    fn dust_payments_cannot_lock_escrow_beyond_budget() {
        let mut v = vault();
        fund(&mut v);
        set("agent.testnet", 0, NOW + 1);
        let mut n = 0u32;
        let reason = loop {
            set("agent.testnet", 0, NOW + 1);
            let nonce = format!("dust{n:06}");
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _ = v.request_payment(nonce.clone(), "allowed.testnet".parse().unwrap(), U128(1));
            }));
            if r.is_err() {
                break refused(|| v.request_payment(nonce, "allowed.testnet".parse().unwrap(), U128(1)));
            }
            n += 1;
            assert!(n < 100_000, "dust payments never hit the budget");
        };
        assert!(
            reason.contains("X3_VAULT_DENIED: cumulative budget exceeded"),
            "{reason}"
        );
        let committed = v.spent().0 + v.reserved().0 + v.storage_charged().0;
        assert!(
            committed <= 5 * 10u128.pow(23),
            "agent committed beyond the owner budget"
        );
        assert!(
            v.storage_charged().0 > v.reserved().0,
            "storage dominated the dust attack"
        );
        assert!(v.available().0 >= 10u128.pow(24) - 5 * 10u128.pow(23));
        assert_ledger(&v);
    }

    #[test]
    fn payment_debits_amount_and_storage_from_escrow() {
        let mut v = vault();
        fund(&mut v);
        set("agent.testnet", 0, NOW + 1);
        let amount = 10u128.pow(23);
        let _ = v.request_payment("p1".into(), "allowed.testnet".parse().unwrap(), U128(amount));
        assert_eq!(v.reserved().0, amount);
        assert_eq!(v.storage_charged().0, record_cost("p1"));
        assert_eq!(v.available().0, 10u128.pow(24) - amount - record_cost("p1"));
        callback(near_sdk::PromiseResult::Successful(vec![]), NOW + 2);
        assert!(v.on_payout("p1".into()));
        assert_eq!(v.spent().0, amount);
        assert_eq!(v.reserved().0, 0);
        assert_eq!(v.payment("p1".into()).unwrap().state, PaymentState::Paid);
        assert!(refused(|| v.on_payout("p1".into())).contains("terminal payout"));
        assert_ledger(&v);
    }

    #[test]
    fn owner_recovers_unspent_escrow_after_revocation() {
        let mut v = vault();
        fund(&mut v);
        set("agent.testnet", 0, NOW + 1);
        let _ = v.request_payment("p".into(), "allowed.testnet".parse().unwrap(), U128(10u128.pow(23)));
        set("owner.testnet", 0, NOW + 2);
        v.revoke();
        let left = v.available().0;
        let _ = v.withdraw(U128(left));
        assert_eq!(v.available().0, 0);
        assert_eq!(v.withdrawn().0, left);
        assert_eq!(v.reserved().0, 10u128.pow(23), "pending payout stays reserved");
        assert_ledger(&v);
    }

    #[test]
    fn withdraw_is_owner_only_and_bounded_by_available() {
        let mut v = vault();
        fund(&mut v);
        for actor in ["agent.testnet", "attacker.testnet", "allowed.testnet", "vault.testnet"] {
            set(actor, 0, NOW + 1);
            assert!(refused(|| v.withdraw(U128(1))).contains("X3_VAULT_DENIED: owner only"));
        }
        set("owner.testnet", 0, NOW + 1);
        assert!(refused(|| v.withdraw(U128(10u128.pow(24) + 1))).contains("exceeds available"));
        assert!(refused(|| v.withdraw(U128(0))).contains("exceeds available"));
        assert_eq!(v.available().0, 10u128.pow(24));
        assert_ledger(&v);
    }

    #[test]
    fn failed_withdrawal_restores_escrow() {
        let mut v = vault();
        fund(&mut v);
        set("owner.testnet", 0, NOW + 1);
        let _ = v.withdraw(U128(4 * 10u128.pow(23)));
        assert_eq!(v.available().0, 6 * 10u128.pow(23));
        callback(near_sdk::PromiseResult::Failed, NOW + 2);
        assert!(!v.on_withdraw(U128(4 * 10u128.pow(23))));
        assert_eq!(v.available().0, 10u128.pow(24));
        assert_eq!(v.withdrawn().0, 0);
        assert_ledger(&v);
    }

    #[test]
    fn refusals_carry_stable_reason_codes() {
        let mut v = vault();
        fund(&mut v);
        set("agent.testnet", 0, NOW + 1);
        let pay = |v: &mut AgentVault, nonce: &str, to: &str, amount: u128| {
            let to: AccountId = to.parse().unwrap();
            let nonce = nonce.to_string();
            refused(move || v.request_payment(nonce, to, U128(amount)))
        };
        assert!(pay(&mut v, "a", "attacker.testnet", 1).contains("X3_VAULT_DENIED: recipient not allowlisted"));
        assert!(pay(&mut v, "a", "allowed.testnet", 3 * 10u128.pow(23))
            .contains("X3_VAULT_DENIED: per-payment cap exceeded"));
        assert!(pay(&mut v, "bad nonce!", "allowed.testnet", 1).contains("X3_VAULT_DENIED: invalid nonce"));
        assert!(pay(&mut v, &"n".repeat(65), "allowed.testnet", 1).contains("X3_VAULT_DENIED: invalid nonce"));
        set("attacker.testnet", 0, NOW + 1);
        assert!(pay(&mut v, "a", "allowed.testnet", 1).contains("X3_VAULT_DENIED: unregistered agent"));
        set("owner.testnet", 0, NOW + 1);
        v.revoke();
        set("agent.testnet", 0, NOW + 2);
        assert!(pay(&mut v, "a", "allowed.testnet", 1).contains("X3_VAULT_DENIED: owner revoked mandate"));
        assert_ledger(&v);
    }
}
