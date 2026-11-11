# Guard storage for restricted agent keys

This change is prepared for review. It is not a mainnet deployment or swap proof.

NEAR function-call keys cannot attach deposits. Previously every successful
agent write that grew storage required a deposit, so the deployed Guard could
not support the restricted-key mainnet workflow. The upgrade permits the same
three agent mutation methods to consume bounded storage credit prepaid by the
mandate owner. Quote, approval, policy, replay and spend checks are unchanged.

## Public API

| Method | Caller and arguments | Deposit and result |
| --- | --- | --- |
| `fund_agent_storage` | Owner; `{"agent":"agent.near"}`; requires that owner's active, unexpired mandate | Positive deposit, at most 1 NEAR. First funding pays the credit record's storage; the rest becomes credit. Returns decimal-string yoctoNEAR balance. Total unspent credit cannot exceed 1 NEAR. |
| `get_agent_storage_balance` | View; `{"owner":"owner.near","agent":"agent.near"}` | Returns decimal-string yoctoNEAR, zero if never funded. Fails `X3_STORAGE_WITHDRAWAL_PENDING` during withdrawal. |
| `withdraw_agent_storage` | Owner; `{"agent":"agent.near"}`; works after expiry/revocation | Zero deposit. Returns a promise transferring all unused credit to the owner. Its callback clears credit on success or restores it on failure. |
| `resolve_agent_storage_withdrawal` | Private receipt callback only | Reserved credit cannot be spent, topped up or withdrawn again while the transfer is pending. |

Zero-deposit `agent_register_intent`, `agent_accept_quote` and
`agent_record_outcome` charge net positive storage growth at the protocol's
storage-byte cost. Insufficient credit fails `X3_STORAGE_BUDGET_EXHAUSTED` and
the runtime rolls back the receipt. Existing calls with attached deposits
continue to pay their own storage and refund excess; they do not debit credit.
They may execute while a withdrawal is pending because they pay independently.
Approvals and mandate creation remain caller-funded owner operations.

Credit is isolated by owner and agent, persists across mandate replacement, and
is not replenished by refunds or storage shrinkage. An authorized agent can
consume its finite budget by registering many otherwise valid intents; the
payment cap only counts accepted quotes. Owners choose whether to top up again.
The fixed credit record remains allocated after withdrawal; its storage charge
is not returned. Gas comes from the agent account/key allowance separately.

## Explicit client storage mode

After reviewing the deployed Guard's prepaid-storage support and funding the
owner/agent storage credit, select `--prepaid-agent-storage` on
`near_intents.py prepare`, `near_intents.py guard-args`,
`near_agent.py register-args`, or `near_agent.py pay`. The generated JSON and
commands use `0 NEAR` for `agent_register_intent`, `agent_accept_quote`, and
`agent_record_outcome`. Live agent reports preserve the option in their next
commands. For the guided script, export `X3_AGENT_PREPAID_STORAGE=1` for each
`agent-demo.sh` invocation; its default is `0`, and other values are rejected.

This option selects deposits only. It does not verify deployed code, available
credit, key permissions, or authorization, and it never funds or raises a budget.
Insufficient credit still fails on chain. The default retains legacy attached
deposits. Owner registration, quote, outcome, approval, mandate and revoke
commands retain their existing deposits even when the option is selected.
The existing settlement evidence gates and owner confirmations still apply.

## Upgrade compatibility

The `Contract`, `Mandate`, intent layouts and map prefixes are unchanged.
Credits use a new `X3_AGENT_STORAGE_V1:<owner>/<agent>` raw-storage namespace,
with a fixed 17-byte record (little-endian u128 balance plus pending flag).
No migration or reinitialization is required. Never call `new` on an existing
deployed account. Prior WASM cannot manage these new credits: before any rollback,
stop agents and withdraw all unused balances, waiting for callbacks to finish.
An owner-authorized code upgrade changes the pinned code hash; all signing
preflight must review and pin the new exact binary before any swap action.

Do not remove an agent's old key until the upgrade and storage path have been
verified and the new restricted key has been installed and checked. Each
mainnet deployment, funding, key addition/removal and Guard action requires its
own exact-transaction owner approval. No mainnet action is part of local tests.

## Verification and limits

Run `cargo test --locked` from `x3-lang/near/x3-guard`, and
`bash scripts/hackathon/scoped_agent_sandbox_smoke.sh` from the repository root.
The Sandbox exercises the compiled production WASM with a real function-call
signer, storage debit, mandate refusal, withdrawal and failed-registration
rollback. Unit tests cover acceptance/outcome charging, credit isolation/caps,
and failed-transfer callback restoration. Mocked promise outcomes do not prove
real failed-transfer execution. The Sandbox does not claim accepted-quote
storage-exhaustion/replay-ledger rollback or a cross-chain settlement.

Dependency reference: `near-sdk 5.30.0`, pinned in Cargo.lock. Its
[generated self-call and private-method documentation](https://docs.rs/near-sdk/5.30.0/near_sdk/attr.near.html)
defines the callback boundary. The version-matched SDK source documents
`env::promise_result_checked`; the callback reads at most zero result bytes
because a native transfer's successful result is empty.
The [NEAR access-key specification](https://near.github.io/nearcore/DataStructures/AccessKey.html)
describes restricted permissions. Guard authorizes quote terms; it does not
custody swap assets or prevent a foreign-chain wallet keyholder bypassing it.

## Reproduce the proposed binary

With Rust 1.99.1, its `rust-src` component, the WASM target and locked
Cargo dependencies installed, run from `x3-lang/near/x3-guard`:

```bash
RUSTFLAGS="--cfg near --remap-path-prefix=$(rustc --print sysroot)/lib/rustlib/src/rust=/rustc/58cf8f9c35b355199727627d07ae34a4e2846d67 --remap-path-prefix=${CARGO_HOME:-$HOME/.cargo}/registry/src=/home/lojak/.cargo/registry/src" \
  cargo build --locked --release --target wasm32-unknown-unknown
```

The expected binary is 572248 bytes, SHA-256
`9a614640ac8bc39bbbcb1674ce0f4fe2f51610ac7ba828c9c4574be92712a632`,
NEAR code hash `BPdpJAw8R4SDh6Wi7dPcXTs5T7ncJDVKCLgzuqZFb15T`.
The local-node CI workflow compares its normalized rebuild against the
[reviewed verification manifest](../../audit-artifacts/guard-scoped-storage-20261110/verification.json).
A later intentional Guard change must regenerate the reviewed artifact and
verification manifest; a mismatching binary must never be deployed under this
review packet. This proposed hash does not identify an existing mainnet deployment.
