//! Liquidity/flash-loan ledger invariants: shadow accounting, never a live token adapter.
use x3_lang_vm::liquidity::{LiquidityPool, PoolError};

#[test]
fn lp_shares_mint_proportionally_and_withdraw_only_owned_shares() {
    let mut pool = LiquidityPool::new("near-usdc", "usdc.near").unwrap();
    assert_eq!(pool.deposit("alice.near", 1_000_000).unwrap(), 1_000_000);
    assert_eq!(pool.deposit("bob.near", 500_000).unwrap(), 500_000);
    assert_eq!(pool.total_shares(), 1_500_000);
    assert_eq!(pool.share_balance("bob.near"), 500_000);
    assert_eq!(pool.withdraw("bob.near", 250_000).unwrap(), 250_000);
    assert_eq!(pool.reserves(), 1_250_000);
    assert_eq!(pool.share_balance("bob.near"), 250_000);
    assert_eq!(pool.withdraw("bob.near", 250_001), Err(PoolError::InsufficientShares));
}

#[test]
fn flash_loan_fee_rounds_up_and_is_distributed_to_lp_share_value() {
    let mut pool = LiquidityPool::new("near-usdc", "usdc.near").unwrap();
    pool.deposit("alice.near", 1_000_000).unwrap();
    let ticket = pool.borrow_flash("solver.near", 100_000, 9).unwrap();
    assert_eq!(ticket.principal, 100_000);
    assert_eq!(ticket.fee, 90);
    assert_eq!(ticket.amount_due, 100_090);
    assert_eq!(pool.reserves(), 900_000);
    assert_eq!(
        pool.repay_flash(&ticket, ticket.amount_due - 1),
        Err(PoolError::Underpayment)
    );
    assert_eq!(pool.reserves(), 900_000);
    pool.repay_flash(&ticket, ticket.amount_due).unwrap();
    assert_eq!(pool.reserves(), 1_000_090);
    assert_eq!(pool.withdraw("alice.near", 1_000_000).unwrap(), 1_000_090);
}

#[test]
fn flash_loan_fails_closed_for_insufficient_liquidity_double_borrow_or_lp_mutations() {
    let mut pool = LiquidityPool::new("near-usdc", "usdc.near").unwrap();
    pool.deposit("alice", 100_000).unwrap();
    assert_eq!(
        pool.borrow_flash("attacker", 200_000, 9),
        Err(PoolError::InsufficientLiquidity)
    );
    let ticket = pool.borrow_flash("solver", 100_000, 9).unwrap();
    assert_eq!(pool.borrow_flash("solver", 1, 9), Err(PoolError::FlashOutstanding));
    assert_eq!(pool.deposit("attacker", 10), Err(PoolError::FlashOutstanding));
    assert_eq!(pool.withdraw("alice", 1), Err(PoolError::FlashOutstanding));
    assert!(pool.repay_flash(&ticket, ticket.amount_due).is_ok());
    assert_eq!(
        pool.repay_flash(&ticket, ticket.amount_due),
        Err(PoolError::NoFlashOutstanding)
    );
}

#[test]
fn invalid_ticket_and_bad_repayment_cannot_erase_debt() {
    let mut pool = LiquidityPool::new("near-usdc", "usdc.near").unwrap();
    pool.deposit("alice", 1_000_000).unwrap();
    let ticket = pool.borrow_flash("solver", 100_000, 9).unwrap();
    let mut forged = ticket.clone();
    forged.borrower = "someone-else".to_owned();
    assert_eq!(
        pool.repay_flash(&forged, ticket.amount_due),
        Err(PoolError::TicketMismatch)
    );
    assert_eq!(pool.finish_atomic(), Err(PoolError::FlashOutstanding));
    assert_eq!(
        pool.repay_flash(&ticket, ticket.amount_due + 1),
        Err(PoolError::Overpayment)
    );
    pool.repay_flash(&ticket, ticket.amount_due).unwrap();
    assert_eq!(pool.finish_atomic(), Ok(()));
}

#[test]
fn arithmetic_cannot_overflow_or_mint_zero_shares() {
    let mut pool = LiquidityPool::new("p", "token").unwrap();
    pool.deposit("alice", u128::MAX - 1).unwrap();
    assert_eq!(pool.deposit("bob", 2), Err(PoolError::Overflow));
    assert_eq!(pool.borrow_flash("solver", u128::MAX - 1, 1), Err(PoolError::Overflow));
    assert_eq!(pool.reserves(), u128::MAX - 1);
}

#[test]
fn snapshot_rollback_restores_only_the_local_shadow_ledger() {
    let mut pool = LiquidityPool::new("p", "token").unwrap();
    pool.deposit("alice", 100_000).unwrap();
    let ticket = pool.borrow_flash("solver", 50_000, 10).unwrap();
    assert_eq!(pool.reserves(), 50_000);
    assert_eq!(pool.abort_shadow_flash(&ticket), Ok(()));
    assert_eq!(pool.reserves(), 100_000);
    assert_eq!(pool.finish_atomic(), Ok(()));
    assert_eq!(
        pool.repay_flash(&ticket, ticket.amount_due),
        Err(PoolError::NoFlashOutstanding)
    );
}

#[test]
fn reject_invalid_pool_owner_and_amounts() {
    assert!(matches!(LiquidityPool::new("", "token"), Err(PoolError::InvalidInput)));
    assert!(matches!(LiquidityPool::new("p", ""), Err(PoolError::InvalidInput)));
    let mut pool = LiquidityPool::new("p", "token").unwrap();
    assert_eq!(pool.deposit("alice", 0), Err(PoolError::InvalidInput));
    assert_eq!(pool.deposit("", 100), Err(PoolError::InvalidInput));
    assert_eq!(
        pool.borrow_flash("solver", 100, 9),
        Err(PoolError::InsufficientLiquidity)
    );
    assert_eq!(pool.withdraw("alice", 0), Err(PoolError::InvalidInput));
}
