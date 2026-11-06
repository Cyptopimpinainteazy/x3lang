# NEAR light-client Merkle primitives — cryptography implemented, finality NOT verified

The module `x3-lang/bridge/near_merkle.py` implements protocol-level cryptographic operations described in the [NEAR light-client specification](https://nomicon.io/ChainSpec/LightClient.html):

- Strict base58 decoding of canonical 32-byte NEAR hashes, with rejection of malformed/oversized representations.
- Borsh serialization of `BlockHeaderInnerLiteView` (little-endian `u64` height/timestamp and six 32-byte hashes), combined with supplied `inner_rest_hash` and `prev_block_hash` using the specified nested SHA-256 block hash.
- SHA-256 binary Merkle-path reconstruction respecting Left/Right sibling placement and rejecting malformed paths.
- Cryptographic comparison of calculated block-header hash with the receipt proof's claimed containing block and reconstruction of its membership against a **caller-supplied** head block-Merkle root.

These are **real hash calculations**, but the source of the expected Merkle root is still untrusted. The `BlockMerkleCheck.authenticated` and `.authorized_to_sign` flags are always `False`. No committee key is loaded or signature produced. None of this code is wired to BSC `AttestedBnbEscrow.settle`.

## Crucial missing cryptographic validation

1. **A real checkpoint and validator chain:** a trusted NEAR checkpoint must be obtained and pinned independently; validate successive light-client block views with epoch/next-epoch commitments, Ed25519 validator approval signatures, strict >2/3 total signed stake, `next_bps` Borsh commitment, and valid header / epoch transitions. Only then can the head block-Merkle root be treated as authenticated.
2. **Outcome inclusion:** hash an exact NEAR execution outcome per its versioned Borsh encoding; verify `outcome_proof.proof` and `outcome_root_proof` against the included header's outcome root. A successful receipt alone does not prove the permanent terminal state of the escrow.
3. **State / code identity:** prove the expected vault code hash and authenticated escrow storage state (`Claimed`, `Refunded`, `NotFunded`) at a finalized block. This includes contract-specific withdrawal and funds-liability semantics.
4. **Financial approval:** bind the above to BSC contract address, chain IDs, swap and vault identity, owners, hashlock, amounts, and the exact output. Implement audited signer isolation, distributed anti-equivocation, chain reorg and crash recovery, then funded testnet trials and audit.

Tests use synthetic Borsh and hash vectors as **algorithmic unit tests**, not transactions, live chain proofs, trusted validator sets, or production evidence. Even a successful `check_receipt_block_merkle` only proves membership relative to the input root, not consensus finality. Never connect it to a financial signer.

**RELEASE BLOCKED / NO_MAINNET_FUNDS.**
