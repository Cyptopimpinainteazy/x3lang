# X3 bridge: attested settlement pilot (testnet-only, trusted committee)

## What is implemented

`x3-lang/bridge/bsc/src/AttestedBnbEscrow.sol` is a **separate** experimental BNB escrow. It does NOT replace the existing `NativeBnbHTLC` or deploy anywhere. It cannot be deployed on BSC mainnet (chain ID 56). Its constructor fixes three verifier addresses; 2-of-3 distinct secp256k1 ECDSA attestations determine whether an exact swap's NEAR contract ended in `Claimed`, `Refunded`, or `NotFunded` state. The EIP-712 digest binds the BSC chain ID and escrow address, depositor, LP, exact swap ID and hashlock, NEAR network tag, NEAR vault and recipient account hashes, NEAR amount, BNB amount, BSC expiry, NEAR funding cutoff, NEAR expiry and outcome. New escrows require the NEAR funding deadline to precede the NEAR claim expiry and require at least a one-hour BSC release buffer after NEAR expiry. Anyone may submit a valid `Claimed` certificate to pay the BNB liquidity provider; the BNB depositor cannot veto payment. Refund outcomes additionally require both the corresponding NEAR deadline and the BSC expiry. A late authenticated NEAR `Claimed` certificate still pays the provider after expiry.

No fee mechanism, upgrade authority, signer rotation or unilateral timelock refund exists. Default failure mode on missing quorum is **indefinite locked BNB**, not falsely claiming completion. This eliminates unilateral depositor-withholding under honest, available quorum, at the expense of trusted validators and liveness.

## Fundamental security limitations

- **Not trustless.** Any 2 colluding or compromised verifier keys can attest a false NEAR outcome and steal escrowed BNB. Even 3 honest verifiers can become unavailable or censored and lock assets indefinitely. Tests use local signing keys, not authenticated NEAR execution evidence.
- **Off-chain verifier not implemented.** There is no authenticated NEAR RPC final-state reader, consensus proof/light client, quorum key management, signer slashing, or anti-equivocation evidence storage. Actual attestations MUST follow independently verified **finalized contract state** (not a transaction hash, public preimage, RPC status string, or transient transfer). Every signer must check exact vault account, swap ID, recipient, amount, hashlock and claim/refund state.
- **Swap identity derivation is off-chain.** The reviewed Python plan derives the swap identity from both contract identities, both parties, both amounts, both deadline schedules and a 32-byte nonce. `AttestedBnbEscrow` accepts a supplied `swapId` and binds it into the certificate, but cannot prove that the ID was derived from those terms. The verifier must recompute the plan identity before signing or accepting an outcome.
- **No-lock evidence.** The NEAR contract has an enforceable funding cutoff and terminal `NotFunded` state, and the BSC escrow recognizes a separate signed `NotFunded` outcome. The missing component is an authenticated observer that proves the exact terminal NEAR storage state and deployed code identity. A corrupt or unavailable committee can still refund falsely or lock BNB indefinitely.
- **Conflicting certificates.** A corrupt quorum could sign both outcomes; first finalized BSC settlement wins. Quorum dishonesty is not preventable by contract alone. Attester bond/slashing, operational independence, monitoring and a formal threat model are missing.
- **Solvency and observability.** NEAR `Claimed` currently credits the recipient inside its contract, but later asynchronous transfer could fail; correct reserve accounting, withdraw retry, storage/gas sufficiency, refund safety and destination account viability still need adversarial tests and on-chain proof.
- **Not integration tested.** No funded BSC/NEAR testnet deployments, independent chain finality receipts, live funding, authentic two-chain settlement, key provisioning or audit exist.
- **Not linked into production routing.** No wallet, relayer, liquidity registry or `.x3` runtime is authorized to call this contract. The existing `NativeBnbHTLC` remains unsafe for permissionless settlement.

## Required next gates

1. Implement an authenticated observer for the existing terminal `NotFunded` state and bind it to the deployed NEAR escrow code identity without making a successful NEAR credit refundable.
2. Implement the external committee's authenticated finalized NEAR state verification, signer authorization and durable anti-equivocation controls. Treat signed outcomes as financial authorization.
3. Test reorgs, malformed receipts, stale/conflicting RPCs, forged and duplicate signatures, replay, chain/contract substitution, committee censorship and disaster recovery.
4. Conduct adversarial two-chain testnet trials, independent code audit, verified builds, exact-head CI and owner-controlled staged funds approval.

**Release status: BLOCKED; no mainnet funds, deployment or live wallet authority.**
