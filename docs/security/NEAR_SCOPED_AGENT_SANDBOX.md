# X3 Shield: local scoped-agent key authorization proof

**Scope: one NEAR account-access-key permission boundary, using a real local NEAR Sandbox node.** This is the first adversarial milestone of [Issue #119](https://github.com/Cyptopimpinainteazy/x3lang/issues/119), **not** the completed non-bypassable payment architecture.

## Run locally

On a prepared Linux development host, from the X3Lang repository root:

```bash
bash scripts/hackathon/scoped_agent_sandbox_smoke.sh
```

The script compiles the **actual x3-guard WASM** and the Rust `near-api` harness from the committed source. On Ubuntu 22.04/glibc 2.35 it runs the local node in the already-pinned, **network-disconnected** Ubuntu 24.04 image, using the SHA-256-pinned local NEAR Sandbox binary. No Testnet/Mainnet RPC connection, transaction, wallet, deposit or public chain deployment occurs. Missing cache/toolchain fails closed.

A disposable `agent.sandbox` account receives a freshly generated key with `AccessKeyPermission::FunctionCall`, receiver `guard.sandbox`, method list **exactly** `["state_version", "agent_register_intent", "agent_accept_quote", "agent_record_outcome"]`, and an explicit five-NEAR gas allowance. These values are re-read from the **local node's account state**. The agent's key performs a real zero-deposit function call to the deployed Guard and succeeds. It then attempts transactions **with the identical restricted key** using the raw SDK (not an X3 frontend):

| Attempt | Node-enforced expectation |
| --- | --- |
| `guard.sandbox::state_version`, 0 deposit | Succeeds |
| Direct transfer of 1 NEAR to `attacker.sandbox` | InvalidTransaction; no transfer |
| `guard.sandbox::get_manager_public_key` | InvalidTransaction; unapproved method |
| `attacker.sandbox::state_version` | InvalidTransaction; wrong receiver |
| Permitted method with 1 yoctoNEAR deposit | InvalidTransaction; FunctionCall keys cannot attach deposits |
| Repeat permitted method after owner deletes key | Access-key lookup fails; client cannot sign after node confirms key absent |

After key revocation, the SDK cannot prepare another signed transaction: the local node reports that the access key does not exist. That is **not** counted as a broadcast InvalidTransaction.

After the forbidden transfers, the recipient's local chain balance is queried again and **must be unchanged**. A generic timeout or network error must never count as a permission denial. `near-api 0.8.6` surfaces the local node error as `ServerError(InvalidTransaction(...))`, but does not preserve the exact `InvalidAccessKeyError` subtype; the test must **not** claim a specific on-chain failure reason that was not supplied.

## Trust boundary and evidence limits

- The privileged Sandbox administrator retains an ephemeral **FullAccess** key only within the trusted test harness, so the owner can register/revoke the agent key. The agent signer's `Signer` instance uses **only its scoped key**. This is an access-key behavior test, **not** a demonstration of fully process-isolated owner secret custody.
- The Guard remains **non-custodial for swap assets**. The test also proves owner-funded zero-deposit intent registration, unchanged mandate refusal, and rollback of unfunded/exhausted registration. It does not prove live quote acceptance, private account hardening, USDC transfer or cross-chain settlement. Storage credit is native NEAR held only to pay Guard storage or return unused credit to its owner.
- The agent account itself is funded with **local Sandbox NEAR** to pay gas. That account retains the trusted administrator's separate FullAccess key. Therefore this test proves that the *scoped key* cannot bypass access permissions, not that an arbitrary compromised environment holding a FullAccess key is safe.
- Base Sepolia source assets and EVM EOA signatures are **out of scope**. A Base-side agent with an unrestricted funded EOA can still bypass advisory X3 Guard; an independently security-reviewed smart-account/vault or signer must be enforced there.
- The local WASM SHA printed by the script identifies its compiled binary. It is **not** asserted to be byte-identical to the existing public testnet Guard or a public release.
- Independent public-testnet contract/vault enforcement, owner-approved funding/deployment, adversarial signing with an isolated disposable agent, exact-head CI, reproducible code hashes and reviewer signoff remain P0 gates.

This document must not be reworded as “X3 Guard cannot be bypassed” until a real restricted spending path and the corresponding negative/positive **public-testnet** evidence have passed independent review.

NEAR specification: https://near.github.io/nearcore/DataStructures/AccessKey.html

## Owner-funded agent storage

Function-call access keys cannot attach deposits. The three `agent_*` mutation
methods now support zero-deposit storage charging through an owner/agent credit.
See [the storage API and upgrade limits](GUARD_SCOPED_STORAGE.md).
The harness proves initial credit-record cost, registration debit, policy refusal
without debit, unused-credit withdrawal through its callback, and rollback after
budget exhaustion on the real local node. Callback transfer-failure recovery is
covered separately by a unit test with a mocked promise result.

For a local upgrade rehearsal, the harness accepts an optional second path to
an old Guard WASM. It deploys that binary, creates a mandate and caller-funded
intent, upgrades to the first WASM without initialization, then compares both
records and runs the sponsored-registration checks. This optional path never
fetches public bytecode automatically. Verify the old binary hash separately.
