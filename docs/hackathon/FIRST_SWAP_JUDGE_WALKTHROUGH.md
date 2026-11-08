# X3Lang Shield — First verified two-chain testnet execution

**Evidence class:** real Base Sepolia transfer + real finalized NEAR testnet delivery, verified independently from public-chain RPC; **NOT a trustless atomic bridge or native X3VM bytecode execution**.

**Scope:** owner-bound X3Lang Python frontend policy, deployed NEAR testnet Tidepool Guard, authenticated maker quote, Base Sepolia ERC-20 transfer, Tidepool community testnet relay, finalized NEAR receipt, independently verified pre/post NEP-141 balances.

**Release posture:** this repository remains private until the owner approves hackathon disclosure.

**Historical evidence:** never reuse the expired deposit address, nonce or quote.

## What judges should see (90–120-second sequence)

**00:00–00:15 | The programmable strategy**

Show [the actual `.x3` strategy](../../x3-lang/examples/base_sepolia_to_near_testnet_1usdc.x3). It specifies exactly 1 test USDC from Base Sepolia, `x3lang.testnet` as recipient, a minimum token output, max 1% slippage, a one-use nonce and refund handling. The *Python X3Lang frontend* parsed and typechecked this real source and produced the locked policy commitment SHA-256:

`1398f64e7e208d3796883aa59cb169187c1042ac1af3781bd6efc014755df860`.

Do **not** call this proof that the native Rust X3VM compiled or executed the program's bytecode on-chain.

**00:15–00:35 | Owner-bound, real on-chain authorization**

Show the separately deployed NEAR testnet Guard `x3guardlaunch2026.testnet`; both NEAR RPCs confirmed the on-chain WASM SHA-256:
`1d7a8b1ee97b7cdc500db1c19b8f3a2bef06e986576ec428f62f8c3ea1b5a145`.

The owner registered `x3_base_testnet_trade_001` and authorized the **genuine Tidepool maker-signed quote**, hash:
`131f314c2532965f2678e79f3f7cc87cfe8238a549da1405e802f4c90db1034c`.

The strict verifier recovered a historical finalized NEAR block containing the
code hash, quote authorization, owner, nonce and exact program. They were on-chain
at **2026-11-07 06:32:56 UTC**, before the Base transaction entered its block at
06:35:02 UTC. Approval block **276595150**, hash
`sPCSCCtVtPE3d4JGvno83zNoK41K5Ns51xspSx3fH1x`.

**00:35–00:55 | Real source-chain execution, with FINALIZED head**

Show Base Sepolia transaction:
https://sepolia.basescan.org/tx/0x46c21dc305a9271c4e74c6d0734fb50f847a1e53d3cad87976133d66fd6728d9

Two independently operated public RPCs agreed on the transaction, successful receipt, exact ERC-20 `Transfer` event, **1,000,000 test-USDC base units**, signed destination deposit address, token bytecode SHA-256, canonical inclusion block **55,557,170**, and an L2 `finalized` head of **55,557,819** or later (as recorded by the immutable proof capture). The source proof requires a minimum of 12 confirmations and real finality; `safe` alone does not pass.

**00:55–01:15 | Real NEAR settlement receipt**

Show the genuine finalized NEAR testnet transaction:
https://testnet.nearblocks.io/txns/EBEBxJNgV9U5tBFBLj18ZzukgArqwXH5rBsBStFyCm1d

Independent NEAR RPC evidence shows `FINAL` execution and **six successful receipts**. The verifier's `ft_withdraw` and token contract's actual `ft_transfer` credited **997,400 base units = 0.997400 test USDC** to **`x3lang.testnet`**, above the maker-signed minimum of **987,426 units**.

Canonical finalized recipient balance at block 276593025 was **100.000000 test USDC**;
by block 276596190 it was **100.997400 test USDC**. The token transfer receipt was
included at block **276595461**, inside the snapshot interval.

**01:15–01:35 | Demonstrate the hard denials**

Show the real local NEAR Sandbox WASM tests (test code is on PRs #94, #98, #101, #105, not all merged): tampered genuine manager signature, altered payload, expired/revoked mandate, unauthorized recipient, repeated approval, competing nonce submissions and cross-owner namespace isolation. Each denial must leave the on-chain test state unchanged where required. The real Sandbox suite is a **local adversarial test**, separate from this public testnet swap.

**01:35–01:45 | The trust boundary and next milestone**

Say clearly: *"We demonstrated a genuine, independently verified two-chain testnet execution using an owner-authorized `.x3` policy and a trusted Tidepool testnet relayer. This is not our native permissionless BSC/NEAR bridge, not atomic-HTLC proof, and not mainnet authorization."*

## Evidence files (exact bytes, no private keys)

| File | Description | SHA-256 |
|---|---|---|
| [Historical signed quote](../../evidence/tidepool-testnet/first-swap-signed-live-quote-20261107.json) | Authentic expired Tidepool maker-signed live response, captured before funding. The original classification is historical, not the current trade status. | `7718da27680bbb35cbf199caaae06ee77987dd5daf60f2dfd1ca175a08da7535` |
| [Finalized pretrade baseline](../../evidence/tidepool-testnet/pretrade-finalized-baseline-20261107.json) | Two-chain independent baseline before this swap. | `69a835814805876acd2eafb1ca0390081e6bf4d4fca6bcc4c5347dbb609c4120` |
| [Strict successful two-chain proof](../../evidence/tidepool-testnet/first-swap-strict-two-chain-proof-20261107.json) | Actual read-only chain verification output with source RPC quorum, historical Guard approval, finalized NEAR receipt and canonical balance change. | `9aa6d251b7e3c8f8cb6fee57e466034d3f747a84b3792233fe36cd97570bbbcc` |

The maker-signed quote payload SHA-256 (`131f314c…1034c`) and archived JSON file SHA-256 (`7718da2…7535`) are **different digests of different bytes**. Do not mix them.

## Reproduction

```bash
# From repo root; verifies the saved real-world evidence and cryptographic quote.
PYTHONPATH=x3-lang python3 -m unittest \
  scripts/hackathon/test_first_swap_proof_bundle.py \
  scripts/hackathon/test_two_chain_settlement_proof.py \
  scripts/hackathon/test_two_chain_baseline.py -v

# Re-run the STRICT independent live chain verifier to create a new,
# non-overwriting file. Never feed it an invented tx, signer or block hash.
PYTHONPATH=x3-lang python3 scripts/hackathon/two_chain_settlement_proof.py \
  --quote-file evidence/tidepool-testnet/first-swap-signed-live-quote-20261107.json \
  --guard-account x3guardlaunch2026.testnet \
  --guard-wasm-sha256 1d7a8b1ee97b7cdc500db1c19b8f3a2bef06e986576ec428f62f8c3ea1b5a145 \
  --nonce x3_base_testnet_trade_001 \
  --program-sha256 1398f64e7e208d3796883aa59cb169187c1042ac1af3781bd6efc014755df860 \
  --guard-approval-near-block sPCSCCtVtPE3d4JGvno83zNoK41K5Ns51xspSx3fH1x \
  --base-tx-hash 0x46c21dc305a9271c4e74c6d0734fb50f847a1e53d3cad87976133d66fd6728d9 \
  --near-tx-hash EBEBxJNgV9U5tBFBLj18ZzukgArqwXH5rBsBStFyCm1d \
  --near-tx-signer maker.tidepool-sandbox.testnet \
  --near-balance-before-block A5dpq1mU5qPY5Mo71aUzkG97atz8yQ4V7grHAkxydCvU \
  --near-balance-after-block ujr51FjdXqTKDUENrkaMn2679EHF19sS17o1imns4Q3 \
  --output /tmp/x3lang-first-swap-independent-rerun-unique.json
```

Historical provider status may describe `SUCCESS`, but only the public-chain observations above qualify as chain evidence. Causality linking these chains depends on the **trusted Tidepool relay**; there is no independently verified *cryptographic atomic bridge proof*. No live token contracts or custody assurances are implied for mainnet.

## Separate adversarial Guard evidence (local, not the Tidepool on-chain contract)

The [recorded local NEAR Sandbox + Rust mutation results](../../evidence/tidepool-testnet/local-guard-adversarial-review-20261107.json) have SHA-256 `2298a4e0f69a398f783e295d1fbcb4d9096a3c7f2047e77289b61ccccf68e3ac` and correspond to head `426a953c4824eedf5208e351eb869f4e80b8469b` (stacked draft PR #105). All operations use disposable local accounts and actual locally deployed `x3-lang/near/x3-guard` WASM. The Sandbox run exited 0 and produced explicit acceptance/refusal markers for: genuine archived signed dry quote rejection, tampered genuine signature rejection, unauthorized recipient, revoked/expired mandates, competing nonce submissions with exactly one winner, four repeated nonce races, duplicate/unauthorized approval replay, and two-owner nonce namespace isolation. The separate mutation run exited 0 and killed **6 of 6 curated source mutants** (signature, nonce replay, duplicate approval, expiry, recipient and cap) with **0** survived, invalid or timed-out. This is **100% of six specific mutants**, not general security coverage.

**Separately, the Tidepool-specific on-chain Guard Rust crate** `x3-lang/near/x3-tidepool-guard` was re-tested at this review checkout: **13/13 tests passed** (local Rust host testing) including nonce reuse, unauthorized owner, revoked policy, wrong program, malformed signature and storage funding. These are contract unit tests, not a fresh public-NEAR attack transaction.

**Non-equivalence:** The local Sandbox proof uses the production `x3-guard` Rust code from the security PR, **not** the separately deployed `x3-tidepool-testnet-guard` that authorized this swap. It is **not** additional public-NEAR testnet transaction evidence. Those implementations have different keys and are subject to separate review/deployment gates.

## How X3 differs from the native bridge work

The Tidepool execution validates the **X3 policy/Guard/quote/evidence workflow**. The separately developed native BNB↔NEAR LP, HTLC, and attested-escrow prototypes have their own unresolved trust, verifier, inventory, audit and mainnet blockers; this proof is **not** evidence that those prototypes settled funds. Keep the demo labels and feature maturity separate.
