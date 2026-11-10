# X3 Guard testnet proof — unsigned review packet

**Status: NOT READY.** Read-only preparation only; no deployment, signing,
funding, provider deposit, transaction, or owner authorization is recorded by
this document. This packet proposes a fresh-state **NEAR testnet** demonstration.
It is not an instruction to deploy and does not itself authorize any action.

## Candidate identity

- Historical source candidate: X3Lang PR #170 head
  `35152b85f12b657f3e8918c999e205e71d8a9668`. PR #170 merged to master at
  `8dc617cd556e152dd283dfc51a75b95ca7cbdc97`. The candidate build below
  is pinned to the historical PR head. Repeat the checks for any new source.
- Two CI jobs at that head logged an unsigned Guard WASM with SHA-256
  `ae7cac5962fe4bba3e527e466e7671a587c4899488e2035e8411a80a3905d302`,
  551,302 bytes, and an `env.panic_utf8` import:
  [preflight build](https://github.com/Cyptopimpinainteazy/x3lang/actions/runs/42664246712)
  and [Sandbox build](https://github.com/Cyptopimpinainteazy/x3lang/actions/runs/42664246585).
  These are CI log observations; those runs did not retain a deployable
  artifact. They do not prove a locally reproduced release or deployment.
- A later independent CI job in [PR #174](https://github.com/Cyptopimpinainteazy/x3lang/pull/174)
  checked out the pinned candidate, tested and built it, checked that hash,
  byte length, and panic import, and retained an unsigned [artifact](https://github.com/Cyptopimpinainteazy/x3lang/actions/runs/42683867418/artifacts/13703488647)
  with provenance ([successful run](https://github.com/Cyptopimpinainteazy/x3lang/actions/runs/42683867418)).
  GitHub records the archive SHA-256 `46cf82ab89124cdba7020e470795b8e419f85bf854854da0d48bc543739d1ac8`
  and expiry 2026-11-23. This packet has not downloaded or independently
  checked the archive; retention is not deployment approval or on-chain proof.
- Historical deployed code has a different hash; do not alter its evidence or
  represent this candidate as that deployed binary.
- The local Sandbox run is test evidence, not a public-chain transaction.

## Fields that must be resolved before a deployment decision

| Item | Current value |
| --- | --- |
| Explicit fresh contract account and owner | **Unselected; ownership and key control unverified** |
| Current account code, balance, state and storage | **Not observed for a selected account** |
| Local artifact path, SHA-256 and byte size | **Not built or verified in this packet** |
| Target network and genesis | NEAR testnet; preflight pins `FWJ9kR6KFWoyMoNjpLXXGHeuiy7tEY6GmoFeCA5yuc6b` |
| Initialization parameters and manager key | **Not reviewed for this candidate deployment** |
| Owner-authorized gas, storage and loss ceilings | **Not specified** |
| Fresh, provider-signed allowed and prohibited live quotes | **Not obtained; availability unknown** |
| Agent, recipient, mandate and exact allowed/denied calls | **Not selected or reviewed** |
| Independent security review and recovery plan | **Outstanding** |
| Exact deployment and test transaction authorization | **Not recorded** |

Prefer a fresh-state account subject to review; do not treat this as approval
to create or fund it. An in-place upgrade has a replay-ledger migration
constraint: historical accepted quote hashes are not automatically backfilled,
and the fresh-ledger marker makes uninitialized upgraded state fail closed.
See [the replay boundary](../security/GUARD_QUOTE_HASH_REPLAY.md).

## Read-only preparation, after selecting an account

On an isolated checkout of the final reviewed revision, with the pinned Rust
toolchain installed, build and test the contract from its own directory:

```bash
git rev-parse HEAD
git status --short
(cd x3-lang/near/x3-guard && cargo test --locked && cargo build --locked --release --target wasm32-unknown-unknown)
sha256sum x3-lang/near/x3-guard/target/wasm32-unknown-unknown/release/x3_guard.wasm
python3 scripts/hackathon/check_guard_wasm_host_panics.py --expect present x3-lang/near/x3-guard/target/wasm32-unknown-unknown/release/x3_guard.wasm
```

Compare the hash with the candidate record only when the source revision and
toolchain match. A mismatch blocks promotion until explained. Then, for an
owner-selected testnet account and new report path, run the existing read-only
preflight:

```bash
python3 scripts/hackathon/guard_readonly_deployment_preflight.py \
  --account '<selected-account>.testnet' \
  --wasm x3-lang/near/x3-guard/target/wasm32-unknown-unknown/release/x3_guard.wasm \
  --report /tmp/x3-guard-candidate-preflight.json
```

The preflight rejects accounts with existing code and compares public RPC
observations; its code-storage estimate is only a lower bound. It does not
verify ownership, key control, available gas, initialization, migration, the
full storage requirement or consensus finality. Exit zero is not deploy approval.
Read [the historical preflight and its limitations](GUARD_TESTNET_DEPLOYMENT_READINESS.md).

## Proof design and evidence needed

1. Obtain two fresh official manager-signed **live** quotes for reviewed,
   nonfunded test scenarios, if the provider supports both recipients. Record
   signed bytes, issuance, expiry, network, signer key, and exact terms; verify
   the signature before evaluating policy. Do not deposit.
2. On a separately authorized testnet deployment, establish a mandate and
   registered request. Capture the successful allowed `agent_accept_quote`
   and the refusal of a valid-signature quote with a prohibited recipient.
   Depending on the registered policy and signed fields the relevant failure
   can be `quote_request_echo`; confirm the actual check rather than assuming
   `mandate_recipient`.
3. For each case retain finalized transaction and receipt IDs, block identity,
   exact code hash, signed-quote hash, relevant policy/intent state before and
   after, and balance/spend/accepted-quote assertions. Compare public RPCs and
   label their trust limitations. A generic RPC error or trap alone is not a
   demonstrated policy refusal.
4. Verify readable `X3_QUOTE_REJECTED` or other actual reason in the receipt
   using the new code; if absent, report the receipt and separately observed
   view honestly. Never infer settlement from quote acceptance.
5. If the provider cannot issue a prohibited live quote, record that as blocked.
   Do not substitute a dry quote, expired quote or test-key signature.

Existing proof is described in [Guard testnet evidence](GUARD_TESTNET_EVIDENCE.md)
and [the local test-key signed-quote demonstration](X3_SHIELD_SIGNED_QUOTE_REFUSAL_DEMO.md).
Do not equate the two. The testnet demo does not constrain an unrestricted
wallet, prove a funded transfer, settlement, or cross-chain causality.
