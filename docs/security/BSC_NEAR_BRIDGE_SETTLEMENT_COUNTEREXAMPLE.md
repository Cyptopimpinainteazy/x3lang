# BSC ↔ NEAR bridge settlement counterexample (blocking)

This report documents a **confirmed protocol design failure** in the experimental native-asset bilateral escrow at `x3-lang/bridge/`. It is not evidence of a live exploit or a funded transaction.

## Invariant required for mainnet

After final NEAR delivery to the source-side depositor, the counterparty providing NEAR must be able to receive the escrowed BNB under an enforceable protocol rule. Neither party may receive the other party's asset **and** unilaterally recover or withhold its own payment.

## Counterexample: NEAR first, BNB withheld

1. Alice (BNB depositor) locks BNB for Bob (NEAR liquidity provider) in `NativeBnbHTLC`.
2. Bob locks NEAR for Alice in `NativeNearHtlc`.
3. Alice obtains final NEAR credit by a successful NEAR `claim` (and can initiate withdrawal). The preimage is public, but `NativeBnbHTLC.ownerRelease` is restricted to Alice.
4. Alice refuses to call `ownerRelease`. Bob's attempt reverts even with the correct preimage.
5. After BSC expiry, Alice calls `refund` and recovers her original BNB. Bob provided NEAR but received no BNB.

`testCounterexampleNearCreditedButDepositorRefundsBnb` encodes the BSC portion and **explicitly assumes** finalized NEAR credit; it is not a two-chain integration test. NEAR-side claim/credit behavior is separately implemented in the Rust contract. Successful tests demonstrate the unsafe reachable BSC state, not acceptable settlement.

## Other attack: preimage disclosed without NEAR credit

A NEAR transaction that fails after disclosing the preimage may reveal it without committing a NEAR credit. Allowing arbitrary BSC release on knowledge of that preimage would then let a counterparty collect BNB without payment. The depositor-only gate prevents this attack but creates the counterexample above. Removing the gate is therefore **not** a safe fix.

## Required redesign before live value

- Choose and document an explicit settlement trust model. A pure preimage-only cross-chain HTLC is insufficient for the stated failure model.
- For trust-minimized settlement, require independently verifiable finalized **success state** and authenticated linkage of the NEAR swap/recipient/amount/claim to the BSC payout. If using attestations or a quorum, expose and audit the new trust/censorship assumptions. Never treat a client-provided boolean or balance screenshot as proof.
- Alternatively, for a **trusted counterparty pilot only**, make it explicit that the depositor's authorization is an off-chain contractual trust requirement; this is not permissionless or atomic settlement.
- Build tests for failed-claim preimage disclosure, owner withholding after successful destination claim, both expiry orders, claim/refund races, chain reorg/finality disagreement, replay/cross-swap substitution, callback/withdrawal failures and relayer restarts.
- Require full two-chain funded testnet swaps and independent audit before authorization to deploy/fund mainnet.

**Release decision:** `NO_MAINNET_FUNDS` until a redesigned protocol closes both counterexamples and has independent verification. The BSC constructor's testnet-only chain-id gate must remain intact.
