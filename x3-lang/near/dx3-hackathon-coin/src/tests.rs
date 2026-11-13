use super::*;
use near_sdk::test_utils::{get_logs, VMContextBuilder};
use near_sdk::testing_env;

const START: u64 = 1_794_000_000 * NS_PER_SEC; // 2026-11-07 UTC
const ONE_DAY: u64 = NS_PER_DAY;
/// The real burn program shipped with the contract; its hash is what deploy binds.
const BURN_X3: &str = include_str!("../x3/dx3hack_burn.x3");
const BURN_X3_SHA256: &str = "220396425eadf9036b33ba15ebe4497687048bd385f523538d750dc0411e0c7e";

fn acc(s: &str) -> AccountId {
    s.parse().unwrap()
}
fn owner() -> AccountId {
    acc("x3lang.near")
}
fn operator() -> AccountId {
    acc("faucet.x3lang.near")
}
fn parent() -> AccountId {
    acc("judges.x3lang.near")
}
fn judge(n: u32) -> AccountId {
    acc(&format!("j{}.judges.x3lang.near", n))
}
fn guard() -> AccountId {
    acc("guard.x3lang.near")
}

fn burn_hash() -> String {
    to_hex(&env::sha256(BURN_X3.as_bytes()))
}

fn ctx(predecessor: &AccountId, ts: u64, deposit: u128) {
    let mut b = VMContextBuilder::new();
    b.current_account_id(acc("dx3hack.x3lang.near"))
        .predecessor_account_id(predecessor.clone())
        .signer_account_id(predecessor.clone())
        .block_timestamp(ts)
        .attached_deposit(NearToken::from_yoctonear(deposit))
        .account_balance(NearToken::from_near(50));
    testing_env!(b.build());
}

fn setup() -> Contract {
    ctx(&owner(), START, 0);
    Contract::new(
        owner(),
        operator(),
        parent(),
        U64(START + 18 * ONE_DAY),
        burn_hash(),
        U128(1_000_000_000_000), // max supply
        U128(100_000_000),       // 100 DX3HACK per claim
        3_600,                   // 1 hour cooldown
        U128(1_000_000_000),     // 1,000 DX3HACK per day
        5,                       // transfers per sender per day
        10,                      // max holders
    )
}

fn with_judges(n: u32) -> Contract {
    let mut c = setup();
    ctx(&operator(), START, 0);
    for i in 0..n {
        c.add_holder(judge(i));
    }
    c
}

#[test]
fn metadata_is_fixed() {
    let c = setup();
    let m = c.ft_metadata();
    assert_eq!(m.name, "Dotx3 HACKATHON ONLY COIN");
    assert_eq!(m.symbol, "DX3HACK");
    assert_eq!(m.decimals, 6);
    assert_eq!(c.hackathon_config().burn_program_sha256, burn_hash());
}

#[test]
#[should_panic(expected = "expires_at must be within 90 days")]
fn rejects_far_future_expiry() {
    ctx(&owner(), START, 0);
    Contract::new(
        owner(),
        operator(),
        parent(),
        U64(START + 91 * ONE_DAY),
        burn_hash(),
        U128(10),
        U128(1),
        1,
        U128(1),
        1,
        1,
    );
}

#[test]
#[should_panic(expected = "burn_program_sha256 must be 64 hex characters")]
fn rejects_bad_burn_hash() {
    ctx(&owner(), START, 0);
    Contract::new(
        owner(),
        operator(),
        parent(),
        U64(START + ONE_DAY),
        "abc".into(),
        U128(10),
        U128(1),
        1,
        U128(1),
        1,
        1,
    );
}

#[test]
fn operator_adds_direct_subaccount() {
    let c = with_judges(1);
    assert!(c.is_holder(judge(0)));
    assert_eq!(c.hackathon_config().holders, 1);
    assert_eq!(c.ft_balance_of(judge(0)).0, 0);
}

#[test]
#[should_panic(expected = "only add direct subaccounts")]
fn operator_cannot_add_outside_parent() {
    let mut c = setup();
    ctx(&operator(), START, 0);
    c.add_holder(acc("v2.ref-finance.near"));
}

#[test]
#[should_panic(expected = "only add direct subaccounts")]
fn operator_cannot_add_nested_subaccount() {
    let mut c = setup();
    ctx(&operator(), START, 0);
    c.add_holder(acc("pool.j1.judges.x3lang.near"));
}

#[test]
#[should_panic(expected = "only add direct subaccounts")]
fn operator_cannot_add_lookalike_parent() {
    let mut c = setup();
    ctx(&operator(), START, 0);
    c.add_holder(acc("evil-judges.x3lang.near"));
}

#[test]
#[should_panic(expected = "Only the owner can do this")]
fn stranger_cannot_add_holder() {
    let mut c = setup();
    ctx(&acc("attacker.near"), START, 0);
    c.add_holder(acc("attacker.near"));
}

#[test]
fn owner_adds_any_contract() {
    let mut c = setup();
    ctx(&owner(), START, 0);
    c.add_holder(guard());
    assert!(c.is_holder(guard()));
}

#[test]
#[should_panic(expected = "Holder limit reached")]
fn holder_limit() {
    with_judges(11);
}

#[test]
fn faucet_claim_mints() {
    let mut c = with_judges(1);
    ctx(&operator(), START, 0);
    assert_eq!(c.faucet_claim(judge(0)).0, 100_000_000);
    assert_eq!(c.ft_balance_of(judge(0)).0, 100_000_000);
    assert_eq!(c.ft_total_supply().0, 100_000_000);
    assert!(get_logs().iter().any(|l| l.contains("ft_mint")));
}

#[test]
#[should_panic(expected = "claimed recently")]
fn faucet_cooldown() {
    let mut c = with_judges(1);
    ctx(&operator(), START, 0);
    c.faucet_claim(judge(0));
    ctx(&operator(), START + 3_599 * NS_PER_SEC, 0);
    c.faucet_claim(judge(0));
}

#[test]
fn faucet_after_cooldown() {
    let mut c = with_judges(1);
    ctx(&operator(), START, 0);
    c.faucet_claim(judge(0));
    ctx(&operator(), START + 3_600 * NS_PER_SEC, 0);
    c.faucet_claim(judge(0));
    assert_eq!(c.ft_balance_of(judge(0)).0, 200_000_000);
}

#[test]
#[should_panic(expected = "daily limit is spent")]
fn faucet_daily_cap() {
    let mut c = with_judges(10);
    ctx(&operator(), START, 0);
    for i in 0..10 {
        c.faucet_claim(judge(i));
    }
    // 10 x 100 = 1,000 spent; cooldown passed but still the same UTC day
    ctx(&operator(), START + 2 * 3_600 * NS_PER_SEC, 0);
    c.faucet_claim(judge(0));
}

#[test]
fn faucet_cap_resets_next_day() {
    let mut c = with_judges(10);
    ctx(&operator(), START, 0);
    for i in 0..10 {
        c.faucet_claim(judge(i));
    }
    ctx(&operator(), START + ONE_DAY, 0);
    c.faucet_claim(judge(0));
    assert_eq!(c.ft_balance_of(judge(0)).0, 200_000_000);
}

#[test]
#[should_panic(expected = "Only the faucet operator can claim")]
fn only_operator_claims() {
    let mut c = with_judges(1);
    ctx(&judge(0), START, 0);
    c.faucet_claim(judge(0));
}

#[test]
#[should_panic(expected = "Receiver isn't an approved DX3HACK holder")]
fn faucet_requires_holder() {
    let mut c = setup();
    ctx(&operator(), START, 0);
    c.faucet_claim(acc("attacker.near"));
}

#[test]
#[should_panic(expected = "Max supply reached")]
fn max_supply() {
    let mut c = setup();
    ctx(&owner(), START, 0);
    c.add_holder(guard());
    c.mint(guard(), U128(1_000_000_000_000));
    c.mint(guard(), U128(1));
}

#[test]
fn transfer_between_holders() {
    let mut c = with_judges(2);
    ctx(&operator(), START, 0);
    c.faucet_claim(judge(0));
    ctx(&judge(0), START, 1);
    c.ft_transfer(judge(1), U128(10_000_000), None);
    assert_eq!(c.ft_balance_of(judge(0)).0, 90_000_000);
    assert_eq!(c.ft_balance_of(judge(1)).0, 10_000_000);
    assert_eq!(c.transfers_left_today(judge(0)), 4);
}

#[test]
#[should_panic(expected = "Receiver isn't an approved DX3HACK holder")]
fn cannot_transfer_to_pool() {
    let mut c = with_judges(1);
    ctx(&operator(), START, 0);
    c.faucet_claim(judge(0));
    ctx(&judge(0), START, 1);
    c.ft_transfer(acc("v2.ref-finance.near"), U128(1), None);
}

#[test]
#[should_panic(expected = "Receiver isn't an approved DX3HACK holder")]
fn cannot_transfer_call_to_pool() {
    let mut c = with_judges(1);
    ctx(&operator(), START, 0);
    c.faucet_claim(judge(0));
    ctx(&judge(0), START, 1);
    let _ = c.ft_transfer_call(acc("v2.ref-finance.near"), U128(1), None, "".into());
}

#[test]
#[should_panic(expected = "Sender isn't an approved DX3HACK holder")]
fn removed_holder_cannot_send() {
    let mut c = with_judges(2);
    ctx(&operator(), START, 0);
    c.faucet_claim(judge(0));
    ctx(&owner(), START, 0);
    c.remove_holder(judge(0));
    ctx(&judge(0), START, 1);
    c.ft_transfer(judge(1), U128(1), None);
}

#[test]
#[should_panic(expected = "Daily transfer limit reached")]
fn transfer_limit_per_day() {
    let mut c = with_judges(2);
    ctx(&operator(), START, 0);
    c.faucet_claim(judge(0));
    ctx(&judge(0), START, 1);
    for _ in 0..6 {
        c.ft_transfer(judge(1), U128(1), None);
    }
}

#[test]
fn transfer_limit_resets_next_day() {
    let mut c = with_judges(2);
    ctx(&operator(), START, 0);
    c.faucet_claim(judge(0));
    ctx(&judge(0), START, 1);
    for _ in 0..5 {
        c.ft_transfer(judge(1), U128(1), None);
    }
    ctx(&judge(0), START + ONE_DAY, 1);
    c.ft_transfer(judge(1), U128(1), None);
    assert_eq!(c.transfers_left_today(judge(0)), 4);
}

#[test]
#[should_panic(expected = "Only approved DX3HACK holders can register")]
fn storage_deposit_requires_holder() {
    let mut c = setup();
    ctx(&acc("attacker.near"), START, 10_000_000_000_000_000_000_000);
    c.storage_deposit(None, None);
}

#[test]
#[should_panic(expected = "Holders are removed by the owner")]
fn no_self_unregister() {
    let mut c = with_judges(1);
    ctx(&judge(0), START, 1);
    c.storage_unregister(Some(true));
}

#[test]
#[should_panic(expected = "paused by the owner")]
fn pause_blocks_transfers() {
    let mut c = with_judges(2);
    ctx(&operator(), START, 0);
    c.faucet_claim(judge(0));
    ctx(&owner(), START, 0);
    c.set_paused(true);
    ctx(&judge(0), START, 1);
    c.ft_transfer(judge(1), U128(1), None);
}

#[test]
fn balances_read_zero_after_expiry() {
    let mut c = with_judges(1);
    ctx(&operator(), START, 0);
    c.faucet_claim(judge(0));
    ctx(&judge(0), START + 18 * ONE_DAY, 0);
    assert_eq!(c.ft_balance_of(judge(0)).0, 0);
    assert_eq!(c.ft_total_supply().0, 0);
    assert!(!c.hackathon_config().active);
}

#[test]
#[should_panic(expected = "The hackathon is over")]
fn no_transfers_after_expiry() {
    let mut c = with_judges(2);
    ctx(&operator(), START, 0);
    c.faucet_claim(judge(0));
    ctx(&judge(0), START + 18 * ONE_DAY, 1);
    c.ft_transfer(judge(1), U128(1), None);
}

#[test]
#[should_panic(expected = "The hackathon is over")]
fn no_claims_after_expiry() {
    let mut c = with_judges(1);
    ctx(&operator(), START + 18 * ONE_DAY, 0);
    c.faucet_claim(judge(0));
}

#[test]
#[should_panic(expected = "The hackathon is over")]
fn no_owner_mint_after_expiry() {
    let mut c = setup();
    ctx(&owner(), START, 0);
    c.add_holder(guard());
    ctx(&owner(), START + 18 * ONE_DAY, 0);
    c.mint(guard(), U128(1));
}

#[test]
fn end_now_only_moves_earlier() {
    let mut c = setup();
    ctx(&owner(), START + ONE_DAY, 0);
    c.end_now();
    assert_eq!(c.hackathon_config().expires_at.0, START + ONE_DAY);
    ctx(&owner(), START + 2 * ONE_DAY, 0);
    c.end_now();
    assert_eq!(c.hackathon_config().expires_at.0, START + ONE_DAY);
}

#[test]
#[should_panic(expected = "hasn't expired yet")]
fn purge_refused_before_expiry() {
    let mut c = with_judges(1);
    ctx(&acc("anyone.near"), START, 0);
    c.purge(vec![judge(0)], BURN_X3.into());
}

#[test]
#[should_panic(expected = "doesn't match the burn program")]
fn purge_requires_bound_x3_program() {
    let mut c = with_judges(1);
    ctx(&acc("anyone.near"), START + 18 * ONE_DAY, 0);
    c.purge(vec![judge(0)], "some other program\n".into());
}

#[test]
fn purge_with_bound_x3_program_burns_everything() {
    let mut c = with_judges(2);
    ctx(&operator(), START, 0);
    c.faucet_claim(judge(0));
    c.faucet_claim(judge(1));
    ctx(&acc("anyone.near"), START + 18 * ONE_DAY, 0);
    let n = c.purge(vec![judge(0), judge(1), acc("never-held.near")], BURN_X3.into());
    assert_eq!(n, 2);
    assert_eq!(c.token.total_supply, 0);
    assert!(!c.token.accounts.contains_key(&judge(0)));
    assert!(!c.is_holder(judge(1)));
    assert_eq!(c.hackathon_config().holders, 0);
    assert!(get_logs().iter().filter(|l| l.contains("ft_burn")).count() == 2);
}

fn check_init_args(raw: &str, suffix: &str) -> near_sdk::serde_json::Value {
    let v: near_sdk::serde_json::Value = near_sdk::serde_json::from_str(raw).unwrap();
    let s = |k: &str| v[k].as_str().unwrap().to_string();
    let n = |k: &str| v[k].as_u64().unwrap();
    assert_eq!(s("burn_program_sha256"), BURN_X3_SHA256);
    // 2026-11-25 23:59:59 UTC, one week after judging ends.
    assert_eq!(s("expires_at"), "1795651199000000000");
    // The .x3 burn program burns exactly the max supply.
    assert!(BURN_X3.contains(&format!("amount {} ", s("max_supply"))));
    for k in ["owner_id", "faucet_operator_id", "holder_parent_id"] {
        assert!(s(k).ends_with(suffix), "{k} must be a {suffix} account");
    }
    ctx(&owner(), START, 0);
    let c = Contract::new(
        s("owner_id").parse().unwrap(),
        s("faucet_operator_id").parse().unwrap(),
        s("holder_parent_id").parse().unwrap(),
        U64(s("expires_at").parse().unwrap()),
        s("burn_program_sha256"),
        U128(s("max_supply").parse().unwrap()),
        U128(s("claim_amount").parse().unwrap()),
        n("claim_cooldown_sec"),
        U128(s("faucet_daily_cap").parse().unwrap()),
        n("max_transfers_per_day") as u32,
        n("max_holders") as u32,
    );
    let cfg = c.hackathon_config();
    assert!(cfg.active);
    assert_eq!(cfg.claim_amount.0, 100_000_000); // 100 DX3HACK
    v
}

#[test]
fn deploy_init_args_are_valid() {
    let main = check_init_args(include_str!("../deploy/init-args.json"), ".near");
    let test = check_init_args(include_str!("../deploy/init-args.testnet.json"), ".testnet");
    // Testnet must rehearse the exact mainnet terms; only account names differ.
    let obj = main.as_object().unwrap();
    for (k, val) in obj {
        if !k.ends_with("_id") {
            assert_eq!(&test[k], val, "testnet {k} differs from mainnet");
        }
    }
    assert_eq!(obj.len(), test.as_object().unwrap().len());
}

#[test]
fn shipped_burn_program_hash_is_pinned() {
    assert_eq!(burn_hash(), BURN_X3_SHA256);
}

#[test]
#[should_panic(expected = "doesn't match the burn program")]
fn purge_refuses_one_byte_change() {
    let mut c = with_judges(1);
    ctx(&acc("anyone.near"), START + 18 * ONE_DAY, 0);
    let tampered = BURN_X3.replacen("judges.x3lang.near", "judges.x3lang.nea", 1);
    c.purge(vec![judge(0)], tampered);
}

#[test]
fn direct_subaccount_helper() {
    assert!(is_direct_subaccount(&judge(7), &parent()));
    assert!(!is_direct_subaccount(&parent(), &parent()));
    assert!(!is_direct_subaccount(&acc("a.b.judges.x3lang.near"), &parent()));
    assert!(!is_direct_subaccount(&acc("xjudges.x3lang.near"), &parent()));
}
