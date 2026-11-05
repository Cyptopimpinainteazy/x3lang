# NEAR receipt inclusion proof acquisition — NOT authenticated proof verification

This isolated, read-only prototype requests the documented NEAR `light_client_proof` RPC method with `type: receipt`, `receipt_id`, `receiver_id` and `light_client_head`. It syntactically rejects malformed base58 32-byte identifiers, mismatched receipt ID/contract executor, failed execution status, missing proof components, malformed Merkle branch directions, oversized proof objects, HTTP failures and mismatched RPC request IDs. The returned SHA-256 fingerprint is a digest **of untrusted response JSON**, not a cryptographic receipt inclusion proof.

The `UnauthenticatedReceiptCandidate.authenticated` and `.authorized_to_sign` flags are always `False`. `authorize_receipt()` deliberately raises. There is no signing key import or transaction submission. Even a correctly formatted `light_client_proof` object can be forged by an RPC service. Do not feed any candidate from this module to EIP-712 BSC payout signing.

Sources for upstream proof formats and verification:
- NEAR Light Client spec: https://nomicon.io/ChainSpec/LightClient.html
- NEAR light-client-proofs RPC schema: https://docs.fastnear.com/rpc/protocol/light-client-proof
- NEAR light client reference: https://github.com/near/near-light-client

## Missing before financial authorization

1. Instantiate and pin a trusted NEAR checkpoint and validator set. Advance a real light client with verified validator signatures, epoch transitions and finality guarantees.
2. Independently verify the complete outcome, outcome-root and block Merkle branches against that trusted finalized head, including exact outcome serialization and all protocol version rules.
3. Verify a *specific successful contract call*, its receipt parameters, code deployment hash, and exact escrow state transition. A successful receipt alone is not necessarily a durable final `Claimed`, `Refunded` or `NotFunded` escrow state; `NotFunded` must be proven through authenticated contract state and immutable on-chain deadline rules.
4. Bind verified settlement facts to the BSC escrow, chain ID, expected LP/recipient, swap ID, amount and hashlock, then implement isolated authenticated quorum signing and independent anti-equivocation controls.
5. Review and test adversarial receipts, reorgs, duplicate signatures, censorship and outage recovery, followed by funded testnet evidence and an independent audit.

**Status:** proof-request parser and fail-closed ingress complete; actual light-client verifier NOT IMPLEMENTED. `NO_MAINNET_FUNDS`, no auto-signing or deployment. The producer and verifier of a proof must be independent.
