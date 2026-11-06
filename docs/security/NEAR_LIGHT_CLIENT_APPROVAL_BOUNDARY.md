# Stage 2 NEAR approval signatures — cryptographically checked, NOT a finality verifier

`x3-lang/bridge/lightclient-core/` is an isolated Rust primitive with pinned `ed25519-dalek` dependencies. Its `approval_message()` encodes NEAR `ApprovalInner::Endorsement(next_block_hash)` using the Borsh enum tag and `height + 2`, with checked arithmetic. `reconstruct_next_block_hash()` computes `SHA256(next_block_inner_hash || current_block_hash)`. `verify_approvals()` checks Ed25519 signatures against the **complete caller-supplied validator set**, rejects duplicated keys, invalid signatures, overflow and zero stake, counts only authenticated signatures, and requires signed stake **strictly greater than 2/3** of total stake. Missing trailing signatures are treated as absent; signatures beyond validator count contribute no stake. Rust tests use deterministic test keys and cover forged signatures, wrong height, duplicate keys, insufficient and sufficient stake.

## WARNING — untrusted validator set

Passing these checks DOES NOT authenticate NEAR's finalized chain. A malicious service can invent a validator set and sign a block with those keys. Do not use `verify_approvals` to authorize settlement without **all** of the following: independently pinned checkpoint; authentic preceding validator/epoch commitments; `next_bps` versioned Borsh hash and rotation; verified block-header hash and chain transition; actual receipt outcome Borsh hashing/Merkle inclusion; storage/code identity; exact swap term binding and independent audit. No network fetch, live validator set, checkpoint database, signer, BSC contract call, RPC secrets, or wallet transfer is implemented in this crate.

This crate must remain isolated from any financial signing key. NEAR protocol upgrades and `ValidatorStakeView` variants must be reviewed before accepting external validator sets. Current tests are synthetic cryptographic acceptance tests, **not live-chain proof fixtures**. Mainnet and unattended fund movement are blocked.

References: https://nomicon.io/ChainSpec/LightClient.html and https://github.com/near/near-light-client
