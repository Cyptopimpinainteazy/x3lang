use near_sdk::{env, near, AccountId, NearToken, PanicOnDefault, Promise};
use near_sdk::json_types::{Base64VecU8, U128};
use std::collections::BTreeMap;

#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SwapState {
    Locked, ClaimPending, ClaimRetry, Claimed,
    RefundPending, RefundRetry, Refunded,
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
}

#[near]
impl NativeNearHtlc {
    #[init]
    pub fn new() -> Self { Self { swaps: BTreeMap::new() } }
    #[payable]
    pub fn lock(&mut self, _id: String, _recipient: AccountId, _hashlock: Base64VecU8, _expires_at_ns: u64) {
        env::panic_str("not implemented")
    }
    pub fn claim(&mut self, _id: String, _preimage: Base64VecU8) -> Promise {
        env::panic_str("not implemented")
    }
    pub fn refund(&mut self, _id: String) -> Promise {
        env::panic_str("not implemented")
    }
    pub fn get(&self, id: String) -> Option<Swap> { self.swaps.get(&id).cloned() }
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
            .block_timestamp(time).build();
        testing_env!(ctx);
    }
    const START: u64 = 1_700_000_000_000_000_000;
    fn secret() -> Vec<u8> { vec![3u8; 32] }
    fn swap_id() -> String { "a".repeat(64) }
    fn lock(c: &mut NativeNearHtlc) {
        set("lp.testnet", 10u128.pow(24), START);
        c.lock(swap_id(), "alice.testnet".parse().unwrap(), Base64VecU8(env::sha256(&secret())), START+600_000_000_000);
    }
    #[test]
    fn locks_native_near_and_records_payout_owner() {
        let mut c = NativeNearHtlc::new();
        lock(&mut c);
        let swap=c.get(swap_id()).unwrap();
        assert_eq!(swap.amount, 10u128.pow(24));
        assert_eq!(swap.recipient.as_str(),"alice.testnet");
        assert_eq!(swap.state,SwapState::Locked);
    }
    #[test]
    fn only_receiver_with_preimage_can_claim() {
        let mut c=NativeNearHtlc::new();
        lock(&mut c);
        set("attacker.testnet",0,START+1);
        let unauthorized=std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| c.claim(swap_id(), Base64VecU8(secret()))));
        assert!(unauthorized.is_err());
        set("alice.testnet",0,START+1);
        let wrong=std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| c.claim(swap_id(),Base64VecU8(vec![4u8;32]))));
        assert!(wrong.is_err());
        c.claim(swap_id(),Base64VecU8(secret()));
        assert_eq!(c.get(swap_id()).unwrap().state,SwapState::ClaimPending);
    }
    #[test]
    fn duplicate_swap_is_rejected() {
        let mut c=NativeNearHtlc::new();
        lock(&mut c);
        let duplicate=std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| lock(&mut c)));
        assert!(duplicate.is_err());
    }
    #[test]
    fn refund_waits_for_deadline_and_only_initiator() {
        let mut c=NativeNearHtlc::new();
        lock(&mut c);
        set("lp.testnet",0,START+1);
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| c.refund(swap_id()))).is_err());
        set("other.testnet",0,START+600_000_000_000);
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| c.refund(swap_id()))).is_err());
        set("lp.testnet",0,START+600_000_000_000);
        c.refund(swap_id());
        assert_eq!(c.get(swap_id()).unwrap().state,SwapState::RefundPending);
    }
}
