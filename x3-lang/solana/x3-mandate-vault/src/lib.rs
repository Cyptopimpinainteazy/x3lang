//! X3 owner-controlled, agent-executable SPL token vault.
//!
//! Explicit owner approval of the EXACT token destination and quote commitment is required
//! for every one-time transfer. A plain user wallet outside this vault is NOT controlled by
//! these instructions. There is no private key, auto-deployment or solver trust here.
//!
//! Wire format: Borsh enum (variant indexes are part of v1 ABI); policy state is fixed-size
//! Borsh, 268 bytes. All amounts are token base units. Classic SPL Token only.
use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::{
    account_info::{next_account_info, AccountInfo},
    clock::Clock,
    entrypoint,
    entrypoint::ProgramResult,
    log::sol_log_data,
    program::invoke_signed,
    program_error::ProgramError,
    program_pack::Pack,
    pubkey::Pubkey,
    rent::Rent,
    sysvar::Sysvar,
};
use solana_sdk_ids::system_program;
use solana_system_interface::instruction as system_instruction;
use spl_token::{
    instruction::transfer_checked,
    state::{Account as TokenAccount, AccountState, Mint},
};

const PREFIX: &[u8] = b"x3-vault";
const VERSION: u8 = 1;
pub const POLICY_SIZE: usize = 1308;
const DAY_SECONDS: i64 = 86_400;
const MAX_SPENDS: usize = 64;

#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VaultError {
    Unauthorized = 1,
    IncorrectPda = 2,
    InvalidPolicy = 3,
    Expired = 4,
    Revoked = 5,
    PendingApproval = 6,
    MissingApproval = 7,
    Replay = 8,
    LimitExceeded = 9,
    LedgerFull = 13,
    IncorrectTokenAccount = 10,
    IncorrectQuote = 11,
    InvalidTime = 12,
}

impl From<VaultError> for ProgramError {
    fn from(e: VaultError) -> Self {
        ProgramError::Custom(e as u32)
    }
}

/// The owner authorizes exact destination TOKEN ACCOUNTS, not arbitrary wallet addresses.
/// The quote digest is SHA-256 of the canonical signed 1Click quote payload; matching the
/// digest on execution does not constitute cryptographic 1Click verification on-chain.
/// That quote has to be authenticated off-chain *before the owner signs Approve*.
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone, PartialEq, Eq)]
pub enum VaultInstruction {
    Initialize {
        agent: [u8; 32],
        max_per_payment: u64,
        max_per_day: u64,
        expires_at: i64,
    },
    Approve {
        nonce: u64,
        destination_token: [u8; 32],
        destination_owner: [u8; 32],
        quote_digest: [u8; 32],
        amount: u64,
        valid_until: i64,
    },
    Execute {
        nonce: u64,
        quote_digest: [u8; 32],
    },
    Cancel,
    Revoke,
    Withdraw {
        amount: u64,
    },
}

/// One finalized vault transfer in the exact rolling 24-hour ledger.
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Spend {
    pub timestamp: i64,
    pub amount: u64,
}

/// Exactly POLICY_SIZE bytes on chain. No serialized variable-length data.
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone, PartialEq, Eq)]
pub struct Policy {
    pub version: u8,
    pub bump: u8,
    pub owner: [u8; 32],
    pub agent: [u8; 32],
    pub mint: [u8; 32],
    pub vault_token: [u8; 32],
    pub max_per_payment: u64,
    pub max_per_day: u64,
    pub expires_at: i64,
    pub revoked: bool,
    pub last_nonce: u64,
    pub pending: bool,
    pub pending_nonce: u64,
    pub pending_destination: [u8; 32],
    pub pending_destination_owner: [u8; 32],
    pub pending_quote_digest: [u8; 32],
    pub pending_amount: u64,
    pub pending_until: i64,
    pub spends: [Spend; MAX_SPENDS],
}

impl Policy {
    #[expect(
        clippy::too_many_arguments,
        reason = "fixed owner, token, limit and expiry fields are explicit authorization boundaries"
    )]
    fn new(
        bump: u8,
        owner: &Pubkey,
        agent: [u8; 32],
        mint: &Pubkey,
        vault_token: &Pubkey,
        max_per_payment: u64,
        max_per_day: u64,
        expires_at: i64,
        now: i64,
    ) -> Result<Self, ProgramError> {
        if agent == [0; 32] || max_per_payment == 0 || max_per_day < max_per_payment || now >= expires_at {
            return Err(VaultError::InvalidPolicy.into());
        }
        Ok(Self {
            version: VERSION,
            bump,
            owner: owner.to_bytes(),
            agent,
            mint: mint.to_bytes(),
            vault_token: vault_token.to_bytes(),
            max_per_payment,
            max_per_day,
            expires_at,
            revoked: false,
            last_nonce: 0,
            pending: false,
            pending_nonce: 0,
            pending_destination: [0; 32],
            pending_destination_owner: [0; 32],
            pending_quote_digest: [0; 32],
            pending_amount: 0,
            pending_until: 0,
            spends: [Spend::default(); MAX_SPENDS],
        })
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "all owner-signed quote authorization fields are explicit and independently validated"
    )]
    fn approve(
        &mut self,
        now: i64,
        nonce: u64,
        destination: [u8; 32],
        destination_owner: [u8; 32],
        quote_digest: [u8; 32],
        amount: u64,
        valid_until: i64,
    ) -> ProgramResult {
        self.check_active(now)?;
        if self.pending {
            return Err(VaultError::PendingApproval.into());
        }
        if nonce == 0 || nonce <= self.last_nonce {
            return Err(VaultError::Replay.into());
        }
        if destination == [0; 32]
            || destination == self.vault_token
            || destination_owner == [0; 32]
            || quote_digest == [0; 32]
        {
            return Err(VaultError::IncorrectQuote.into());
        }
        if amount == 0 || amount > self.max_per_payment {
            return Err(VaultError::LimitExceeded.into());
        }
        if valid_until <= now || valid_until > self.expires_at {
            return Err(VaultError::InvalidTime.into());
        }
        self.last_nonce = nonce; // Nonces never become reusable, even after cancellation.
        self.pending = true;
        self.pending_nonce = nonce;
        self.pending_destination = destination;
        self.pending_destination_owner = destination_owner;
        self.pending_quote_digest = quote_digest;
        self.pending_amount = amount;
        self.pending_until = valid_until;
        Ok(())
    }

    fn check_active(&self, now: i64) -> ProgramResult {
        if self.revoked {
            return Err(VaultError::Revoked.into());
        }
        if now >= self.expires_at {
            return Err(VaultError::Expired.into());
        }
        if now < 0 {
            return Err(VaultError::InvalidTime.into());
        }
        Ok(())
    }

    /// Find a reusable ledger slot and sum all spends in (now - 86400s, now].
    /// Exactly 64 transfers can remain in the window. On overflow, DENY, never forget.
    fn execution_window(&self, now: i64, nonce: u64, digest: &[u8; 32]) -> Result<(usize, u64), ProgramError> {
        self.check_active(now)?;
        if !self.pending {
            return Err(VaultError::MissingApproval.into());
        }
        if nonce != self.pending_nonce || digest != &self.pending_quote_digest {
            return Err(VaultError::IncorrectQuote.into());
        }
        if now >= self.pending_until {
            return Err(VaultError::Expired.into());
        }
        if self.pending_amount == 0 || self.pending_amount > self.max_per_payment {
            return Err(VaultError::LimitExceeded.into());
        }
        let mut total = 0u64;
        let mut free_slot = None;
        for (slot, spend) in self.spends.iter().enumerate() {
            if spend.amount == 0 {
                free_slot = Some(slot);
                continue;
            }
            let age = now.checked_sub(spend.timestamp).ok_or(VaultError::InvalidTime)?;
            if age < 0 {
                return Err(VaultError::InvalidTime.into());
            }
            if age >= DAY_SECONDS {
                free_slot = Some(slot);
            } else {
                total = total.checked_add(spend.amount).ok_or(VaultError::LimitExceeded)?;
            }
        }
        total = total
            .checked_add(self.pending_amount)
            .ok_or(VaultError::LimitExceeded)?;
        if total > self.max_per_day {
            return Err(VaultError::LimitExceeded.into());
        }
        Ok((free_slot.ok_or(VaultError::LedgerFull)?, total))
    }

    fn finish_execution(&mut self, now: i64, slot: usize) {
        self.spends[slot] = Spend {
            timestamp: now,
            amount: self.pending_amount,
        };
        self.pending = false; // replay-proof one-time approval
        self.pending_amount = 0;
        self.pending_quote_digest = [0; 32];
        self.pending_destination = [0; 32];
        self.pending_destination_owner = [0; 32];
        self.pending_until = 0;
    }

    fn clear_pending(&mut self) {
        self.pending = false;
        self.pending_amount = 0;
        self.pending_destination = [0; 32];
        self.pending_destination_owner = [0; 32];
        self.pending_quote_digest = [0; 32];
        self.pending_until = 0;
    }
}

fn load_policy(program_id: &Pubkey, account: &AccountInfo) -> Result<Policy, ProgramError> {
    if account.owner != program_id || account.data_len() != POLICY_SIZE {
        return Err(VaultError::InvalidPolicy.into());
    }
    let policy = Policy::try_from_slice(&account.try_borrow_data()?).map_err(|_| VaultError::InvalidPolicy)?;
    if policy.version != VERSION {
        return Err(VaultError::InvalidPolicy.into());
    }
    let (expected, bump) = Pubkey::find_program_address(&[PREFIX, &policy.owner, &policy.vault_token], program_id);
    if expected != *account.key || policy.bump != bump {
        return Err(VaultError::IncorrectPda.into());
    }
    Ok(policy)
}

fn save_policy(account: &AccountInfo, policy: &Policy) -> ProgramResult {
    if !account.is_writable {
        return Err(ProgramError::InvalidAccountData);
    }
    let bytes = borsh::to_vec(policy).map_err(|_| VaultError::InvalidPolicy)?;
    if bytes.len() != POLICY_SIZE || account.data_len() != POLICY_SIZE {
        return Err(VaultError::InvalidPolicy.into());
    }
    account.try_borrow_mut_data()?.copy_from_slice(&bytes);
    Ok(())
}

fn require_signer(account: &AccountInfo, expected: &[u8; 32]) -> ProgramResult {
    if !account.is_signer || account.key.as_ref() != expected {
        return Err(VaultError::Unauthorized.into());
    }
    Ok(())
}

fn require_token_program(account: &AccountInfo) -> ProgramResult {
    if *account.key != spl_token::id() || !account.executable {
        return Err(VaultError::IncorrectTokenAccount.into());
    }
    Ok(())
}

fn read_token(account: &AccountInfo, token_program: &AccountInfo) -> Result<TokenAccount, ProgramError> {
    if account.owner != token_program.key {
        return Err(VaultError::IncorrectTokenAccount.into());
    }
    TokenAccount::unpack(&account.try_borrow_data()?).map_err(|_| VaultError::IncorrectTokenAccount.into())
}

fn check_source(source: &TokenAccount, state_key: &Pubkey, mint_key: &Pubkey) -> ProgramResult {
    use solana_program::program_option::COption;
    if source.owner != *state_key
        || source.mint != *mint_key
        || source.state != AccountState::Initialized
        || source.delegate != COption::None
        || source.is_native != COption::None
        || (source.close_authority != COption::None && source.close_authority != COption::Some(*state_key))
    {
        return Err(VaultError::IncorrectTokenAccount.into());
    }
    Ok(())
}

fn checked_accounts<'a>(
    state: &AccountInfo<'a>,
    source: &AccountInfo<'a>,
    destination: &AccountInfo<'a>,
    mint: &AccountInfo<'a>,
    token_program: &AccountInfo<'a>,
    policy: &Policy,
    expected_destination_owner: &[u8; 32],
) -> Result<u8, ProgramError> {
    require_token_program(token_program)?;
    if source.key.as_ref() != policy.vault_token || mint.key.as_ref() != policy.mint || source.key == destination.key {
        return Err(VaultError::IncorrectTokenAccount.into());
    }
    if mint.owner != token_program.key {
        return Err(VaultError::IncorrectTokenAccount.into());
    }
    let mint_data = Mint::unpack(&mint.try_borrow_data()?).map_err(|_| VaultError::IncorrectTokenAccount)?;
    let vault = read_token(source, token_program)?;
    check_source(&vault, state.key, mint.key)?;
    let dest = read_token(destination, token_program)?;
    if dest.mint != *mint.key || dest.state != AccountState::Initialized {
        return Err(VaultError::IncorrectTokenAccount.into());
    }
    if dest.owner.as_ref() != expected_destination_owner {
        return Err(VaultError::Unauthorized.into());
    }
    Ok(mint_data.decimals)
}

#[expect(
    clippy::too_many_arguments,
    reason = "CPI account identities are kept explicit to avoid accidentally substituting accounts"
)]
fn transfer_from_vault<'a>(
    program_id: &Pubkey,
    state: &AccountInfo<'a>,
    source: &AccountInfo<'a>,
    destination: &AccountInfo<'a>,
    mint: &AccountInfo<'a>,
    token_program: &AccountInfo<'a>,
    policy: &Policy,
    amount: u64,
    expected_destination_owner: &[u8; 32],
) -> ProgramResult {
    let decimals = checked_accounts(
        state,
        source,
        destination,
        mint,
        token_program,
        policy,
        expected_destination_owner,
    )?;
    if !source.is_writable || !destination.is_writable || amount == 0 {
        return Err(ProgramError::InvalidAccountData);
    }
    let ix = transfer_checked(
        token_program.key,
        source.key,
        mint.key,
        destination.key,
        state.key,
        &[],
        amount,
        decimals,
    )?;
    let seeds: &[&[u8]] = &[PREFIX, &policy.owner, &policy.vault_token, &[policy.bump]];
    let _ = program_id;
    invoke_signed(
        &ix,
        &[
            source.clone(),
            mint.clone(),
            destination.clone(),
            state.clone(),
            token_program.clone(),
        ],
        &[seeds],
    )
}

#[cfg(not(feature = "no-entrypoint"))]
entrypoint!(process_instruction);

/// Instruction accounts are fixed and documented in README.md in this crate.
pub fn process_instruction(program_id: &Pubkey, accounts: &[AccountInfo], data: &[u8]) -> ProgramResult {
    let instruction = VaultInstruction::try_from_slice(data).map_err(|_| ProgramError::InvalidInstructionData)?;
    match instruction {
        VaultInstruction::Initialize {
            agent,
            max_per_payment,
            max_per_day,
            expires_at,
        } => run_initialize(program_id, accounts, agent, max_per_payment, max_per_day, expires_at),
        VaultInstruction::Approve {
            nonce,
            destination_token,
            destination_owner,
            quote_digest,
            amount,
            valid_until,
        } => run_approve(
            program_id,
            accounts,
            nonce,
            destination_token,
            destination_owner,
            quote_digest,
            amount,
            valid_until,
        ),
        VaultInstruction::Execute { nonce, quote_digest } => run_execute(program_id, accounts, nonce, quote_digest),
        VaultInstruction::Cancel => run_cancel(program_id, accounts),
        VaultInstruction::Revoke => run_revoke(program_id, accounts),
        VaultInstruction::Withdraw { amount } => run_withdraw(program_id, accounts, amount),
    }
}

#[inline(never)]
fn run_initialize(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    agent: [u8; 32],
    max_per_payment: u64,
    max_per_day: u64,
    expires_at: i64,
) -> ProgramResult {
    let a = &mut accounts.iter();
    let owner = next_account_info(a)?;
    let state = next_account_info(a)?;
    let source = next_account_info(a)?;
    let mint = next_account_info(a)?;
    let token_program = next_account_info(a)?;
    let system = next_account_info(a)?;
    if !owner.is_signer || !owner.is_writable || !state.is_writable || *system.key != system_program::id() {
        return Err(VaultError::Unauthorized.into());
    }
    require_token_program(token_program)?;
    if source.owner != token_program.key || mint.owner != token_program.key {
        return Err(VaultError::IncorrectTokenAccount.into());
    }
    Mint::unpack(&mint.try_borrow_data()?).map_err(|_| VaultError::IncorrectTokenAccount)?;
    let (expected, bump) = Pubkey::find_program_address(&[PREFIX, owner.key.as_ref(), source.key.as_ref()], program_id);
    if expected != *state.key || state.lamports() != 0 || state.data_len() != 0 || state.owner != system.key {
        return Err(VaultError::IncorrectPda.into());
    }
    let token = read_token(source, token_program)?;
    check_source(&token, state.key, mint.key)?;
    let now = Clock::get()?.unix_timestamp;
    let policy = Policy::new(
        bump,
        owner.key,
        agent,
        mint.key,
        source.key,
        max_per_payment,
        max_per_day,
        expires_at,
        now,
    )?;
    let rent = Rent::get()?.minimum_balance(POLICY_SIZE);
    invoke_signed(
        &system_instruction::create_account(owner.key, state.key, rent, POLICY_SIZE as u64, program_id),
        &[owner.clone(), state.clone(), system.clone()],
        &[&[PREFIX, owner.key.as_ref(), source.key.as_ref(), &[bump]]],
    )?;
    save_policy(state, &policy)
}

#[inline(never)]
#[expect(
    clippy::too_many_arguments,
    reason = "separately bound owner-approved transfer fields must be explicit"
)]
fn run_approve(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    nonce: u64,
    destination_token: [u8; 32],
    destination_owner: [u8; 32],
    quote_digest: [u8; 32],
    amount: u64,
    valid_until: i64,
) -> ProgramResult {
    let a = &mut accounts.iter();
    let owner = next_account_info(a)?;
    let state = next_account_info(a)?;
    let mut policy = load_policy(program_id, state)?;
    require_signer(owner, &policy.owner)?;
    policy.approve(
        Clock::get()?.unix_timestamp,
        nonce,
        destination_token,
        destination_owner,
        quote_digest,
        amount,
        valid_until,
    )?;
    save_policy(state, &policy)
}

#[inline(never)]
fn run_execute(program_id: &Pubkey, accounts: &[AccountInfo], nonce: u64, quote_digest: [u8; 32]) -> ProgramResult {
    let a = &mut accounts.iter();
    let agent = next_account_info(a)?;
    let state = next_account_info(a)?;
    let source = next_account_info(a)?;
    let destination = next_account_info(a)?;
    let mint = next_account_info(a)?;
    let token_program = next_account_info(a)?;
    let mut policy = load_policy(program_id, state)?;
    require_signer(agent, &policy.agent)?;
    if destination.key.as_ref() != policy.pending_destination {
        return Err(VaultError::IncorrectTokenAccount.into());
    }
    let now = Clock::get()?.unix_timestamp;
    let (slot, _total) = policy.execution_window(now, nonce, &quote_digest)?;
    let amount = policy.pending_amount;
    transfer_from_vault(
        program_id,
        state,
        source,
        destination,
        mint,
        token_program,
        &policy,
        amount,
        &policy.pending_destination_owner,
    )?;
    policy.finish_execution(now, slot);
    save_policy(state, &policy)?;
    // Six event fields identify the owner-approved SPL Token CPI.
    // A verifier must also check this program's invocation/success in the
    // finalized Solana transaction, not merely grep the log text.
    let nonce_bytes = nonce.to_le_bytes();
    let amount_bytes = amount.to_le_bytes();
    sol_log_data(&[
        b"X3_VAULT_EXECUTE_V1",
        state.key.as_ref(),
        &nonce_bytes,
        &quote_digest,
        destination.key.as_ref(),
        &amount_bytes,
    ]);
    Ok(())
}

#[inline(never)]
fn run_cancel(program_id: &Pubkey, accounts: &[AccountInfo]) -> ProgramResult {
    let a = &mut accounts.iter();
    let owner = next_account_info(a)?;
    let state = next_account_info(a)?;
    let mut policy = load_policy(program_id, state)?;
    require_signer(owner, &policy.owner)?;
    policy.clear_pending();
    save_policy(state, &policy)
}

#[inline(never)]
fn run_revoke(program_id: &Pubkey, accounts: &[AccountInfo]) -> ProgramResult {
    let a = &mut accounts.iter();
    let owner = next_account_info(a)?;
    let state = next_account_info(a)?;
    let mut policy = load_policy(program_id, state)?;
    require_signer(owner, &policy.owner)?;
    policy.revoked = true;
    policy.clear_pending();
    save_policy(state, &policy)
}

#[inline(never)]
fn run_withdraw(program_id: &Pubkey, accounts: &[AccountInfo], amount: u64) -> ProgramResult {
    let a = &mut accounts.iter();
    let owner = next_account_info(a)?;
    let state = next_account_info(a)?;
    let source = next_account_info(a)?;
    let destination = next_account_info(a)?;
    let mint = next_account_info(a)?;
    let token_program = next_account_info(a)?;
    let policy = load_policy(program_id, state)?;
    // Owner-only escape hatch: the owner may reclaim their own funds at any time,
    // including after revocation or expiry. Destination must be owned by the owner.
    require_signer(owner, &policy.owner)?;
    transfer_from_vault(
        program_id,
        state,
        source,
        destination,
        mint,
        token_program,
        &policy,
        amount,
        &policy.owner,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy() -> Policy {
        Policy::new(
            253,
            &Pubkey::new_from_array([1; 32]),
            [2; 32],
            &Pubkey::new_from_array([3; 32]),
            &Pubkey::new_from_array([4; 32]),
            100,
            200,
            50_000,
            10,
        )
        .unwrap()
    }

    #[test]
    fn wire_size_is_fixed_and_stable() {
        let p = policy();
        let raw = borsh::to_vec(&p).unwrap();
        assert_eq!(raw.len(), POLICY_SIZE);
        assert_eq!(Policy::try_from_slice(&raw).unwrap(), p);
    }

    #[test]
    fn bad_limits_and_expiry_are_rejected() {
        assert!(Policy::new(
            1,
            &Pubkey::new_unique(),
            [0; 32],
            &Pubkey::new_unique(),
            &Pubkey::new_unique(),
            10,
            100,
            1_000,
            10,
        )
        .is_err());
        assert!(Policy::new(
            1,
            &Pubkey::new_unique(),
            [2; 32],
            &Pubkey::new_unique(),
            &Pubkey::new_unique(),
            100,
            99,
            1_000,
            10,
        )
        .is_err());
        assert!(Policy::new(
            1,
            &Pubkey::new_unique(),
            [2; 32],
            &Pubkey::new_unique(),
            &Pubkey::new_unique(),
            100,
            100,
            10,
            10,
        )
        .is_err());
    }

    #[test]
    fn approvals_are_exact_one_time_and_nonces_monotonic() {
        let mut p = policy();
        assert!(p.approve(100, 1, [5; 32], [9; 32], [6; 32], 50, 1_000).is_ok());
        assert_eq!(
            p.approve(100, 2, [5; 32], [9; 32], [7; 32], 50, 1_000),
            Err(VaultError::PendingApproval.into())
        );
        assert_eq!(
            p.execution_window(100, 2, &[6; 32]),
            Err(VaultError::IncorrectQuote.into())
        );
        assert_eq!(
            p.execution_window(100, 1, &[7; 32]),
            Err(VaultError::IncorrectQuote.into())
        );
        let (slot, total) = p.execution_window(100, 1, &[6; 32]).unwrap();
        assert_eq!(total, 50);
        p.finish_execution(100, slot);
        assert!(!p.pending);
        assert_eq!(p.spends[slot].amount, 50);
        assert_eq!(
            p.execution_window(101, 1, &[6; 32]),
            Err(VaultError::MissingApproval.into())
        );
        assert_eq!(
            p.approve(102, 1, [5; 32], [9; 32], [6; 32], 50, 1_000),
            Err(VaultError::Replay.into())
        );
    }

    #[test]
    fn owner_cancel_never_reuses_a_nonce() {
        let mut p = policy();
        p.approve(100, 1, [5; 32], [9; 32], [6; 32], 50, 1_000).unwrap();
        p.clear_pending();
        assert_eq!(
            p.approve(101, 1, [5; 32], [9; 32], [6; 32], 50, 1_000),
            Err(VaultError::Replay.into())
        );
        assert!(p.approve(101, 2, [5; 32], [9; 32], [7; 32], 50, 1_000).is_ok());
    }

    #[test]
    fn exact_rolling_24h_limit_is_enforced_at_each_execution() {
        let mut p = policy();
        p.approve(100, 1, [5; 32], [9; 32], [6; 32], 100, 1_000).unwrap();
        let (slot, total) = p.execution_window(100, 1, &[6; 32]).unwrap();
        assert_eq!(total, 100);
        p.finish_execution(100, slot);
        p.approve(200, 2, [5; 32], [9; 32], [7; 32], 100, 1_000).unwrap();
        let (slot, total) = p.execution_window(200, 2, &[7; 32]).unwrap();
        assert_eq!(total, 200);
        p.finish_execution(200, slot);
        p.approve(300, 3, [5; 32], [9; 32], [8; 32], 1, 1_000).unwrap();
        assert_eq!(
            p.execution_window(300, 3, &[8; 32]),
            Err(VaultError::LimitExceeded.into())
        );
        // The ledger forgets a payment ONLY once that individual payment is 24h old.
        p.expires_at = 150_000;
        p.clear_pending();
        p.approve(86_601, 4, [5; 32], [9; 32], [8; 32], 100, 100_000).unwrap();
        let (slot, total) = p.execution_window(86_601, 4, &[8; 32]).unwrap();
        assert!(slot < MAX_SPENDS);
        assert_eq!(total, 100);
    }

    #[test]
    fn revocation_and_expiry_block_spending_even_with_valid_quote() {
        let mut p = policy();
        p.approve(100, 1, [5; 32], [9; 32], [6; 32], 30, 1_000).unwrap();
        assert_eq!(p.execution_window(1_000, 1, &[6; 32]), Err(VaultError::Expired.into()));
        p.revoked = true;
        assert_eq!(p.execution_window(101, 1, &[6; 32]), Err(VaultError::Revoked.into()));
        assert_eq!(
            p.approve(101, 2, [5; 32], [9; 32], [6; 32], 20, 1_000),
            Err(VaultError::Revoked.into())
        );
    }

    #[test]
    fn rolling_boundary_cannot_double_spend_at_midnight() {
        let mut p = policy();
        p.expires_at = 300_000;
        p.max_per_day = 100;
        p.approve(86_390, 1, [5; 32], [9; 32], [6; 32], 100, 90_000).unwrap();
        let (slot, _) = p.execution_window(86_390, 1, &[6; 32]).unwrap();
        p.finish_execution(86_390, slot);
        p.approve(86_410, 2, [5; 32], [9; 32], [7; 32], 100, 90_000).unwrap();
        assert_eq!(
            p.execution_window(86_410, 2, &[7; 32]),
            Err(VaultError::LimitExceeded.into()),
        );
        p.clear_pending();
        p.approve(172_800, 3, [5; 32], [9; 32], [8; 32], 100, 180_000).unwrap();
        assert_eq!(p.execution_window(172_800, 3, &[8; 32]).unwrap().1, 100);
    }

    #[test]
    fn ledger_capacity_fails_closed_instead_of_forgetting_payments() {
        let mut p = policy();
        p.expires_at = 300_000;
        for nonce in 1..=MAX_SPENDS {
            let now = 100 + nonce as i64;
            p.approve(now, nonce as u64, [5; 32], [9; 32], [6; 32], 1, 1_000)
                .unwrap();
            let (slot, _) = p.execution_window(now, nonce as u64, &[6; 32]).unwrap();
            p.finish_execution(now, slot);
        }
        p.approve(200, 65, [5; 32], [9; 32], [6; 32], 1, 1_000).unwrap();
        assert_eq!(
            p.execution_window(200, 65, &[6; 32]),
            Err(VaultError::LedgerFull.into())
        );
        p.clear_pending();
        p.approve(86_600, 66, [5; 32], [9; 32], [6; 32], 1, 90_000).unwrap();
        assert_eq!(p.execution_window(86_600, 66, &[6; 32]).unwrap().1, 1);
    }

    #[test]
    fn quote_binding_rejects_mintless_recipient_and_bad_amount() {
        let mut p = policy();
        assert_eq!(
            p.approve(100, 1, [0; 32], [9; 32], [6; 32], 10, 1_000),
            Err(VaultError::IncorrectQuote.into())
        );
        assert_eq!(
            p.approve(100, 1, [5; 32], [9; 32], [0; 32], 10, 1_000),
            Err(VaultError::IncorrectQuote.into())
        );
        assert_eq!(
            p.approve(100, 1, [5; 32], [9; 32], [6; 32], 101, 1_000),
            Err(VaultError::LimitExceeded.into())
        );
        assert_eq!(
            p.approve(100, 1, [5; 32], [9; 32], [6; 32], 1, 50_001),
            Err(VaultError::InvalidTime.into())
        );
    }
}
