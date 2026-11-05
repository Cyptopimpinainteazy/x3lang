# X3 Solana Mandate Vault (experimental)

**Owner-controlled SPL tokens. Exact quote approvals. Agent-executable transfers.**

This is a *new, standalone Solana program*, not a wallet extension or an off-chain allowance service. Funds governed by the vault must first reside in a classic SPL Token account whose **token authority is the X3 vault's program-derived policy address (PDA)**. A normal external wallet remains unrestricted outside that account.

> **Status:** source and local testing only. **No deployment, external audit, funded use, or verified cross-chain settlement is claimed.** Never send funds to the example keys, and do not use this program on mainnet without an independent security review and full SBF/runtime tests.

## Enforced trust boundary

- **Policy owner** signs initialization, exact quote approval, cancellation, revocation, and owner-only withdrawal.
- **Agent** may execute a previously approved payment, but does not need the owner's transaction signature at execution time.
- **Vault PDA** is the only token-account spending authority; `TransferChecked` is invoked with `invoke_signed` and the correct PDA seeds.
- **Each owner approval** binds the next monotonic nonce, exact destination **token account**, that account's authority, exact token amount, 32-byte quote digest, and expiration.
- **Agent execution** checks identity, that exact approval, nonce, matching account authority and mint, expiry, revocation, maximum per-payment amount, and a strict rolling 24-hour cap.
- **Rolling ledger:** 64 fixed on-chain spend slots. After 64 transfers in any 24 hours, subsequent transfers **fail closed** rather than discard recent spend history. Each old entry expires individually after 86,400 seconds.
- **Owner-only withdrawal** can recover remaining funds to a token account owned by the owner, including after revocation or expiration.
- **Classic SPL Token only.** Token-2022, delegated source accounts, wrapped SOL, arbitrary hooks, and multisig authorities are outside this first implementation.

## Flow (no auto-spending by X3Lang)

```text
Owner deposits USDC into SPL token account owned by X3 vault PDA
                                 |
 Owner sets policy: agent, mint, vault source, payment/day caps, expiry
                                 |
            X3Lang gets a 1Click signed live quote
                                 |
Off-chain builder independently verifies the 1Click quote signature
                                 |
Owner signs Approve for exact destination token account, its authority,
         amount, digest SHA256(signed quote payload), nonce, expiry
                                 |
  Agent signs Execute; Solana program checks policy + actual SPL state
                                 |
        SPL Token TransferChecked CPI signed by vault PDA
                                 |
   Program data event emitted ONLY after successful CPI/state update
                                 |
Independent Solana finalized transaction and NEAR settlement evidence
              required before claiming cross-chain completion
```

**Critical distinction:** the program does **not** verify the 1Click quote's Ed25519 manager signature on chain. The owner signs the exact quote digest, token account and amount *after* authenticating them off-chain. This is an owner-approved single-quote settlement path, not unrestricted autonomous agent spending. X3Lang's unrelated external wallets are **not** protected by this vault.

## Program account order and ABI

All instruction data is Borsh-serialized `VaultInstruction` from `src/lib.rs`. Variant indexes are part of the v1 wire ABI.

| Instruction | Ordered accounts | Signer |
| --- | --- | --- |
| `Initialize` | owner, policy PDA, vault SPL token account, mint, classic SPL Token program, System program | owner |
| `Approve` | owner, policy PDA | owner |
| `Execute` | agent, policy PDA, vault SPL token account, approved destination SPL token account, mint, classic SPL Token program | agent |
| `Cancel` | owner, policy PDA | owner |
| `Revoke` | owner, policy PDA | owner |
| `Withdraw` | owner, policy PDA, vault SPL token account, owner-owned destination SPL token account, mint, classic SPL Token program | owner |

Policy PDA seeds: `["x3-vault", owner_pubkey_bytes, vault_token_account_pubkey_bytes]` under the actual deployed vault program ID. The policy is a fixed-size Borsh record. No program ID or wallet keys are hard-coded in the source.

The owner must pre-create an initialized classic SPL token account with the derived policy PDA as its SPL owner/authority (it must have **no delegate**). Creating the policy does not itself mint, fund, or transfer tokens. Do not confuse a Solana deposit **wallet** address with its associated **token account** address: the quote owner must approve the token account and its authority after verifying both.

### Event format for Solana-side evidence

A successful execution emits `sol_log_data` with seven binary fields, in order:

1. Literal UTF-8 `X3_VAULT_EXECUTE_V1`
2. Policy PDA public key (32 bytes)
3. Owner-approved nonce (little-endian u64)
4. SHA-256 quote digest (32 bytes)
5. Exact destination token account public key (32 bytes)
6. Approved destination **token-account owner** (32 bytes); this must match the 1Click-signed deposit address
7. Transferred amount in token base units (little-endian u64)

The standalone [read-only Solana source verifier](../vault_evidence.py) checks the
finalized transaction from at least two distinct Solana RPC endpoints, decodes the
seven binary event fields **within the expected vault-program invocation frame**,
verifies the exact classic SPL Token `TransferChecked` CPI grouped under that
instruction, validates the cryptographically signed live 1Click quote, and binds
the approved destination-token owner to the quote's signed deposit address.

```bash
# Read-only: asks two Solana RPCs for the finalized transaction, never signs.
python3 x3-lang/solana/vault_evidence.py \
  --receipt /path/to/live-quote.near-receipt.json \
  --tx <finalized-solana-transaction-signature> \
  --program <audited-deployed-X3-vault-program-id> \
  --policy <exact-vault-policy-PDA> \
  --source-token <PDA-owned-SPL-token-account> \
  --destination-token <1Click-deposit-SPL-token-account> \
  --nonce <owner-approved-u64-nonce> \
  --expected-mint <independently-verified-Solana-SPL-mint> \
  --rpc-url https://<first-solana-rpc> \
  --rpc-url https://<second-solana-rpc>
```

The required `--expected-mint` is an **independently vetted** classic SPL
mint address (for example, verified against a trusted asset registry and
on-chain mint record); it must not be copied blindly from the receipt.
The quote signs a 1Click asset ID, **not** the receipt's mutable
`contractAddress` metadata. This additional input prevents mutable
receipt metadata from silently redefining the token being "proven", but
the operator must still establish that the pinned mint corresponds to
the 1Click origin asset ID.

The verifier returns `source_verified: true` **only** when every source
check succeeds and both RPCs agree. Its `cross_chain_causality_proven` and
`destination_verified` fields **always remain false**: it cannot by itself
prove solver causality or NEAR payout. The deployed SBF program ID/code hash
must also be independently checked against the reviewed source. A recipient's
balance increase alone may be unrelated. This second proof is still a release blocker.

## Local validation

The program has its own Cargo workspace to avoid contaminating X3Lang's Rust compiler/VM dependency graph.

```bash
cd x3-lang/solana/x3-mandate-vault
cargo fmt --all -- --check
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings

# Requires a separately installed and pinned Solana Agave SBF toolchain.
cargo build-sbf --manifest-path Cargo.toml --sbf-out-dir /safe/build/artifacts
```

`tests/vault_runtime.rs` exercises the actual X3 processor and SPL Token processor in Solana's local program-test banks runtime. This is a test harness, **not** a publicly finalized chain. The SBF artifact is a *compiled candidate* until independently validated and audited; the build tool may create a local program keypair automatically. **Never commit generated keypairs.**

## Known blockers

1. Independent security review of the Solana vault and full integration tests, including malicious token accounts and owner/agent confusion.
2. Authenticated off-chain quote-to-destination instruction construction with no arbitrary RPC trust, and an explicit owner-signing UX.
3. Verified finalized Solana transaction evidence tied to the approved quote digest and actual program/Token CPI.
4. Independently verified NEAR destination transaction and causal link to the same quote, rather than only a balance change.
5. Safe validator/deployment/upgrade authority strategy, public artifact checksums, rollback, and operational monitoring.

**Use a zero-value local test environment until all blockers are closed.** No wallets or funded transaction instructions are shipped by this README.
