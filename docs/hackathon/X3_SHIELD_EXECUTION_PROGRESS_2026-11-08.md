# X3 Shield execution ledger — 2026-11-08

Plan: [X3_SHIELD_EXECUTION_PLAN.md](X3_SHIELD_EXECUTION_PLAN.md).
Tracking: draft PR [#159](https://github.com/Cyptopimpinainteazy/x3lang/pull/159),
related to issue [#119](https://github.com/Cyptopimpinainteazy/x3lang/issues/119).
Baseline: `038e82d73a522642ae755af48eb6acb838902329`.

## Current status

Task 1 now has a passing local Sandbox run, not merely a compile check:

- The FunctionCall-only agent key is rejected with the exact structured node
  error `RequiresFullAccess` when it attempts a direct NEAR transfer.
- Calling `recover_available` with that key is rejected with the exact structured
  node error `MethodNameMismatch:recover_available`.
- Included contract failures are matched to the intended stable
  `X3_VAULT_DENIED` reason through a typed `ActionError` /
  `FunctionCallError::ExecutionError`, not arbitrary failure text.
- A permitted payment succeeds and remains terminal after replay and revocation
  refusals.
- Owner recovery succeeds only after revocation and changes both the vault ledger
  and the observed owner/vault account balances.

The run used an ephemeral local NEAR Sandbox 2.14.0 node and an ephemeral agent
key. It used no public RPC, wallet, account, token or funds. This is local
custody-harness evidence, not a public-testnet deployment.

Machine-readable evidence:
[typed-refusal-sandbox-20261108.json](../../evidence/agent-vault-local/typed-refusal-sandbox-20261108.json).
The sanitized 13-line runner output is
[typed-refusal-sandbox-20261108.txt](../../evidence/agent-vault-local/typed-refusal-sandbox-20261108.txt).

## Exact verified behavior

| Observation | Required result |
| --- | --- |
| Scoped key attempts direct transfer | `InvalidAccessKeyError: RequiresFullAccess`; attacker balance unchanged |
| Wrong recipient | `X3_VAULT_DENIED: recipient not allowlisted` |
| Amount above per-payment cap | `X3_VAULT_DENIED: per-payment cap exceeded` |
| Both denied requests | escrow remains exactly `1.0` NEAR |
| Allowed payment | exactly `0.1` NEAR; payment state becomes `Paid` |
| Replay of used nonce | `X3_VAULT_DENIED: nonce already used` |
| Recovery while mandate is active | `X3_VAULT_DENIED: revoke or expire mandate before recovery` |
| Payment after owner revocation | `X3_VAULT_DENIED: owner revoked mandate` |
| Original payment after later refusals | remains `Paid` |
| Scoped key attempts owner recovery | `InvalidAccessKeyError: MethodNameMismatch:recover_available` |
| Owner recovery after revocation | `0.5` NEAR recovered; vault escrow becomes `0.4` NEAR |
| Owner and vault balances | measured before/after; vault decreases and owner net increases after gas |

The final runner marker is:

```text
X3_VAULT_SANDBOX_COMPLETE agent_key_restricted=true funds_vault_custodied=true public_chain=false
```

## Verification actually performed

The exact compiled contract WASM used by the successful run had SHA-256:

```text
9a6d8932d11094bb99f683f9c2b52c7c790a1bb0b8db55354397b70e79cc3a9c
```

| Command | Result |
| --- | --- |
| `RUSTUP_TOOLCHAIN=1.95.0 RUSTFLAGS='--cfg near' cargo build --locked --release --target wasm32-unknown-unknown` | PASS |
| `RUSTUP_TOOLCHAIN=1.90.0 cargo check --all-targets --locked` | PASS |
| `RUSTUP_TOOLCHAIN=1.90.0 cargo test --locked` | PASS, 4 focused classifier tests |
| `RUSTUP_TOOLCHAIN=1.90.0 cargo clippy --all-targets --locked -- -D warnings` | PASS |
| `python3 scripts/hackathon/test_vault_refusal_source_guard.py` | PASS, 8 source guards |
| `python3 -m py_compile scripts/hackathon/test_vault_refusal_source_guard.py` | PASS |
| Workflow YAML parse | PASS |
| Real local NEAR Sandbox lifecycle on the compiled WASM | PASS, exit 0 |

The CI workflow now adds a `sandbox-lifecycle` job that downloads the pinned
Sandbox 2.14.0 binary, verifies its SHA-256, runs the same compiled-WASM harness,
checks the required success markers, and uploads the sanitized local log.

## Implementation boundary

No vault contract logic, state layout, dependency lockfile outside the Sandbox
test harness, immutable chain evidence or historical provenance changed in this
increment. The harness adds:

- a raw JSON-RPC path because `near-api` 0.8.6 collapses access-key subtypes to
  `InvalidTransaction({})`;
- typed matching of access-key and contract execution errors;
- balance/state controls around the tested refusals;
- focused unit tests proving empty, generic and wrong variants fail closed.

No unknown, transport, timeout, malformed, nonce, gas or wrong-network error may
be treated as a successful policy refusal.

## Remaining work

1. Bind a supported `.x3` mandate source to generated vault configuration without
   a second parser or manually substituted constructor values.
2. Add expiry and attached-deposit cases to the real Sandbox run. The current run
   already proves a fresh post-revocation nonce is refused and the original
   terminal payment is preserved.
3. Pass exact-head GitHub CI, including the new Sandbox lifecycle job.
4. Complete independent review and owner approval.
5. Reconcile the final README/demo claims only after exact-head CI and independent
   review pass.

The non-bypassable custody claim still excludes an unrestricted wallet keyholder,
public deployment, audited key custody, state migration and production storage
economics.

## Non-actions

No merge, public deployment, funding, real wallet signing, public disclosure,
mainnet action or historical evidence rewrite was performed. This draft remains
reviewable evidence, not production custody approval.
