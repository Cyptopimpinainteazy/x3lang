# X3 Guard: real-node adversarial tests and Rust source mutation gate

This is a **test-only addition** stacked on the separate expired-mandate approval fix in PR #94. No runtime authorization rules, contract source, wallets, testnet deployments, or existing proof gates are weakened.

## Local NEAR Sandbox execution (real WASM, ephemeral accounts)

Run from the repository root:

```bash
bash scripts/hackathon/guard_sandbox_smoke.sh
```

The harness builds production Guard WASM, starts an isolated real local NEAR Sandbox node, deploys and initializes that exact WASM, and checks:

- Archived **real 1Click-manager-signed dry quote** verifies cryptographically but cannot authorize a deposit; an altered signed payload or changed signature is separately rejected at `quote_signature`. Both state-changing rejection receipts must fail, and the stored `quote` remains null.
- Two different owner-authorized ephemeral **agents simultaneously** submit the **same owner and nonce**; require **exactly one** success and verify the stored identity matches its winner. Distinct signer accounts avoid conflating account access-key nonce conflicts with Guard's owner/nonce uniqueness.
- A named approver signs once, then submits a duplicate. The duplicate and unauthorized agent approval must both fail; the stored list must contain exactly one approver.
- Previous PR #94's test proves expiry against a deployed WASM, using the contract's observed block timestamp, rather than assuming a specific block count equals wall-clock time.

The race checks a single pair of concurrent submissions in an isolated chain. It is a reproducible regression, **not** proof that every race, fork, or production RPC failure has been excluded. No funds, deposits, public chain signing or cross-chain settlement are exercised.

**Harness reliability finding:** A repeat run initially failed during a local account's access-key lookup before the contract tests started. The Sandbox account creator uses state patches, whose account/key visibility is not guaranteed to be instantaneous. The harness now polls the **actual read-only access-key RPC** in a bounded fashion before using each ephemeral account. Any absent key remains a hard failure; no fake successful transaction is substituted. The first run after this change completed with exit 0 and the full scenario matrix. CI must still establish fresh-run repeatability.

## Actual Rust source mutants

Run from the repository root with pinned Rust toolchain and cached Guard Cargo dependencies:

```bash
cargo fetch --locked --manifest-path x3-lang/near/x3-guard/Cargo.toml
python3 scripts/security/guard_rust_mutations.py \
  --output /tmp/x3-guard-rust-mutants.json --timeout 180
```

The runner creates a disposable copy of `x3-lang/near/x3-guard`, includes its actual `include_str!` test fixtures, and runs each focused Rust test against both the unmodified baseline and one uniquely matching source mutation.

| Mutation | Focused test |
|---|---|
| Signature validation bypass | `tampering_with_any_signed_byte_fails_verification` |
| Owner/nonce reuse bypass | `a_nonce_registers_once` |
| Duplicate approval bypass | `payments_above_the_threshold_need_n_of_m_approvals` |
| Expired approval bypass | `expired_mandate_cannot_record_late_approvals` |
| Recipient allowlist bypass | `every_term_refuses_an_intent_that_breaks_it` |
| Per-payment cap bypass | `every_term_refuses_an_intent_that_breaks_it` |
| Contract-wide signed-quote-hash replay bypass | `one_signed_quote_cannot_authorize_two_distinct_intents` |
| Missing legacy-upgrade replay ledger check | `existing_state_without_a_fresh_quote_ledger_marker_refuses_new_quotes` |

**KILLED** requires a real Rust assertion/test failure. An import/compile error, timeout, bad source-match count, or red baseline fails closed and cannot count as a killed mutant. **SURVIVED** means the original test still passed against the changed source; any survivor blocks the mutation gate. Each report includes exact HEAD, original/mutant SHA-256, command, test and failure details.

### Local evidence from first run

On source base commit `6bf0cb80580e44b1f9eb2b763d038a40d1b89631`, the first local mutation run on x3star1 recorded **6 killed / 0 survived / 0 invalid / 0 timeouts**. The first Sandbox run added the concurrent submissions, two signature negatives and approval replay and finished **exit 0**. The GitHub Actions workflow must independently rerun these at the exact proposed PR HEAD before the suite can be called CI-green.

A subsequent local run with the contract-wide quote-consumption patch added the seventh replay-bypass mutation (**7/7 killed**). A further independent local run added an eighth mutation disabling the new legacy-upgrade marker check: **8 killed / 0 survived / 0 invalid / 0 timeouts**, with seven distinct focused baselines passing. These measurements concern only curated native Rust source changes; exact-head GitHub CI on the proposed PR is still required.

## Hard release boundaries

- Native Rust tests use NEAR SDK simulated context; this is **not** node-level evidence. Local NEAR Sandbox tests provide actual node execution, **not public testnet or authenticated finality**.
- The Sandbox fixture's authentic 1Click-manager signature proves signed quote terms only; its archived dry quote neither authorizes a deposit nor demonstrates settlement.
- The patched WASM bytes differ from the public testnet deployment; source mutation test success does **not** update production code identity.
- PR #94 and this stacked test PR must remain **draft** pending independent review and existing exact-head security gates, including private `xxxstar` peer checkout and deployed-byte proof. No mainnet assertions or unattended wallet operations.
