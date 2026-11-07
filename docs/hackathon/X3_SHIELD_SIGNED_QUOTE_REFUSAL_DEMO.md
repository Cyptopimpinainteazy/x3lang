# X3 Shield — Signed quote passes cryptography, fails owner policy (LOCAL TEST)

**Security truth:** This section is an actual Rust NEAR Guard contract method
executed against near-sdk's **mock blockchain** using a deliberately generated
**test-only Ed25519 signing key**. It is not the NEAR testnet evidence and not
the official 1Click manager's prohibited-live-quote refusal. Separately, the
owner-authorized testnet run in
[`GUARD_TESTNET_EVIDENCE.md`](GUARD_TESTNET_EVIDENCE.md) deployed the reviewed
Guard, refused an official manager-signed **dry** quote after signature
verification due to its recipient mismatch, and accepted an official
manager-signed allowed **live** quote without depositing funds. No wallet
transfer, deposit, settlement, causality, or independent finality is claimed.

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

- A **live** official 1Click manager-signed quote whose exact signed terms
  violate a registered mandate and is refused by the deployed Guard's agent
  mandate path. The current official forbidden quote is dry and fails at
  recipient echo; the live allowed control is only a positive control.
- A wallet path that obeys Guard decisions and a source-chain deposit. The
  current live testnet acceptance stopped before deposit.
- An independently authenticated NEAR light-client trust anchor and verified Guard receipt, rather than public-RPC agreement alone.
- An observed destination receipt, settlement, and cross-chain causality. A
  signed quote and acceptance authorize terms; they do not execute or settle a
  swap. An unrestricted owner wallet can still bypass this authorization
  contract.

The strongest supported statement is: **"The local Guard refuses a correctly
TEST-signed 1Click-format quote after validating its signature when the
recipient violates policy. On NEAR testnet, the deployed Guard refuses an
official manager-signed dry quote after signature verification because its
recipient does not match the registered request, and accepts a separate
official live allowed quote without any deposit. No funds are moved, and a
live forbidden-quote refusal remains missing."**
