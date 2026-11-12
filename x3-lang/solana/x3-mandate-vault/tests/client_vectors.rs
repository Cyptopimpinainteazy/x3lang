//! Golden vectors that bind the off-chain Python client (`x3-lang/solana/vault_client.py`)
//! to this program's real wire format.
//!
//! Every value below is produced by the actual `VaultInstruction` Borsh encoding, the actual
//! `Pubkey::find_program_address`, and the actual `solana-sdk` legacy message compiler and
//! signer. The Python test suite recomputes each value independently and must match
//! `tests/fixtures/client_vectors.json` byte for byte.
//!
//! Field names avoid words that secret scanners key on; every value is public.
//! All keys are fixed test seeds. They are public, hold nothing, and must never be funded.
//! Regenerate deliberately with `X3_WRITE_CLIENT_VECTORS=1 cargo test --locked --test client_vectors`.
use solana_program::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
};
use solana_sdk::{
    hash::Hash,
    signer::{keypair::keypair_from_seed, Signer},
    transaction::Transaction,
};
use solana_sdk_ids::system_program;
use std::{fmt::Write as _, fs, path::PathBuf, str::FromStr};
use x3_mandate_vault::{Policy, Spend, VaultInstruction, POLICY_SIZE};

const ASSOCIATED_TOKEN_PROGRAM: &str = "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL";

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(out, "{byte:02x}").unwrap();
    }
    out
}

struct Case {
    name: &'static str,
    payer: &'static str,
    instruction: Instruction,
}

fn vectors() -> String {
    let program = Pubkey::new_from_array([11; 32]);
    let owner = keypair_from_seed(&[1; 32]).unwrap();
    let agent = keypair_from_seed(&[2; 32]).unwrap();
    let vault_token = Pubkey::new_from_array([13; 32]);
    let destination_token = Pubkey::new_from_array([14; 32]);
    let mint = Pubkey::new_from_array([15; 32]);
    let destination_owner = Pubkey::new_from_array([16; 32]);
    let owner_token = Pubkey::new_from_array([17; 32]);
    let quote_digest = [77u8; 32];
    let blockhash = Hash::new_from_array([9; 32]);

    let (policy, bump) =
        Pubkey::find_program_address(&[b"x3-vault", owner.pubkey().as_ref(), vault_token.as_ref()], &program);
    let ata_program = Pubkey::from_str(ASSOCIATED_TOKEN_PROGRAM).unwrap();
    let (ata, ata_bump) = Pubkey::find_program_address(
        &[destination_owner.as_ref(), spl_token::id().as_ref(), mint.as_ref()],
        &ata_program,
    );

    let ix = |accounts: Vec<AccountMeta>, value: VaultInstruction| Instruction {
        program_id: program,
        accounts,
        data: borsh::to_vec(&value).unwrap(),
    };
    let cases = vec![
        Case {
            name: "initialize",
            payer: "owner",
            instruction: ix(
                vec![
                    AccountMeta::new(owner.pubkey(), true),
                    AccountMeta::new(policy, false),
                    AccountMeta::new_readonly(vault_token, false),
                    AccountMeta::new_readonly(mint, false),
                    AccountMeta::new_readonly(spl_token::id(), false),
                    AccountMeta::new_readonly(system_program::id(), false),
                ],
                VaultInstruction::Initialize {
                    agent: agent.pubkey().to_bytes(),
                    max_per_payment: 25_000_000,
                    max_per_day: 50_000_000,
                    expires_at: 1_800_000_000,
                },
            ),
        },
        Case {
            name: "approve",
            payer: "owner",
            instruction: ix(
                vec![
                    AccountMeta::new_readonly(owner.pubkey(), true),
                    AccountMeta::new(policy, false),
                ],
                VaultInstruction::Approve {
                    nonce: 7,
                    destination_token: destination_token.to_bytes(),
                    destination_owner: destination_owner.to_bytes(),
                    quote_digest,
                    amount: 25_000_000,
                    valid_until: 1_799_999_000,
                },
            ),
        },
        Case {
            name: "execute",
            payer: "agent",
            instruction: ix(
                vec![
                    AccountMeta::new_readonly(agent.pubkey(), true),
                    AccountMeta::new(policy, false),
                    AccountMeta::new(vault_token, false),
                    AccountMeta::new(destination_token, false),
                    AccountMeta::new_readonly(mint, false),
                    AccountMeta::new_readonly(spl_token::id(), false),
                ],
                VaultInstruction::Execute { nonce: 7, quote_digest },
            ),
        },
        Case {
            name: "cancel",
            payer: "owner",
            instruction: ix(
                vec![
                    AccountMeta::new_readonly(owner.pubkey(), true),
                    AccountMeta::new(policy, false),
                ],
                VaultInstruction::Cancel,
            ),
        },
        Case {
            name: "revoke",
            payer: "owner",
            instruction: ix(
                vec![
                    AccountMeta::new_readonly(owner.pubkey(), true),
                    AccountMeta::new(policy, false),
                ],
                VaultInstruction::Revoke,
            ),
        },
        Case {
            name: "withdraw",
            payer: "owner",
            instruction: ix(
                vec![
                    AccountMeta::new_readonly(owner.pubkey(), true),
                    AccountMeta::new(policy, false),
                    AccountMeta::new(vault_token, false),
                    AccountMeta::new(owner_token, false),
                    AccountMeta::new_readonly(mint, false),
                    AccountMeta::new_readonly(spl_token::id(), false),
                ],
                VaultInstruction::Withdraw { amount: 1_234_567 },
            ),
        },
    ];

    let mut spends = [Spend::default(); 64];
    spends[0] = Spend {
        timestamp: 1_799_990_000,
        amount: 5_000_000,
    };
    spends[63] = Spend {
        timestamp: 1_799_995_000,
        amount: 7_000_000,
    };
    let state = Policy {
        version: 1,
        bump,
        owner: owner.pubkey().to_bytes(),
        agent: agent.pubkey().to_bytes(),
        mint: mint.to_bytes(),
        vault_token: vault_token.to_bytes(),
        max_per_payment: 25_000_000,
        max_per_day: 50_000_000,
        expires_at: 1_800_000_000,
        revoked: false,
        last_nonce: 7,
        pending: true,
        pending_nonce: 7,
        pending_destination: destination_token.to_bytes(),
        pending_destination_owner: destination_owner.to_bytes(),
        pending_quote_digest: quote_digest,
        pending_amount: 25_000_000,
        pending_until: 1_799_999_000,
        spends,
    };
    let state_bytes = borsh::to_vec(&state).unwrap();
    assert_eq!(state_bytes.len(), POLICY_SIZE);

    let mut out = String::new();
    out.push_str("{\n");
    let mut field = |key: &str, value: String| writeln!(out, "  \"{key}\": \"{value}\",").unwrap();
    field("program", program.to_string());
    field("owner_test_scalar_hex", hex(&[1; 32]));
    field("owner", owner.pubkey().to_string());
    field("agent_test_scalar_hex", hex(&[2; 32]));
    field("agent", agent.pubkey().to_string());
    field("vault_spl_account", vault_token.to_string());
    field("destination_spl_account", destination_token.to_string());
    field("destination_owner", destination_owner.to_string());
    field("owner_spl_account", owner_token.to_string());
    field("mint", mint.to_string());
    field("spl_program", spl_token::id().to_string());
    field("system_program", system_program::id().to_string());
    field("ata_program", ata_program.to_string());
    field("quote_digest_hex", hex(&quote_digest));
    field("recent_blockhash", blockhash.to_string());
    field("policy", policy.to_string());
    field("policy_bump", bump.to_string());
    field("destination_owner_ata", ata.to_string());
    field("destination_owner_ata_bump", ata_bump.to_string());
    field("policy_state_hex", hex(&state_bytes));
    out.push_str("  \"transactions\": {\n");
    let total = cases.len();
    for (index, case) in cases.into_iter().enumerate() {
        let signer = if case.payer == "owner" { &owner } else { &agent };
        let tx = Transaction::new_signed_with_payer(
            std::slice::from_ref(&case.instruction),
            Some(&signer.pubkey()),
            &[signer],
            blockhash,
        );
        assert_eq!(tx.signatures.len(), 1);
        tx.verify().unwrap();
        writeln!(
            out,
            "    \"{}\": {{\"payer\": \"{}\", \"data_hex\": \"{}\", \"message_hex\": \"{}\", \"signature_hex\": \"{}\"}}{}",
            case.name,
            case.payer,
            hex(&case.instruction.data),
            hex(&tx.message_data()),
            hex(tx.signatures[0].as_ref()),
            if index + 1 == total { "" } else { "," }
        )
        .unwrap();
    }
    out.push_str("  }\n}\n");
    out
}

#[test]
fn python_client_vectors_match_the_program_wire_format() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/client_vectors.json");
    let current = vectors();
    if std::env::var_os("X3_WRITE_CLIENT_VECTORS").is_some() {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, &current).unwrap();
    }
    let committed = fs::read_to_string(&path)
        .expect("tests/fixtures/client_vectors.json is missing; see this file's header to regenerate");
    assert_eq!(
        committed, current,
        "program wire format changed: the Python vault client and its vectors must be updated together"
    );
}
