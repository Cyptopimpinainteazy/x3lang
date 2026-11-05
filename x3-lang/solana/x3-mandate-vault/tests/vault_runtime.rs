//! Runs the actual X3 vault processor and actual SPL Token processor against Solana's
//! local program-test banks runtime. NO deployment, private keys, RPC, or funded network.
use borsh::BorshDeserialize;
use solana_program::{
    clock::Clock,
    instruction::{AccountMeta, Instruction},
    program_pack::Pack,
    pubkey::Pubkey,
};
use solana_program_test::{processor, ProgramTest, ProgramTestContext};
use solana_sdk::{
    signature::{Keypair, Signer},
    transaction::Transaction,
};
use solana_sdk_ids::system_program;
use solana_system_interface::instruction as system_instruction;
use spl_token::state::{Account as TokenAccount, Mint};
use x3_mandate_vault::{process_instruction, Policy, VaultInstruction};

struct Fixture {
    context: ProgramTestContext,
    program: Pubkey,
    policy: Pubkey,
    source: Keypair,
    destination: Keypair,
    mint: Keypair,
    agent: Keypair,
    recipient: Keypair,
    quote_digest: [u8; 32],
    expiry: i64,
}

fn instruction(program_id: Pubkey, accounts: Vec<AccountMeta>, value: VaultInstruction) -> Instruction {
    Instruction {
        program_id,
        accounts,
        data: borsh::to_vec(&value).unwrap(),
    }
}

impl Fixture {
    fn approve_ix(&self, nonce: u64, amount: u64) -> Instruction {
        instruction(
            self.program,
            vec![
                AccountMeta::new_readonly(self.context.payer.pubkey(), true),
                AccountMeta::new(self.policy, false),
            ],
            VaultInstruction::Approve {
                nonce,
                destination_token: self.destination.pubkey().to_bytes(),
                destination_owner: self.recipient.pubkey().to_bytes(),
                quote_digest: self.quote_digest,
                amount,
                valid_until: self.expiry - 1,
            },
        )
    }

    fn execute_ix(&self, nonce: u64, agent: Pubkey) -> Instruction {
        instruction(
            self.program,
            vec![
                AccountMeta::new_readonly(agent, true),
                AccountMeta::new(self.policy, false),
                AccountMeta::new(self.source.pubkey(), false),
                AccountMeta::new(self.destination.pubkey(), false),
                AccountMeta::new_readonly(self.mint.pubkey(), false),
                AccountMeta::new_readonly(spl_token::id(), false),
            ],
            VaultInstruction::Execute {
                nonce,
                quote_digest: self.quote_digest,
            },
        )
    }

    async fn send_owner(&mut self, ix: Instruction) {
        let tx = Transaction::new_signed_with_payer(
            &[ix],
            Some(&self.context.payer.pubkey()),
            &[&self.context.payer],
            self.context.last_blockhash,
        );
        self.context.banks_client.process_transaction(tx).await.unwrap();
    }

    async fn send_agent(&mut self, ix: Instruction) {
        // The owner is NOT a signer. Even the transaction fee is paid by the agent.
        let tx = Transaction::new_signed_with_payer(
            &[ix],
            Some(&self.agent.pubkey()),
            &[&self.agent],
            self.context.last_blockhash,
        );
        self.context.banks_client.process_transaction(tx).await.unwrap();
    }

    async fn source_balance(&mut self) -> u64 {
        let account = self
            .context
            .banks_client
            .get_account(self.source.pubkey())
            .await
            .unwrap()
            .unwrap();
        TokenAccount::unpack(&account.data).unwrap().amount
    }

    async fn destination_balance(&mut self) -> u64 {
        let account = self
            .context
            .banks_client
            .get_account(self.destination.pubkey())
            .await
            .unwrap()
            .unwrap();
        TokenAccount::unpack(&account.data).unwrap().amount
    }

    async fn policy(&mut self) -> Policy {
        let account = self
            .context
            .banks_client
            .get_account(self.policy)
            .await
            .unwrap()
            .unwrap();
        Policy::try_from_slice(&account.data).unwrap()
    }
}

async fn fixture() -> Fixture {
    let program = Pubkey::new_unique();
    let mut setup = ProgramTest::new("x3_mandate_vault", program, processor!(process_instruction));
    setup.add_program(
        "spl_token",
        spl_token::id(),
        processor!(spl_token::processor::Processor::process),
    );
    let context = setup.start_with_context().await;

    let mint = Keypair::new();
    let source = Keypair::new();
    let destination = Keypair::new();
    let recipient = Keypair::new();
    let agent = Keypair::new();
    let quote_digest = [77; 32];
    let (policy, _) = Pubkey::find_program_address(
        &[b"x3-vault", context.payer.pubkey().as_ref(), source.pubkey().as_ref()],
        &program,
    );
    let rent = context.banks_client.get_rent().await.unwrap();
    let now = context.banks_client.get_sysvar::<Clock>().await.unwrap().unix_timestamp;
    let expiry = now + 200_000;
    let payer = context.payer.pubkey();
    let setup_ixs = vec![
        system_instruction::create_account(
            &payer,
            &mint.pubkey(),
            rent.minimum_balance(Mint::LEN),
            Mint::LEN as u64,
            &spl_token::id(),
        ),
        spl_token::instruction::initialize_mint2(&spl_token::id(), &mint.pubkey(), &payer, None, 6).unwrap(),
        system_instruction::create_account(
            &payer,
            &source.pubkey(),
            rent.minimum_balance(TokenAccount::LEN),
            TokenAccount::LEN as u64,
            &spl_token::id(),
        ),
        spl_token::instruction::initialize_account3(&spl_token::id(), &source.pubkey(), &mint.pubkey(), &policy)
            .unwrap(),
        system_instruction::create_account(
            &payer,
            &destination.pubkey(),
            rent.minimum_balance(TokenAccount::LEN),
            TokenAccount::LEN as u64,
            &spl_token::id(),
        ),
        spl_token::instruction::initialize_account3(
            &spl_token::id(),
            &destination.pubkey(),
            &mint.pubkey(),
            &recipient.pubkey(),
        )
        .unwrap(),
        spl_token::instruction::mint_to(&spl_token::id(), &mint.pubkey(), &source.pubkey(), &payer, &[], 500).unwrap(),
        system_instruction::transfer(&payer, &agent.pubkey(), 1_000_000_000),
    ];
    let tx = Transaction::new_signed_with_payer(
        &setup_ixs,
        Some(&payer),
        &[&context.payer, &mint, &source, &destination],
        context.last_blockhash,
    );
    context.banks_client.process_transaction(tx).await.unwrap();

    let initialize = instruction(
        program,
        vec![
            AccountMeta::new(payer, true),
            AccountMeta::new(policy, false),
            AccountMeta::new_readonly(source.pubkey(), false),
            AccountMeta::new_readonly(mint.pubkey(), false),
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new_readonly(system_program::id(), false),
        ],
        VaultInstruction::Initialize {
            agent: agent.pubkey().to_bytes(),
            max_per_payment: 100,
            max_per_day: 150,
            expires_at: expiry,
        },
    );
    let tx = Transaction::new_signed_with_payer(&[initialize], Some(&payer), &[&context.payer], context.last_blockhash);
    context.banks_client.process_transaction(tx).await.unwrap();

    Fixture {
        context,
        program,
        policy,
        source,
        destination,
        mint,
        agent,
        recipient,
        quote_digest,
        expiry,
    }
}

#[tokio::test]
async fn agent_can_execute_one_owner_approved_transfer_and_replay_fails() {
    let mut f = fixture().await;
    assert_eq!(f.source_balance().await, 500);
    assert_eq!(f.destination_balance().await, 0);
    let approved = f.approve_ix(1, 80);
    f.send_owner(approved).await;
    assert!(f.policy().await.pending);
    let instruction = f.execute_ix(1, f.agent.pubkey());
    f.send_agent(instruction.clone()).await;
    assert_eq!(f.source_balance().await, 420);
    assert_eq!(f.destination_balance().await, 80);
    assert!(!f.policy().await.pending);
    // Use a new blockhash so a duplicate transaction id cannot masquerade as a replay rejection.
    f.context.warp_to_slot(50).unwrap();
    f.context.last_blockhash = f.context.banks_client.get_latest_blockhash().await.unwrap();
    let replay = Transaction::new_signed_with_payer(
        &[instruction],
        Some(&f.agent.pubkey()),
        &[&f.agent],
        f.context.last_blockhash,
    );
    assert!(f.context.banks_client.process_transaction(replay).await.is_err());
    assert_eq!(f.destination_balance().await, 80);
}

#[tokio::test]
async fn owner_revocation_blocks_pending_agent_execution_and_preserves_balance() {
    let mut f = fixture().await;
    let approve = f.approve_ix(1, 90);
    f.send_owner(approve).await;
    let revoke = instruction(
        f.program,
        vec![
            AccountMeta::new_readonly(f.context.payer.pubkey(), true),
            AccountMeta::new(f.policy, false),
        ],
        VaultInstruction::Revoke,
    );
    f.send_owner(revoke).await;
    assert!(f.policy().await.revoked);
    let execute = f.execute_ix(1, f.agent.pubkey());
    let tx = Transaction::new_signed_with_payer(
        &[execute],
        Some(&f.agent.pubkey()),
        &[&f.agent],
        f.context.last_blockhash,
    );
    assert!(f.context.banks_client.process_transaction(tx).await.is_err());
    assert_eq!(f.source_balance().await, 500);
    assert_eq!(f.destination_balance().await, 0);
}

#[tokio::test]
async fn a_payment_signed_by_the_owner_is_not_a_payment_signed_by_the_agent() {
    let mut f = fixture().await;
    f.send_owner(f.approve_ix(1, 75)).await;
    let malicious = f.execute_ix(1, f.context.payer.pubkey());
    let tx = Transaction::new_signed_with_payer(
        &[malicious],
        Some(&f.context.payer.pubkey()),
        &[&f.context.payer],
        f.context.last_blockhash,
    );
    assert!(f.context.banks_client.process_transaction(tx).await.is_err());
    assert_eq!(f.source_balance().await, 500);
    assert_eq!(f.destination_balance().await, 0);
    assert!(f.policy().await.pending);
}

#[tokio::test]
async fn replacing_approved_destination_token_authority_is_rejected() {
    let mut f = fixture().await;
    f.send_owner(f.approve_ix(1, 65)).await;
    let attacker = Keypair::new();
    let change_owner = spl_token::instruction::set_authority(
        &spl_token::id(),
        &f.destination.pubkey(),
        Some(&attacker.pubkey()),
        spl_token::instruction::AuthorityType::AccountOwner,
        &f.recipient.pubkey(),
        &[],
    )
    .unwrap();
    let tx = Transaction::new_signed_with_payer(
        &[change_owner],
        Some(&f.context.payer.pubkey()),
        &[&f.context.payer, &f.recipient],
        f.context.last_blockhash,
    );
    f.context.banks_client.process_transaction(tx).await.unwrap();
    let attempted = f.execute_ix(1, f.agent.pubkey());
    let tx = Transaction::new_signed_with_payer(
        &[attempted],
        Some(&f.agent.pubkey()),
        &[&f.agent],
        f.context.last_blockhash,
    );
    assert!(f.context.banks_client.process_transaction(tx).await.is_err());
    assert_eq!(f.source_balance().await, 500);
    assert_eq!(f.destination_balance().await, 0);
    assert!(f.policy().await.pending);
}

#[tokio::test]
async fn owner_can_recover_remaining_tokens_after_revocation() {
    let mut f = fixture().await;
    let new_authority = f.context.payer.pubkey();
    let ix = spl_token::instruction::set_authority(
        &spl_token::id(),
        &f.destination.pubkey(),
        Some(&new_authority),
        spl_token::instruction::AuthorityType::AccountOwner,
        &f.recipient.pubkey(),
        &[],
    )
    .unwrap();
    let tx = Transaction::new_signed_with_payer(
        &[ix],
        Some(&new_authority),
        &[&f.context.payer, &f.recipient],
        f.context.last_blockhash,
    );
    f.context.banks_client.process_transaction(tx).await.unwrap();
    let revoke = instruction(
        f.program,
        vec![
            AccountMeta::new_readonly(new_authority, true),
            AccountMeta::new(f.policy, false),
        ],
        VaultInstruction::Revoke,
    );
    f.send_owner(revoke).await;
    let recover = instruction(
        f.program,
        vec![
            AccountMeta::new_readonly(new_authority, true),
            AccountMeta::new(f.policy, false),
            AccountMeta::new(f.source.pubkey(), false),
            AccountMeta::new(f.destination.pubkey(), false),
            AccountMeta::new_readonly(f.mint.pubkey(), false),
            AccountMeta::new_readonly(spl_token::id(), false),
        ],
        VaultInstruction::Withdraw { amount: 300 },
    );
    f.send_owner(recover).await;
    assert_eq!(f.source_balance().await, 200);
    assert_eq!(f.destination_balance().await, 300);
    assert!(f.policy().await.revoked);
}
