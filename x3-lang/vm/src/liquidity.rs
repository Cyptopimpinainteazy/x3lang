//! Deterministic LP share and single-asset flash-loan accounting.
//!
//! This is a **shadow ledger**: it moves no NEP-141/SPL/ERC-20 tokens and is
//! not a production venue adapter. On-chain calls must be integrated through
//! the capability-controlled TradingHost, with independently verified reserve
//! and repayment state. Shadow rollback does NOT reverse external transfers.

use std::collections::BTreeMap;

/// Exact errors prevent UI clients from mislabeling a denied operation as a
/// completed deposit, withdrawal, borrow or repayment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PoolError {
    InvalidInput,
    Overflow,
    InsufficientLiquidity,
    InsufficientShares,
    SharesRoundToZero,
    FlashOutstanding,
    NoFlashOutstanding,
    TicketMismatch,
    Underpayment,
    Overpayment,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FlashTicket {
    pub pool_id: String,
    pub borrower: String,
    pub nonce: u64,
    pub principal: u128,
    pub fee: u128,
    pub amount_due: u128,
}

/// Single-asset deterministic liquidity accounting. Do not use this to
/// represent chain funds until a real token custody adapter is reconciled.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiquidityPool {
    pool_id: String,
    asset_id: String,
    reserve_balance: u128,
    total_shares: u128,
    shares: BTreeMap<String, u128>,
    flash: Option<FlashTicket>,
    next_nonce: u64,
}

impl LiquidityPool {
    pub fn new(pool_id: &str, asset_id: &str) -> Result<Self, PoolError> {
        if pool_id.trim().is_empty() || asset_id.trim().is_empty() {
            return Err(PoolError::InvalidInput);
        }
        Ok(Self {
            pool_id: pool_id.to_owned(),
            asset_id: asset_id.to_owned(),
            reserve_balance: 0,
            total_shares: 0,
            shares: BTreeMap::new(),
            flash: None,
            next_nonce: 0,
        })
    }

    pub fn pool_id(&self) -> &str {
        &self.pool_id
    }
    pub fn asset_id(&self) -> &str {
        &self.asset_id
    }
    pub fn reserves(&self) -> u128 {
        self.reserve_balance
    }
    pub fn total_shares(&self) -> u128 {
        self.total_shares
    }
    pub fn share_balance(&self, owner: &str) -> u128 {
        *self.shares.get(owner).unwrap_or(&0)
    }

    fn check_idle(&self) -> Result<(), PoolError> {
        if self.flash.is_some() {
            Err(PoolError::FlashOutstanding)
        } else {
            Ok(())
        }
    }

    /// Mints proportional LP shares, rounded down. Zero-share dust is refused.
    /// State is mutated only after every calculation and validation succeeds.
    pub fn deposit(&mut self, owner: &str, amount: u128) -> Result<u128, PoolError> {
        self.check_idle()?;
        if owner.trim().is_empty() || amount == 0 {
            return Err(PoolError::InvalidInput);
        }
        let issued = if self.total_shares == 0 {
            amount
        } else {
            if self.reserve_balance == 0 {
                return Err(PoolError::InvalidInput);
            }
            amount.checked_mul(self.total_shares).ok_or(PoolError::Overflow)? / self.reserve_balance
        };
        if issued == 0 {
            return Err(PoolError::SharesRoundToZero);
        }
        let reserve = self.reserve_balance.checked_add(amount).ok_or(PoolError::Overflow)?;
        let shares = self.total_shares.checked_add(issued).ok_or(PoolError::Overflow)?;
        let owner_shares = self
            .share_balance(owner)
            .checked_add(issued)
            .ok_or(PoolError::Overflow)?;
        self.reserve_balance = reserve;
        self.total_shares = shares;
        self.shares.insert(owner.to_owned(), owner_shares);
        Ok(issued)
    }

    /// Burns owned shares. Last LP receives all remaining reserve dust.
    pub fn withdraw(&mut self, owner: &str, shares: u128) -> Result<u128, PoolError> {
        self.check_idle()?;
        if owner.trim().is_empty() || shares == 0 {
            return Err(PoolError::InvalidInput);
        }
        if shares > self.share_balance(owner) {
            return Err(PoolError::InsufficientShares);
        }
        let output = if shares == self.total_shares {
            self.reserve_balance
        } else {
            shares.checked_mul(self.reserve_balance).ok_or(PoolError::Overflow)? / self.total_shares
        };
        if output == 0 {
            return Err(PoolError::SharesRoundToZero);
        }
        self.reserve_balance -= output;
        self.total_shares -= shares;
        let remaining = self.share_balance(owner) - shares;
        if remaining == 0 {
            self.shares.remove(owner);
        } else {
            self.shares.insert(owner.to_owned(), remaining);
        }
        Ok(output)
    }

    /// Issue a shadow flash-loan ticket. The fee rounds **up**, never down.
    pub fn borrow_flash(&mut self, borrower: &str, principal: u128, fee_bps: u16) -> Result<FlashTicket, PoolError> {
        self.check_idle()?;
        if borrower.trim().is_empty() || principal == 0 || fee_bps > 10_000 {
            return Err(PoolError::InvalidInput);
        }
        if principal > self.reserve_balance {
            return Err(PoolError::InsufficientLiquidity);
        }
        let fee = principal
            .checked_mul(u128::from(fee_bps))
            .ok_or(PoolError::Overflow)?
            .checked_add(9_999)
            .ok_or(PoolError::Overflow)?
            / 10_000;
        let amount_due = principal.checked_add(fee).ok_or(PoolError::Overflow)?;
        let next = self.next_nonce.checked_add(1).ok_or(PoolError::Overflow)?;
        let ticket = FlashTicket {
            pool_id: self.pool_id.clone(),
            borrower: borrower.to_owned(),
            nonce: next,
            principal,
            fee,
            amount_due,
        };
        self.reserve_balance -= principal;
        self.next_nonce = next;
        self.flash = Some(ticket.clone());
        Ok(ticket)
    }

    /// Require exact principal + rounded-up fee before the loan is closed.
    /// The received fee increases LP share value but never mints new shares.
    pub fn repay_flash(&mut self, ticket: &FlashTicket, payment: u128) -> Result<(), PoolError> {
        let live = self.flash.as_ref().ok_or(PoolError::NoFlashOutstanding)?;
        if live != ticket {
            return Err(PoolError::TicketMismatch);
        }
        if payment < live.amount_due {
            return Err(PoolError::Underpayment);
        }
        if payment > live.amount_due {
            return Err(PoolError::Overpayment);
        }
        let reserve = self.reserve_balance.checked_add(payment).ok_or(PoolError::Overflow)?;
        self.reserve_balance = reserve;
        self.flash = None;
        Ok(())
    }

    /// Only reverses this local shadow ledger, **never an external token transfer**.
    pub fn abort_shadow_flash(&mut self, ticket: &FlashTicket) -> Result<(), PoolError> {
        let live = self.flash.as_ref().ok_or(PoolError::NoFlashOutstanding)?;
        if live != ticket {
            return Err(PoolError::TicketMismatch);
        }
        let reserve = self
            .reserve_balance
            .checked_add(live.principal)
            .ok_or(PoolError::Overflow)?;
        self.reserve_balance = reserve;
        self.flash = None;
        Ok(())
    }

    /// An unsettled flash obligation must never be labeled committed.
    pub fn finish_atomic(&self) -> Result<(), PoolError> {
        self.check_idle()
    }
}
