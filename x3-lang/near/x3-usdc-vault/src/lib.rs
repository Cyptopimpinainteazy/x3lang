//! Testnet NEP-141 custody pilot. The vault enforces its stored payment rules.

#[cfg(all(target_arch = "wasm32", not(near)))]
compile_error!("build the USDC vault with --cfg near");

use near_sdk::json_types::{U128, U64};
use near_sdk::serde_json::json;
use near_sdk::{env, near, require, AccountId, Gas, NearToken, PanicOnDefault, Promise};

const WINDOW_MS: u64 = 86_400_000;
const MAX_WINDOW_PAYMENTS: usize = 128;
const PAYMENT_CAP: u128 = 100_000;
const DAILY_CAP: u128 = 10_000_000;

#[near(serializers = [borsh, json])]
#[derive(Clone)]
pub struct Mandate {
    pub owner: AccountId,
    pub agent: AccountId,
    pub token: AccountId,
    pub merchant: AccountId,
    pub program_sha256: String,
    pub per_payment_cap: U128,
    pub rolling_cap: U128,
    pub total_cap: U128,
    pub expires_at_ms: U64,
}

#[near(serializers = [borsh])]
struct Spend {
    settled_ms: u64,
    amount: u128,
}

#[near(serializers = [borsh, json])]
#[derive(Clone)]
pub struct Pending {
    pub id: U64,
    pub amount: U128,
    pub recipient: AccountId,
    pub withdrawal: bool,
}

#[near(contract_state)]
#[derive(PanicOnDefault)]
pub struct UsdcVault {
    mandate: Mandate,
    available: u128,
    spent: u128,
    next_id: u64,
    pending: Option<Pending>,
    window: Vec<Spend>,
    paused: bool,
    revoked: bool,
}

fn in_window(settled_ms: u64, now_ms: u64) -> bool {
    now_ms.saturating_sub(settled_ms) < WINDOW_MS
}

#[near]
impl UsdcVault {
    #[init]
    pub fn new(mandate: Mandate) -> Self {
        let account = env::current_account_id();
        require!(
            account.as_str().ends_with(".testnet") && env::predecessor_account_id() == account,
            "X3_USDC_INIT: deploy and initialize from the vault testnet account"
        );
        let roles = [
            &account,
            &mandate.owner,
            &mandate.agent,
            &mandate.token,
            &mandate.merchant,
        ];
        for (index, role) in roles.iter().enumerate() {
            require!(
                role.as_str().ends_with(".testnet") && !roles[..index].contains(role),
                "X3_USDC_INIT: distinct testnet roles required"
            );
        }
        require!(
            mandate.program_sha256.len() == 64
                && mandate
                    .program_sha256
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
            "X3_USDC_INIT: invalid program digest"
        );
        require!(
            mandate.per_payment_cap.0 > 0
                && mandate.per_payment_cap.0 <= PAYMENT_CAP
                && mandate.rolling_cap.0 >= mandate.per_payment_cap.0
                && mandate.rolling_cap.0 <= DAILY_CAP
                && mandate.total_cap.0 >= mandate.rolling_cap.0
                && mandate.expires_at_ms.0 > env::block_timestamp_ms(),
            "X3_USDC_INIT: invalid mandate limits"
        );
        Self {
            mandate,
            available: 0,
            spent: 0,
            next_id: 1,
            pending: None,
            window: Vec::new(),
            paused: false,
            revoked: false,
        }
    }

    /// Return any amount the vault does not accept so the token can refund it.
    pub fn ft_on_transfer(&mut self, sender_id: AccountId, amount: U128, msg: String) -> U128 {
        if env::predecessor_account_id() != self.mandate.token
            || sender_id != self.mandate.owner
            || !msg.is_empty()
            || self.paused
            || self.revoked
            || self.pending.is_some()
            || env::block_timestamp_ms() >= self.mandate.expires_at_ms.0
        {
            return amount;
        }
        let Some(available) = self.available.checked_add(amount.0) else {
            return amount;
        };
        if available > self.mandate.total_cap.0.saturating_sub(self.spent) {
            return amount;
        }
        self.available = available;
        U128(0)
    }

    /// The restricted agent key can call this method with zero attached deposit.
    pub fn execute(&mut self, nonce: U64, recipient: AccountId, amount: U128) -> Promise {
        require!(
            env::predecessor_account_id() == self.mandate.agent,
            "X3_USDC_DENIED: agent"
        );
        let now = env::block_timestamp_ms();
        require!(
            !self.paused && !self.revoked && now < self.mandate.expires_at_ms.0,
            "X3_USDC_DENIED: inactive mandate"
        );
        require!(recipient == self.mandate.merchant, "X3_USDC_DENIED: recipient");
        require!(nonce.0 == self.next_id, "X3_USDC_DENIED: nonce");
        require!(
            amount.0 > 0 && amount.0 <= self.mandate.per_payment_cap.0,
            "X3_USDC_DENIED: payment cap"
        );
        let mut count = 0;
        let mut used = 0u128;
        for spend in self.window.iter().filter(|spend| in_window(spend.settled_ms, now)) {
            count += 1;
            used += spend.amount;
        }
        require!(count < MAX_WINDOW_PAYMENTS, "X3_USDC_DENIED: rolling record capacity");
        require!(
            amount.0 <= self.mandate.rolling_cap.0.saturating_sub(used),
            "X3_USDC_DENIED: rolling cap"
        );
        require!(
            amount.0 <= self.mandate.total_cap.0.saturating_sub(self.spent),
            "X3_USDC_DENIED: total cap"
        );
        self.start_transfer(recipient, amount, false)
    }

    pub fn set_paused(&mut self, paused: bool) {
        self.require_owner();
        self.paused = paused;
    }

    pub fn revoke(&mut self) {
        self.require_owner();
        self.revoked = true;
    }

    pub fn withdraw(&mut self, amount: U128) -> Promise {
        self.require_owner();
        require!(
            self.paused || self.revoked || env::block_timestamp_ms() >= self.mandate.expires_at_ms.0,
            "X3_USDC_DENIED: pause before withdrawal"
        );
        self.start_transfer(self.mandate.owner.clone(), amount, true)
    }

    #[private]
    pub fn on_transfer(&mut self, id: U64) -> bool {
        require!(
            env::predecessor_account_id() == env::current_account_id(),
            "X3_USDC_LEDGER: private callback"
        );
        let pending = self.pending.clone().expect("X3_USDC_LEDGER: no pending transfer");
        require!(
            pending.id == id && env::promise_results_count() == 1,
            "X3_USDC_LEDGER: callback mismatch"
        );
        // Oversized successful results must never be mistaken for failed transfers.
        let paid = match env::promise_result_checked(0, 128) {
            Ok(_) => true,
            Err(near_sdk::PromiseError::Failed) => false,
            Err(_) => env::panic_str("X3_USDC_LEDGER: ambiguous transfer result"),
        };
        if !paid {
            self.available = self.available.checked_add(pending.amount.0).expect("escrow overflow");
        } else if !pending.withdrawal {
            let now = env::block_timestamp_ms();
            self.window.retain(|spend| in_window(spend.settled_ms, now));
            self.window.push(Spend {
                settled_ms: now,
                amount: pending.amount.0,
            });
            self.spent += pending.amount.0;
        }
        self.pending = None;
        env::log_str(
            &json!({ "event": "x3_usdc_transfer", "id": id, "paid": paid,
            "recipient": pending.recipient, "amount": pending.amount, "withdrawal": pending.withdrawal })
            .to_string(),
        );
        paid
    }

    pub fn mandate(&self) -> Mandate {
        self.mandate.clone()
    }
    pub fn available(&self) -> U128 {
        U128(self.available)
    }
    pub fn spent(&self) -> U128 {
        U128(self.spent)
    }
    pub fn next_nonce(&self) -> U64 {
        U64(self.next_id)
    }
    pub fn pending(&self) -> Option<Pending> {
        self.pending.clone()
    }

    fn require_owner(&self) {
        require!(
            env::predecessor_account_id() == self.mandate.owner,
            "X3_USDC_DENIED: owner"
        );
    }

    fn start_transfer(&mut self, recipient: AccountId, amount: U128, withdrawal: bool) -> Promise {
        require!(self.pending.is_none(), "X3_USDC_DENIED: transfer pending");
        require!(amount.0 > 0 && amount.0 <= self.available, "X3_USDC_DENIED: escrow");
        let id = U64(self.next_id);
        self.next_id = self.next_id.checked_add(1).expect("X3_USDC_LEDGER: nonce exhausted");
        self.available -= amount.0;
        self.pending = Some(Pending {
            id,
            amount,
            recipient: recipient.clone(),
            withdrawal,
        });
        Promise::new(self.mandate.token.clone())
            .function_call(
                "ft_transfer",
                json!({"receiver_id": recipient, "amount": amount})
                    .to_string()
                    .into_bytes(),
                NearToken::from_yoctonear(1),
                Gas::from_tgas(20),
            )
            .then(
                Self::ext(env::current_account_id())
                    .with_static_gas(Gas::from_tgas(20))
                    .on_transfer(id),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use near_sdk::{test_utils::VMContextBuilder, testing_env, PromiseResult};
    use std::panic::{catch_unwind, AssertUnwindSafe};

    const NOW: u64 = 1_700_000_000_000;

    fn account(value: &str) -> AccountId {
        value.parse().unwrap()
    }

    fn context(actor: &str, now: u64, results: Vec<PromiseResult>) {
        let mut ctx = VMContextBuilder::new();
        ctx.current_account_id(account("vault.testnet"))
            .predecessor_account_id(account(actor))
            .account_balance(NearToken::from_near(100))
            .block_timestamp(now * 1_000_000);
        testing_env!(
            ctx.build(),
            near_sdk::test_vm_config(),
            near_sdk::RuntimeFeesConfig::test(),
            std::collections::HashMap::new(),
            results
        );
    }

    fn vault() -> UsdcVault {
        context("vault.testnet", NOW, vec![]);
        UsdcVault::new(Mandate {
            owner: account("owner.testnet"),
            agent: account("agent.testnet"),
            token: account("token.testnet"),
            merchant: account("merchant.testnet"),
            program_sha256: "a".repeat(64),
            per_payment_cap: U128(100_000),
            rolling_cap: U128(100_000),
            total_cap: U128(300_000),
            expires_at_ms: U64(NOW + 3 * WINDOW_MS),
        })
    }

    fn fund(vault: &mut UsdcVault) {
        context("token.testnet", NOW, vec![]);
        assert_eq!(
            vault
                .ft_on_transfer(account("owner.testnet"), U128(300_000), String::new())
                .0,
            0
        );
    }

    fn pay(vault: &mut UsdcVault, nonce: u64, amount: u128) {
        let _promise = vault.execute(U64(nonce), account("merchant.testnet"), U128(amount));
    }

    fn denied(f: impl FnOnce()) {
        assert!(catch_unwind(AssertUnwindSafe(f)).is_err());
    }

    #[test]
    fn credits_only_the_pinned_token_and_owner() {
        let mut vault = vault();
        context("wrong-token.testnet", NOW, vec![]);
        assert_eq!(
            vault
                .ft_on_transfer(account("owner.testnet"), U128(10), String::new())
                .0,
            10
        );
        context("token.testnet", NOW, vec![]);
        assert_eq!(
            vault
                .ft_on_transfer(account("stranger.testnet"), U128(10), String::new())
                .0,
            10
        );
        assert_eq!(vault.available().0, 0);
        fund(&mut vault);
        assert_eq!(vault.available().0, 300_000);
        assert_eq!(
            vault.ft_on_transfer(account("owner.testnet"), U128(1), String::new()).0,
            1
        );
    }

    #[test]
    fn enforces_caller_recipient_cap_and_pending_reservation() {
        let mut vault = vault();
        fund(&mut vault);
        context("stranger.testnet", NOW, vec![]);
        denied(|| pay(&mut vault, 1, 1));
        context("agent.testnet", NOW, vec![]);
        denied(|| {
            let _promise = vault.execute(U64(1), account("stranger.testnet"), U128(1));
        });
        denied(|| pay(&mut vault, 1, 100_001));
        pay(&mut vault, 1, 100_000);
        denied(|| pay(&mut vault, 2, 1));
        assert_eq!(vault.available().0, 200_000);
        assert_eq!(vault.pending().unwrap().id.0, 1);
        denied(|| {
            vault.on_transfer(U64(1));
        });
    }

    #[test]
    fn restores_only_failed_transfers_and_never_reopens_a_nonce() {
        let mut vault = vault();
        fund(&mut vault);
        context("agent.testnet", NOW, vec![]);
        pay(&mut vault, 1, 100_000);
        context("vault.testnet", NOW, vec![PromiseResult::Successful(vec![0; 129])]);
        denied(|| {
            vault.on_transfer(U64(1));
        });
        assert_eq!(vault.available().0, 200_000);
        context("vault.testnet", NOW, vec![PromiseResult::Failed]);
        denied(|| {
            vault.on_transfer(U64(2));
        });
        assert!(!vault.on_transfer(U64(1)));
        denied(|| {
            vault.on_transfer(U64(1));
        });
        assert_eq!(vault.available().0, 300_000);
        context("agent.testnet", NOW, vec![]);
        denied(|| pay(&mut vault, 1, 1));
        pay(&mut vault, 2, 1);
    }

    #[test]
    fn rolling_window_uses_settlement_time_and_pause_preserves_pending() {
        let mut vault = vault();
        fund(&mut vault);
        context("agent.testnet", NOW, vec![]);
        denied(|| vault.set_paused(true));
        pay(&mut vault, 1, 100_000);
        context("owner.testnet", NOW, vec![]);
        vault.set_paused(true);
        denied(|| {
            let _promise = vault.withdraw(U128(1));
        });
        context("vault.testnet", NOW + 1, vec![PromiseResult::Successful(vec![])]);
        assert!(vault.on_transfer(U64(1)));
        assert_eq!(vault.spent().0, 100_000);
        context("owner.testnet", NOW + 2, vec![]);
        vault.set_paused(false);
        context("agent.testnet", NOW + WINDOW_MS, vec![]);
        denied(|| pay(&mut vault, 2, 1));
        context("agent.testnet", NOW + WINDOW_MS + 1, vec![]);
        pay(&mut vault, 2, 1);
    }

    #[test]
    fn revocation_is_permanent_and_owner_withdrawal_does_not_charge_spend() {
        let mut vault = vault();
        fund(&mut vault);
        context("owner.testnet", NOW, vec![]);
        denied(|| {
            let _promise = vault.withdraw(U128(1));
        });
        vault.revoke();
        vault.set_paused(false);
        context("agent.testnet", NOW, vec![]);
        denied(|| pay(&mut vault, 1, 1));
        context("owner.testnet", NOW, vec![]);
        let _promise = vault.withdraw(U128(50_000));
        assert!(vault.pending().unwrap().withdrawal);
        context("vault.testnet", NOW, vec![PromiseResult::Successful(vec![])]);
        assert!(vault.on_transfer(U64(1)));
        assert_eq!(vault.spent().0, 0);
        assert_eq!(vault.available().0, 250_000);
    }
}
