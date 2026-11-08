# X3 Guard: eight-agent nonce-contention stress

## Scope

This is an additive, test-only extension of the actual local NEAR Sandbox WASM harness in `x3-lang/near/guard-sandbox-tests/src/main.rs`. No signing with real wallets, public RPC transactions, transfers, mainnet code changes or `xxxstar` modifications occur.

### Threat model and invariants

An owner permits eight independently addressed agents to register intents. In each of **four rounds**, all eight agents concurrently submit the **same `(owner,nonce)`** with a valid policy. The test requires **exactly one successful NEAR transaction** and seven rejected transactions per round. It then reads the contract's actual persisted intent and checks that the owner, nonce, recorded winner, empty approvals and absent accepted quote match the expected state.

After each round, the winning agent and a losing agent separately replay the same nonce. Both state-changing transactions must be rejected; a second read-only state query must equal the state recorded before those attempts. Thus the test exercises **32 contending submissions plus 8 separate replay attempts**.

Finally, a second owner uses the **identical nonce string** already used by the original owner. This registration must succeed under the new owner without altering the original owner's record; nonce uniqueness is *scoped to owner*, not globally.

The harness fails hard when any transaction cannot produce a receipt, when the required account key is not observable within a bounded period, or when any persisted-state invariant breaks. It does not treat RPC errors, missing accounts or slow node startup as successful security refusals.

## Reproduction

From the X3Lang repository root:

```bash
bash scripts/hackathon/guard_sandbox_smoke.sh
```

On Linux with an older glibc, the existing script uses a disconnected Ubuntu 24.04 container and a checked, pinned Sandbox binary. The new harness uses a pinned `futures` dependency (`0.3.34`) for concurrent submissions.

Successful completion emits:

```text
X3_SANDBOX_EIGHT_AGENT_RACES local_only=true agents=8 rounds=4 attempts=32 winners=4 replay_rejections=8 state_unchanged=true
X3_SANDBOX_CROSS_OWNER_NONCE_ISOLATED local_only=true owners=2 same_nonce=true separate_state=true
```

Local first execution of these cases on x3star1 completed with **exit 0** against a deployed local production Guard WASM based on PR #98. Its pinned WASM SHA was `e2877a68012ae6dfcd951619f03cbf473226a7c05e306cd6062b884103b069b0`. These results do **not** prove consistency under arbitrarily high concurrency, multi-shard reordering, hostile RPC behavior, public NEAR consensus finality, or cross-chain atomic settlement. Exact-head CI and independent review are required before this can be a release gate.

## Merge and release boundaries

This branch is stacked on PR #98 (which itself depends on security fix PR #94). Maintain the current fail-closed cross-repository mirror, exact deployed-WASM identity, security review, and no-wallet/no-deployment restrictions. No new production claim may be promoted solely by passing this stress test.
