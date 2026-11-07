# X3 bridge: real NEAR receipt outcome and validator commitment checks

This draft implements *cryptographic integrity checks*, **not an authenticated NEAR light client**, release, bridge attestation, or permission to move funds. Both checkers always return `authenticated=False` and `authorized_to_sign=False`. There is no signer or transaction sender. Only read-only, offline code was exercised.

## Real testnet evidence, copied byte-for-byte from draft PR #57

The two raw JSON-RPC response fixtures are duplicate read-only, independently hash-checked references to the genuine NEAR testnet artifacts already captured in draft PR #57, `Cyptopimpinainteazy/x3lang` commit `53ed5bd363c7eb840b599ec4bf289e8d07b88c9e`:

1. `x3-lang/bridge/fixtures/near_live_receipt_proof_20261105.json`: SHA-256 `0fb16971ff172bd30e6ada514a3c9ea688ac77815ea506f28be09b6b3512d5ab`; original `evidence/near-proof/testnet/sha256/0f/0fb16971ff172bd30e6ada514a3c9ea688ac77815ea506f28be09b6b3512d5ab.json`. Requested receipt `3f9FCzqC8k75AGob2UrCxbKVqADRyv5M5vGfoLEXbZyu`, executor `league-v7.near-post.testnet`, light-client head `V7KcHcg16BVWWxhVbw2vQj87D3dJQwXchnU7DCVzo6V`. Its **UNTRUSTED** head block-Merkle root `6F2mBDUkzB8tRpETPpRTtH5zo1E5yGGpftfyNYbRVDVw` was fetched separately by an RPC `block` query for that head. The receipt's recomputed outcome root equals the lite header's `7mUR47bB4y7d7MbakknJGSdqmpCEDL5X47Y5LMq9BQnp`; the receipt block's Merkle path also matches the RPC-provided head root.
2. `x3-lang/bridge/fixtures/near_live_light_client_update_20261105.json`: SHA-256 `c47f0c381dbd9a37ac432aa4f77cf9d47ea8ccc452bfe4ff2a67b868953caed7`; original `evidence/near-proof/testnet/sha256/c4/c47f0c381dbd9a37ac432aa4f77cf9d47ea8ccc452bfe4ff2a67b868953caed7.json`. V1 `next_bps` Borsh serialized hash equals claimed `next_bp_hash` `4nzeJKGgHtgtESYKxfNh7Qq38WuYxCZ4zbVhjQHUYvez` for the 20-validator update.

## Implementation

- `near_outcome.py`: Borsh encodes nearcore `PartialExecutionOutcome` (receipts, gas, tokens, executor, partial execution status); **excludes logs and execution metadata from the partial hash**. Creates `ExecutionOutcomeWithId::to_hashes() = [id, SHA256(partial), SHA256(each log bytes)]` and `SHA256(Borsh(Vec<CryptoHash>))`; validates the outcome-to-shard Merkle path, the SHA256 of the shard root and the shard-to-block outcome Merkle path against the lite header's `outcome_root`. It independently rehashes the lite header and block ancestry against a caller-supplied head Merkle root. Rejects malformed inputs and mismatches. This is cryptographic integrity **relative to untrusted RPC parameters**, not consensus.
- `near_validator_commitment.py`: Borsh encodes `Vec<ValidatorStake::V1>` with `V1` enum tag, AccountId String, Ed25519 key variant tag, raw 32-byte key, and 128-bit little-endian stake. Validates SHA256 result against the **untrusted** lite block's `next_bp_hash`. Rejects duplicated validator accounts/keys, invalid stakes, unknown validator versions and unsupported key algorithms. **V2 and future formats are explicitly not yet supported**.

Real positive fixtures and 20+ adversarial mutation subtests verify the above. An attacker able to invent both the header and validator set can still fabricate a matching commitment. A malicious RPC able to replace both the header and head root can still construct valid Merkle paths. **Never accept a proof as final merely because these comparisons match.**

### Critical remaining release gates

- Pin a trusted NEAR testnet checkpoint with independent provenance and confirmed chain identity. Advance heads via versioned `next_bps` commitments, authenticated **preceding-epoch** validators, valid epoch transitions, reconstructed endorsement messages, Ed25519 signatures and strict >2/3 total stake. The candidate update's *next* validator set must NOT be used to verify signatures from its *current* epoch.
- Cross-check protocol upgrades and V2/future validator stake layouts with nearcore reference source and full inter-epoch fixtures.
- Authenticate the deployed X3 escrow code identity and finalized **persistent storage state** for `Claimed`, `Refunded`, or irrevocably `NotFunded` under its registered funding deadline, including the exact swap terms. A successful unrelated receipt is not proof of X3 settlement.
- Add distributed signer anti-equivocation, protected keys, crash-safe relaying, third-party audit and funded **testnet-only** adversarial matrix before enabling any financial output.

**NO MAINNET FUNDS; NO EIP-712 ATTESTATION SIGNING; NO DEPLOYMENT; NO PAYMENT.**

References: [nearcore source: PartialExecutionOutcome / ExecutionOutcomeWithId::to_hashes](https://github.com/near/nearcore/blob/master/core/primitives/src/transaction.rs), [nearcore light client specification](https://github.com/near/nearcore/blob/master/docs/ChainSpec/LightClient.md), and [real evidence PR #57](https://github.com/Cyptopimpinainteazy/x3lang/pull/57).
