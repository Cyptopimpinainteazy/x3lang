# NEAR-finality-bound BNB settlement (issue #42)

**Release decision: `NO_MAINNET_FUNDS`. Not deployable.** This document and its
code close the *contract-level* free-option paths of issue #42, but the BSC
contract needs a NEAR verifier that does not exist yet (issue #56). Until a
reviewed verifier exists, nothing here can settle real value. This is post-freeze
draft work. It does not touch the frozen submission commit `f2a01a2`, NEAR
Guard (`x3-guard`) or the submission evidence.

| Component | File |
|---|---|
| BSC escrow | `x3-lang/bridge/bsc/src/NearSettledBnbEscrow.sol` |
| Verifier interface (unimplemented, #56) | `x3-lang/bridge/bsc/src/INearSettlementVerifier.sol` |
| NEAR contract changes | `x3-lang/bridge/near/src/lib.rs` (on top of PR #246) |
| Reference encoding | `x3-lang/bridge/settlement.py` |
| Exhaustive free-option model | `x3-lang/bridge/settlement_model.py` |
| Mutation gate | `scripts/security/bridge_settlement_mutation.py` |

## 1. The defect

A NEAR `claim(id, preimage)` publishes the 32-byte preimage in its transaction
arguments whether or not the claim succeeds. A claim can fail, land after NEAR
expiry, sit pending, or be dropped from a non-final block. The original BSC
HTLC (`13054df`) paid the LP for any matching SHA-256 preimage, so the LP could
take the BNB while also refunding its own NEAR.

Commit `03a33a9` (current `NativeBnbHTLC`) made `ownerRelease` depositor-only.
That stopped the LP theft but created the opposite free option: once the
depositor holds final NEAR credit, they can withhold release and refund the BNB
after BSC expiry. See `BSC_NEAR_BRIDGE_SETTLEMENT_COUNTEREXAMPLE.md`.

Both defects share one root cause: the BSC withdrawal rule never looks at NEAR.

## 2. Threat model

**Parties.** Alice deposits BNB on BSC and wants NEAR. The LP deposits NEAR and
wants BNB. Either one may be adversarial. Anyone may relay evidence.

**Adversary capabilities** (all modelled or tested):

- reading every mempool and every failed receipt, including leaked preimages;
- delaying any transaction past any deadline (an adversarial scheduler);
- reordering, front-running and replaying transactions on both chains;
- getting NEAR blocks that are executed but **not final** dropped (reorg);
- submitting evidence for other swaps, other outcomes, other vaults, other
  code hashes or other NEAR chains;
- payout addresses that revert or re-enter;
- reusing the same secret or hashlock across swaps;
- funding NEAR partially or never;
- squatting a swap id on NEAR with a direct `lock()`.

**Trusted assumptions** (not provided by this change):

1. NEAR BFT finality. A block the verifier treats as final is never reverted.
2. **A correct `INearSettlementVerifier` (issue #56).** This is the critical
   dependency. A wrong verifier can pay anyone.
3. The NEAR vault's code is the reviewed code. Its code hash is pinned, and no
   key exists that could redeploy it. The verifier must prove the code hash.
4. SHA-256 collision resistance. Window ids and commitments are SHA-256.
5. Before funding NEAR, the LP checks that the BSC lock is final on BSC. This is
   the LP's own diligence. The NEAR contract cannot see BSC.

## 3. Identifiers

All integers are big-endian. `lp(s) = u32 len || utf8(s)`. The same vector is
asserted in Python, Rust and Solidity (section 9).

```
source_binding = sha256("X3-BSC-SOURCE-V1" || u256 bsc_chain_id || escrow[20]
                        || depositor[20] || lp_evm[20] || u256 bnb_wei || salt[32])

window_id      = sha256("X3-NEAR-BSC-WINDOW-V1" || lp(near_vault) || lp(near_initiator)
                        || lp(near_recipient) || hashlock[32] || u128 near_yocto
                        || u64 funding_deadline_ns || u64 near_expiry_ns
                        || source_binding[32])

commitment     = sha256("X3-NEAR-SETTLEMENT-V1" || window_id[32] || u8 outcome)
                 outcome: 1 Registered, 2 Claimed, 3 Refunded, 4 NotFunded

NEAR log line  = "X3_NEAR_SETTLEMENT_V1 " || lowercase_hex(commitment)
```

`register_window` on NEAR rejects any `id` that is not this `window_id` over the
supplied terms. A record stored under a window id therefore carries exactly the
terms the BSC escrow derives that id from. Neither the registrar nor anyone else
can attach different terms to it.

## 4. Protocol and timelock ordering

```
t0    Alice picks salt + secret; computes source_binding and window_id.
      Registrar calls NEAR register_window(window_id, terms, source_binding)
        -> NEAR log Registered(window_id)
t1    Registered is FINAL on NEAR and proven to the BSC verifier.
      Alice calls BSC lock(terms, registrationProof) with msg.value = bnb_wei.
        The escrow re-derives window_id from msg.sender, msg.value and terms.
        No final Registered proof for exactly that id -> revert.
t2    LP checks the BSC lock is FINAL on BSC, then calls
      NEAR fund_window(window_id) before funding_deadline (exact amount).
t3    Alice checks funding is FINAL on NEAR, then calls claim(window_id, secret)
      before near_expiry.  -> NEAR log Claimed(window_id), credit to Alice.
end   Exactly one terminal NEAR outcome per window, every one permissionless:
        Claimed    only if claim executes at  now <  near_expiry
        Refunded   anyone may call refund at  now >= near_expiry (credit -> LP)
        NotFunded  anyone may call close_unfunded at now >= funding_deadline,
                   only if never funded
      Anyone relays the FINAL terminal outcome to BSC settle().
```

**Timelock ordering:** `now < funding_deadline < near_expiry`, with
`expiry > deadline + 300 s`, enforced by NEAR `register_window`. **The BSC
escrow has no clock.** NEAR's own timelocks decide which outcome is reachable,
and the two NEAR boundaries are strict complements:

- `fund` requires `now < deadline`; `close_unfunded` requires `now >= deadline`.
- `claim` requires `now < expiry`; `refund` requires `now >= expiry`.

So no instant exists at which two terminal outcomes are both reachable.
Staggered BSC/NEAR expiries are no longer a safety mechanism, and no race
exists at a BSC deadline because there is no BSC deadline.

## 5. The exact withdrawal condition

For a BSC swap stored under `window_id`, in state `Locked`:

```
BNB -> LP         iff  V(commitment(window_id, Claimed),   proof) == MAGIC
BNB -> depositor  iff  V(commitment(window_id, Refunded),  proof) == MAGIC
                    or V(commitment(window_id, NotFunded), proof) == MAGIC
```

`V` is `STATICCALL verifier.verifySettlement(genesis, vault, codeHash,
commitment, proof)`. `MAGIC` is that function's selector, compared as the exact
32-byte return word. A revert, a missing contract, short or long return data,
dirty padding, ABI `true` or a state write all fail closed. These are tested.

Additional rules:

- The first verified terminal outcome moves the swap to `PaidToLp` or
  `RefundedToDepositor`. Nothing settles twice.
- `Registered` is never a terminal outcome.
- **No function accepts a preimage.** Publishing the secret confers nothing on
  BSC.
- Payouts are pull-based. A payee whose address reverts keeps its credit and
  can withdraw it to another address.
- There is no admin role, upgrade, pause or timeout refund.

## 6. Why no free option remains

**LP theft (the issue #42 attack).** The LP gets BNB only with a final,
successful NEAR `Claimed` outcome for this exact `window_id`. That outcome
exists only if a claim executed before expiry, by the recipient bound in the id,
for the amount bound in the id. That claim credits Alice. A failed, late,
pending or reorged claim produces no final successful `Claimed` outcome, so it
pays nothing. The leaked preimage has no BSC entry point.

**Depositor double-dip (the `03a33a9` counterexample).** Alice gets BNB back
only with a final `Refunded` or `NotFunded` outcome. Neither can coexist with
`Claimed` for the same window (section 4). Once `Claimed` is final, anyone can
relay it, and Alice has no veto and no timeout.

**Stranded funds.** BNB can only be locked against a window proven to exist
with exact terms. Such a window always reaches a terminal outcome through
permissionless calls: `close_unfunded`, permissionless `refund` (changed by this
PR), or Alice's own claim. Before this PR, `refund` was initiator-only, and an
LP that funded and then went silent could strand Alice's BNB forever. The model
catches this as the `REFUND_INITIATOR_ONLY` mutant.

**Squatting.** A direct `lock()` can take a window id before registration. That
is griefing only: Alice never gets a `Registered` proof, so she never locks BNB.
Direct-lock swaps never emit settlement logs.

**Model check.** `settlement_model.py` exhaustively explores interleavings over
a clock covering every boundary. It includes an adversarial scheduler, NEAR
reorgs of non-final state and failed receipts, and checks `AG EF honest party
whole` for each honest party.

| Rule | Alice honest | LP honest |
|---|---|---|
| LEGACY_HASHLOCK (13054df) | **free option**: submit claim before expiry → delayed past expiry → LP claims BNB with preimage | **free option**: LP stalled past BSC expiry |
| OWNER_RELEASE (03a33a9, master) | safe | **free option**: Alice claims NEAR, withholds release |
| NEAR_SETTLED (this PR) | safe | safe |

All 7 model mutants of NEAR_SETTLED are killed: `ACCEPT_PENDING`,
`ACCEPT_FAILED_RECEIPT`, `PAY_LP_ON_REFUND`, `TIMEOUT_REFUND`,
`CLAIM_WITHOUT_PROOF`, `LOCK_WITHOUT_REGISTRATION` and `REFUND_INITIATOR_ONLY`.
This is a finite model with one window and abstract time. It is evidence, not a
proof about the deployed bytecode.

## 7. Verifier dependency (issue #56)

`INearSettlementVerifier` is the interface #56 must implement on BSC. It must
return the magic word **only if** the proof authenticates, against a block the
verifier holds as **final**, on the chain with the pinned genesis hash:

1. outcome inclusion: outcome proof, outcome root, block Merkle path and header
   hash;
2. `executor_id == near_vault`;
3. status `SuccessValue` or `SuccessReceiptId`. NEAR keeps the logs of failed
   receipts, so a log line alone is not enough;
4. a log entry byte-equal to `"X3_NEAR_SETTLEMENT_V1 " || hex(commitment)`;
5. vault code hash at that block equal to the pinned hash (state proof).

The repository has no such implementation. `near_authenticated.py` is an
off-chain Python head store, not an on-chain verifier. A committee that signs
after running it would be a trusted committee, with the same trust as
`AttestedBnbEscrow`. If #56 ships that, it must be labelled as trusted.

Tests use `test/NearSettlementVerifierDouble.sol`, a **test double**. It lets
the test write a fake NEAR chain (blocks, finality, reorgs, failed receipts,
executors, code hashes). It shows the escrow behaves correctly **given** a
verifier that meets the spec. It authenticates nothing.

## 8. Adversarial matrix

| Attack | NEAR test (`cargo test`) | BSC test (`forge test`) | Model |
|---|---|---|---|
| Failed destination claim | `failed_claims_emit_no_settlement_log_and_success_emits_exactly_one` | `testLeakedSecretFromFailedNearClaimCannotPayLp`, `testGreen_NearSettledEscrow_LeakedSecretGivesLpNothing` | `ACCEPT_FAILED_RECEIPT` killed |
| Pending destination claim | — | `testPendingNearClaimCannotSettleUntilFinal` | `ACCEPT_PENDING` killed |
| Expiry race at timelock boundaries | `claim_one_nanosecond_before_expiry_is_terminal_and_excludes_refund`, `refund_one_nanosecond_before_expiry_fails_and_at_expiry_is_terminal`, `can_never_close_funding_window_early_or_fund_at_deadline` | `testLateFinalClaimStillPaysLpAndNoTimeoutRefundExists` | `TIMEOUT_REFUND` killed; legacy red trace |
| Same-secret replay | `same_hashlock_across_windows_settles_each_window_independently` | `testSameSecretReplayAcrossSwapsAndProofReplayRejected` | — |
| Reentrancy | — | `testReentrantWithdrawAndSettleCannotDoublePay`, `testMalformedVerifiersFailClosed` (stateful verifier) | — |
| Omitted destination funding | `omitted_funding_closes_with_not_funded_log_and_nothing_follows` | `testOmittedNearFundingRefundsDepositorOnlyViaNotFunded`, `testGreen_NearSettledEscrow_NoOneCanReleaseWithoutNearClaim` | `LOCK_WITHOUT_REGISTRATION` killed |
| Partial execution | `unauthorized_registrar_and_wrong_lp_or_amount_rejected` | `testLockRequiresFinalRegistrationOfExactTerms` | — |
| Stale / reorged / foreign evidence | `window_id_must_commit_to_every_term_and_the_source_binding` | `testReorgedOutNearClaimIsRejected`, `testEvidenceForOtherVaultCodeOrChainIsRejected`, `testRegistrationProofIsNotATerminalOutcome`, `testFrontRunnerCannotUseAnotherDepositorsWindow` | `ACCEPT_PENDING` killed |
| Withdrawal callback failure | `failed_async_wallet_transfer_restores_claimant_credit` (existing) | `testWithdrawalCallbackFailureKeepsCreditAndCanRedirect` | — |
| Secret publication | `failed_claims_emit_no_settlement_log_and_success_emits_exactly_one` | `testPublishedSecretHasNoBscEntryPoint` | legacy red trace |
| Window-id squatting | `direct_lock_squatting_a_window_id_cannot_forge_settlement` | — | — |
| Stranded BNB | `refund_after_expiry_is_permissionless_but_credits_only_initiator` | — | `REFUND_INITIATOR_ONLY` killed |
| Fail-closed verifier wiring | — | `testMalformedVerifiersFailClosed`, `testConstructorRejectsMissingVerifierMainnetAndNonTestnetVault` | `CLAIM_WITHOUT_PROOF` killed |

**Red/green.** `Issue42RedGreen.t.sol` passes three RED tests in which the
attack **succeeds**:

- the leaked preimage against `LegacyPreimageHTLC` (verbatim `13054df` logic,
  kept as a test fixture);
- depositor withholding against the current `NativeBnbHTLC`;
- early release against the current `NativeBnbHTLC`.

Each has a GREEN partner that runs the same scenario against
`NearSettledBnbEscrow` and shows the attack failing.

## 9. Cross-language vector and mutation gate

The vector uses inputs chain `31337`, escrow `0x…0E5C40`, depositor
`0x…0A11CE`, LP `0x…00BEEF`, `1e18` wei, salt `0x11…`, vault `x3-vault.testnet`,
`lp.testnet` → `alice.testnet`, hashlock `sha256(0x03×32)`, `1e24` yocto,
deadline `1_700_000_600_000_000_000` and expiry `1_700_001_200_000_000_000`.

| Value | Hash |
|---|---|
| window | `9d66ddfe92e56e993dd87e2f7ca53b87e39ceb35325cc44b404c0a8eb87de110` |
| Claimed | `c956f53937ecff906f42672b636a1b8216fd7beade6dd7eeaaa2074e52d9177e` |

The vector is asserted in `test_settlement.py`,
`settlement_encoding_matches_cross_language_vector` and
`testSettlementEncodingMatchesCrossLanguageVector`.

`scripts/security/bridge_settlement_mutation.py` applies 9 BSC and 5 NEAR
single-expression mutants to disposable copies and reruns the real suites. Any
surviving or invalid mutant fails the gate. CI runs it in the bridge jobs.

## 10. Residual risks (not solved here)

- **No verifier.** The escrow is only as sound as its pinned verifier. None
  exists (#56). The constructor accepts any contract that reports the expected
  genesis hash, so deployment review must pin the exact verifier code.
- **Liveness.** If the verifier halts forever, locked BNB stays locked. There
  is deliberately no escape hatch, because any timeout default recreates a free
  option. This trade-off needs an owner decision.
- **Economic optionality.** Between NEAR funding and NEAR expiry, Alice can
  choose whether to claim depending on price. That is the standard HTLC
  "American option": it costs the LP time value, but it is not theft. Mitigate
  with short claim windows or an LP premium.
- **LP diligence.** The LP must fund NEAR only after the BSC lock is final. A
  careless LP that funds first can lose NEAR to an Alice who never locks BNB.
- **Registrar.** NEAR `register_window` is still registrar-only. With
  term-committed ids the registrar affects liveness only, not safety.
- **NEAR vault identity.** The vault account must have no full-access key, and
  the code hash must be proven by the verifier. Neither is enforced in-contract.
- **Unchanged legacy contracts.** `NativeBnbHTLC` and `AttestedBnbEscrow` stay
  in the repository unchanged. Both are testnet-only and must not be used for
  permissionless settlement.
- **Unmeasured costs.** No NEAR sandbox gas measurement, BSC gas review or
  storage review of the new log lines has been done. No funded testnet trade,
  independent audit or reproducible release build exists.

## 11. Issue #42 gate status

| Gate | Status |
|---|---|
| Protocol bound to independently finalized NEAR claim/credit | **Partial.** Protocol, BSC condition, NEAR events and tests are done. The independent verifier is blocked on #56. |
| No free option (failed/pending calls, timelocks, reorgs, callback failure, secret publication) | **Done at contract and model level**, under the trust assumptions in section 2. Needs independent review. |
| Reconcile chain identity, code hash, swap id, amount, recipient, finality and inventory via independent verifiers | **Partial.** All terms are bound in `window_id`, and genesis and code hash are pinned and passed to the verifier. Verification itself is #56. Escrow inventory is unchanged. |
| Contract-level fault matrix | **Done locally** (section 8). The *testnet* fault matrix is not run: it needs deployments and funds. |
| Independent review, reproducible build, exact-head CI, funded testnet trade, runbooks | **Open.** Needs the owner and reviewers. |
