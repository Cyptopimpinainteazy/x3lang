# Stage 2 read-only NEAR state observation — **not a payout authorizer**

This preliminary module (`x3-lang/bridge/near_observer.py`) reads the experimental
`NativeNearHtlc.get(id)` view using `finality: final` from at least two distinct
HTTPS RPC hostnames. It requires identical block hash, height and the complete
swap fields. It checks the expected NEAR vault account in the RPC request,
swap ID, initiator, recipient, hashlock, integer amount, expiry and state.
Malformed and missing state, failed RPCs, endpoint disagreement, or wrong
assets and accounts all fail closed. A local SQLite journal retains terminal
observations across process restarts, rejecting conflicting terminal state.

**This is only observational agreement, not independently authenticated
NEAR consensus finality or an escrow-payment proof.** Two endpoint hostnames may
share an upstream or collude. Finalized block hash and contract-view response
are untrusted without verified NEAR consensus/receipt proofs, and no pinned
contract code hash or deployment identity is checked. An attacker with control
of the input endpoints may fabricate both responses. No verifier keys are
loaded, no certificate is signed, and there is no bridge deployment or funds
movement. The SQLite journal is not a distributed lock or attester quorum.

## Strict no-signing trust boundary

`Observation.authorized_to_sign` is always `False`. Do **not** connect
`observe()` or `OutcomeJournal.record()` output to the BSC
`AttestedBnbEscrow.settle()` call or a signing key. The BSC testnet contract
only checks signatures, not whether the facts were true. A 2-of-3 signer quorum
can forge outcomes. Compromised signers can steal BNB; unavailable signers can
lock it forever.

## Unimplemented gates

1. Pinned NEAR vault account **and deployed code hash** on known network.
2. Authenticated finalized NEAR state proof verified independently of RPC text,
   with chain identity, transaction/receipt causality and correct storage proof.
3. Independent 2-of-3 signers, durable equivocation prevention, secure key
   management and deliberate recovery procedure (not a local database alone).
4. Safe `NotFunded` evidence requiring an explicit latest-lock deadline and
   authenticated proof of absence at a later finalized block. RPC errors and
   empty `get` responses are **not** absence certificates.
5. Persistent relayer workflow, retry/idempotency, reorg handling and adversarial
   two-chain funded testnet evidence before independent security review.

**Release: NO_MAINNET_FUNDS.** This module is a protective read-only building
block, not a trustworthy settlement certificate issuer.
