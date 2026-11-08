# Issue #141 — vault-origin two-chain verifier handoff (review-only)

This package is a **code review candidate**, not an approval to move funds. The
only positive Base and NEAR cases in its unit tests use **synthetic RPC data**.
No public Base vault payment or linked NEAR settlement was verified in this run.

## Security improvements over the earlier package

1. The original 31 offline tests were reproduced. Four more adversarial test
   methods were added. The completed suite has **35 passing offline tests**.
2. After proving a historical Guard approval, the destination verifier now
   requires `guard_pre_source_block_hash` and verifies that it refers to the
   **last canonical NEAR block before the Base transaction's inclusion
   second**. It verifies the immediate next block is later, and rechecks
   contract code and owner/nonce/policy *at that pre-payment block*. A policy
   revoked or changed after the historical approval is refused. This is a
   two-RPC historical-state check, **not** validator-signature light-client
   proof or synchronous cross-chain consensus.
3. The Base and NEAR reports now include provider **hostnames**. The verifier
   refuses HTTPS RPC URLs containing username/password, query strings or
   fragments. These are not release signatures or authenticated RPC proof.

## Safely stage in a private X3Lang feature branch

The intended base is the existing **private** X3Lang PR #142, stacked on
#140 and #138. Before applying, **check live exact head**: another agent may
have advanced any branch. Do not reset or force-push their work.

1. Review `AGENTS.md`, `README.md`, and `PRIOR_WORK.md` from your real checkout.
2. Work on **one new isolated worktree/branch** based on the then-current
   reviewed #142 head, and verify the worktree is clean.
3. Copy these new files into the same repository-relative paths from the ZIP.
   Do not overwrite existing source files without an explicit diff review.
4. Verify integrity from repository root:

   `sha256sum -c docs/security/ISSUE141_PATCH_SHA256SUMS.txt`

5. Offline unit test (no network):

   `PYTHONPATH=scripts/hackathon python3 -m unittest -q scripts.hackathon.test_vault_origin_base_proof scripts.hackathon.test_near_delivery_receipt_proof scripts.hackathon.test_vault_two_chain_pairing scripts.hackathon.test_vault_two_chain_cli`

   `python3 scripts/hackathon/keccak_min.py`

   `python3 -m py_compile scripts/hackathon/{keccak_min,near_delivery_receipt_proof,vault_origin_base_proof,vault_origin_two_chain_cli,vault_two_chain_pairing}.py`

6. Validate the new workflow YAML and exact PR SHA in GitHub Actions. **Do not
   claim GitHub CI was run by this review package.** The private xxxstar mirror
   gate is a separate, mandatory security requirement; its matching review
   branch requires explicit project authorization, not bypass.
7. Open a **draft PR** and request independent security review. Do not merge
   custody, signing or settlement code without owner approval and release gates.

## Real-chain evidence that does not yet exist

- A reviewed, reproducibly built and independently audited **deployed** vault
  runtime SHA bound to the exact Solidity source/release.
- A fresh Tidepool maker-Ed25519-signed quote; the reviewed Guard policy and
  quote receipt at a new owner-approved nonce; actual owner/signer separation.
- An authenticated Base 84532 **vault-origin** `executePayment` transaction,
  `PaymentExecuted` log, ERC-20 `Transfer` from vault to the signed deposit,
  canonical finalized block and an independent recipient/source balance proof.
- A finalized NEAR testnet `ft_transfer` receipt and actual minimum destination
  `ft_balance_of` delta, pre/post canonical blocks, and Guard state still
  active immediately before source payment.
- Approved wallet, code hash, gas/slippage/loss limits and fresh contract
  deployment/funding/signing transactions. **None were authorized by this
  package or performed by this run.**

The current `base_sepolia_x3_intent.compile_testnet_intent()` in PR #140/142
uses the historical nonce `x3_base_testnet_trade_001`. The combined CLI
correctly refuses that nonce as reused. A new reviewed, compiled `.x3` source
with a fresh nonce and corresponding Guard approval is therefore required
before any real certificate can pass. **Do not disable this gate for the demo.**

The existing previously verified Base→NEAR transfer belongs to the older EOA
path. The unit test references its real transaction hash but uses synthetic
RPC fields to verify classification; this is **not** a new independent RPC
replay of the actual historical receipt.

A user-supplied `vault_runtime_sha256` is not independently authenticated.
Confirm it from reproducible build artifacts plus approved source and
independent review before relying on any future vault-origin evidence.

## Evidence meanings

- `PASS 35 offline synthetic tests` = deterministic *code/test* verification.
- A future Base two-RPC response = reported finalized **Base** source facts.
- A future NEAR two-RPC response = reported finalized **NEAR** payout facts.
- Matching two chains' reported transactions **does not cryptographically prove
  Tidepool relayer causality or atomic cross-chain settlement**.
