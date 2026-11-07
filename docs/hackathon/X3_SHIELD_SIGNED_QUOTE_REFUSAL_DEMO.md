# X3 Shield — Signed quote passes cryptography, fails owner policy (LOCAL TEST)

**Security truth:** This is an actual Rust NEAR Guard contract method executed against near-sdk's **mock blockchain** using a deliberately generated **test-only Ed25519 signing key**. It is **not a deployed NEAR testnet transaction**, not the official 1Click manager's live signature, not an approved wallet deposit and not trusted consensus finality. The separate genuine 1Click captured quotes in `x3-lang/near/x3-guard/src/tests.rs` are signed **dry** quotes: they authenticate provider quote data, but are not live deposit authorizations.

## The judge-critical demonstration

The existing local test `valid_test_signature_with_forbidden_recipient_never_authorizes_a_deposit` in `x3-lang/near/x3-guard/src/mandate_tests.rs` executes the **real Guard method** `agent_accept_quote`, not only the standalone policy evaluator:

1. Owner creates a 50-USDC per-payment mandate for an agent with one allowed recipient; agent registers a matching 25-USDC intent.
2. Test builds a new quote whose signed `recipient` is **`attacker.near`**, signs it with the repository's **test-only** key, and proves the exact signed bytes verify under the local test contract's configured key.
3. Guard's `check_agent_quote` returns `quote_signature = true` but refuses `quote_request_echo` because the signed recipient disagrees with the registered policy.
4. A call to **`agent_accept_quote`** panics with `quote rejected by quote_request_echo; do not deposit`. The rejected nonce remains unaccepted, with no accepted deposit-address state, no spent allowance and no `quote_accepted` log.
5. Positive control: another nonce using the same owner mandate and a separately signed quote for the allowed recipient is **accepted**, records its quote and accounts for its 25-USDC spend.

The positive and negative outcomes share the same cryptographic mechanism and test signer. That isolates the **policy decision after cryptographic verification** rather than confusing invalid signatures with policy refusals.

```bash
(cd x3-lang/near/x3-guard && cargo test --locked valid_test_signature_with_forbidden_recipient_never_authorizes_a_deposit -- --nocapture)
(cd x3-lang/near/x3-guard && cargo test --locked)
node --test x3-lang/near/demo/test/*.test.mjs
```

## See the evidence in the actual browser console

```bash
python3 x3-lang/near/demo/judge/collect_evidence.py --with-contract
(cd x3-lang/near/demo && python3 -m http.server 8000)
# then open http://localhost:8000/judge.html and scroll to Panel 5
```

**Requires Python 3.11+, pytest, Node.js and Cargo installed.** The collector reruns all supported local checks, including the dedicated Guard contract-level refusal test, records their exit codes and pass counts in **untracked local** `x3-lang/near/demo/judge/evidence.json`, and shows a separately labeled **LOCAL TEST** proof row in the browser. It records the tested Git commit and flags a dirty source checkout. A missing or failed focused test is shown as **not proven**, never silently upgraded to green. Never publish `judge/evidence.json` without review: it can contain local file paths and environment details. No blockchain RPC or testnet wallet actions are required for this demonstration.

## Exactly what is still missing

- Owner-authorized testnet deployment of the reviewed code under an independently checked account/code hash and adequate storage funds. Read-only preflight on `x3lang.testnet` found the **545,640-byte** WASM's code alone needs **5.4564 NEAR**, against the observed **2 NEAR** balance; see [Guard deployment readiness](GUARD_TESTNET_DEPLOYMENT_READINESS.md).
- A fresh, authentic, official 1Click **manager-signed** quote whose exact signed terms violate a registered mandate. A test-only signature does not satisfy this, and an unaffiliated testnet sandbox cannot be presumed to use the official pinned key.
- A real Guard NEAR testnet transaction with a failure receipt and unchanged contract state. A reverted transaction may **not** emit a persistent rejection event; do not invent a `quote_denied` event.
- An independently authenticated NEAR light-client trust anchor and verified Guard receipt, rather than public-RPC agreement alone.
- A wallet path that obeys Guard decisions; an unrestricted owner wallet can bypass this read-only/authorization contract. Do not claim universal prevention of wallet transfers.

For now the strongest supported statement is: **"Our local NEAR Guard contract refuses a correctly TEST-signed 1Click-format quote after validating its signature because the recipient violates the owner's policy, while allowing an otherwise matching quote. No funds are moved."**
