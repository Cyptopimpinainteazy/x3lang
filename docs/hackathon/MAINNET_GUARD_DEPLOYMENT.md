# NEAR mainnet Guard upgrade, recovery and agent provisioning

Observed November 11, 2026. This records four separately owner-approved operations
on NEAR mainnet. It does not authorize another transaction or establish official
1Click settlement. [Machine-readable manifest](../../evidence/near-guard-mainnet-2026-11-11/manifest.json).

## Deployed identity

| Field | Verified value |
| --- | --- |
| Account | `x3lang.near` |
| Deployment transaction | [`BVXX3hJEmThWVZNpUknZv9HCpGezisiN8SQKsZgA3faF`](https://nearblocks.io/txns/BVXX3hJEmThWVZNpUknZv9HCpGezisiN8SQKsZgA3faF) |
| Action | One `DeployContract`; no initialization or migration |
| Final transaction and receipts | Success |
| Deployed WASM size | 572248 bytes |
| SHA-256 | `9a614640ac8bc39bbbcb1674ce0f4fe2f51610ac7ba828c9c4574be92712a632` |
| NEAR code hash | `BPdpJAw8R4SDh6Wi7dPcXTs5T7ncJDVKCLgzuqZFb15T` |
| Previous code hash | `GRy6z84ucCR9WMVXyepibVDC34TyKwjWtEi9rteKJtny` |
| Total charged fee | 0.0041436113609864 NEAR, including receipt execution |
| Post-upgrade storage usage | 575538 bytes; increased by 20970 bytes |
| Storage reserve at observed protocol price | 5.75538 NEAR; increase of 0.2097 NEAR |

The independently fetched state at preflight block **224111404** and post-upgrade
block **224111627** contained the same seven key/value records. Their canonical
digest was `950bfc8ba0f81da80928548ef9ab5ff7781b5039718a829862c6ddac7a39c297`.
Schema version 2, mandate version 3, the accepted intent with no recorded outcome,
and replay-ledger state were preserved. No storage credit was funded by deployment.
See the [independent deployment observation](../../evidence/near-guard-mainnet-2026-11-11/deployment-verification.json).

## Source and test provenance

The reviewed runtime source is commit
[`9dcea3cc72ea77532c1f2678e367d09a85c81e6b`](https://github.com/Cyptopimpinainteazy/x3lang/tree/9dcea3cc72ea77532c1f2678e367d09a85c81e6b/x3-lang/near/x3-guard).
Its Guard source and locked dependencies are unchanged at release-candidate head
[`8ec3dec1b5b2b8c43ffe6cd3957696bc090730ad`](https://github.com/Cyptopimpinainteazy/x3lang/commit/8ec3dec1b5b2b8c43ffe6cd3957696bc090730ad)
in [PR #189](https://github.com/Cyptopimpinainteazy/x3lang/pull/189).
That candidate's 18 reported checks passed before deployment. This document does
not imply that the whole candidate branch has merged into canonical `master`.

[PR #206](https://github.com/Cyptopimpinainteazy/x3lang/pull/206), head
`93e78f73a49672f43bded1815b310f815814db1d`, adds a real local Sandbox regression
against unchanged production WASM. It proves rollback on insufficient storage
credit, success after funding and refusal of a cross-nonce replay. Its synthetic
signing key is confined to disposable Sandbox state. This is not official quote
acceptance or public-chain settlement evidence. The [exact-head Sandbox CI run](https://github.com/Cyptopimpinainteazy/x3lang/actions/runs/43086065900)
also reproduced the reviewed WASM. All nine Actions checks and GitGuardian passed;
the separate RecurseML provider returned an analysis error, not a review finding.

[PR #210](https://github.com/Cyptopimpinainteazy/x3lang/pull/210), head
`c358f092ededd4ac1c1d58049d16a7db09efe947`, fixes named-nonce parsing and intent
export fidelity, and makes the generic dry-run VM refuse an external mandate
before effects. Independent focused review passed 22 tests. At that exact head,
the unchanged local source-admission gate, 57 boundary regressions and 46 compiler/VM
proof cases passed under Rust 1.90.0. This branch is not merged; hosted CI was
still incomplete at this observation, with an external RecurseML error. Two broad
CLI corpus failures involving a Base Sepolia example were reproduced on its exact
base commit. These local checks neither authorize a payment nor establish settlement.

## Native account-surplus recovery

A separately approved, single native `Transfer` moved **0.3 NEAR** from
`x3lang.near` to `dotx3.near`:
[`81DFPVN9hTa9WE5fvQFgD91hdp6KiJ49W1ookf4J7YxV`](https://nearblocks.io/txns/81DFPVN9hTa9WE5fvQFgD91hdp6KiJ49W1ookf4J7YxV).

The transaction and receipts finalized successfully. The recipient balance
increased by exactly 0.3 NEAR. The total charged fee was **0.0000446365125 NEAR**.
At the verified post-transfer block, the source balance was
**5.8514654493516443 NEAR**: the full **5.75538 NEAR** storage reserve plus
**0.0960854493516443 NEAR** operating surplus. Code, all seven stored records and
keys were unchanged apart from the expected signing-key nonce increment.
See the [independent recovery observation](../../evidence/near-guard-mainnet-2026-11-11/recovery-verification.json).

This was recovery of native account surplus. It was not a refund of swapped
tokens, a storage-credit withdrawal, or release of the contract's storage reserve.
No account was deleted. These historical balances may change with later activity.

## Owner-prepaid agent storage

The owner approved one `fund_agent_storage` self-call on `x3lang.near` for
`x3-agent.x3lang.near`, attaching **0.03 NEAR**:
[`72QrfNRf131kxnBTmZb8EJjQP6H3MKEWxvSBrMuyB9qw`](https://nearblocks.io/txns/72QrfNRf131kxnBTmZb8EJjQP6H3MKEWxvSBrMuyB9qw).
The transaction and all receipts finalized successfully. The call created
**0.02891 NEAR** unused agent storage credit after reserving **0.00109 NEAR** for
**109 bytes** of additional account storage. The charged fee was
**0.0002515973317774 NEAR**.

Because this was a same-account call, the deposit earmarked existing native
balance; it did not transfer 0.03 NEAR off-account. At block **224116857**, the
account held **5.8512138520198669 NEAR**, including **5.75647 NEAR** storage reserve
and **0.02891 NEAR** unused-credit liability, leaving **0.0658338520198669 NEAR**
unencumbered surplus. The prior seven records, deployed WASM and keys were
preserved apart from the expected owner signing-key nonce. One credit record was
added. All **28 independent read-only checks** passed; see the
[storage observation](../../evidence/near-guard-mainnet-2026-11-11/storage-verification.json).

## Delegated agent key

The owner separately approved one `AddKey` action on `x3-agent.x3lang.near`:
[`5NbFPn49edaLsdiy6FtZ2wzb73k4SZfEfDqvg5CQ2MaQ`](https://nearblocks.io/txns/5NbFPn49edaLsdiy6FtZ2wzb73k4SZfEfDqvg5CQ2MaQ).
It finalized successfully with the following public permissions:

| Field | Verified value |
| --- | --- |
| Delegated key | `ed25519:J3NiU8J63HAfz2MTDu1zQVhFRYS5Pio21h1kNtJdP11x` |
| Permission | `FunctionCall` |
| Receiver | `x3lang.near` |
| Exact allowed methods | `agent_register_intent`, `agent_accept_quote`, `agent_record_outcome` |
| Initial allowance | 0.02 NEAR |
| Retained owner recovery key | `ed25519:8teaXGYFELKQN5QA7gQqeA4B7URrcMQqaczZ5D68c23u` — `FullAccess` |
| Storage increase | 189 bytes |
| Charged fee | 0.0000420792991044 NEAR |

The allowance limits transaction costs; it is not a transfer of 0.02 NEAR or a
payment approval. At block **224117358**, the agent account had exactly these two
keys. Guard code, account fields, all eight state records and the **0.02891 NEAR**
credit were unchanged. All **25 independent read-only checks** passed; see the
[AddKey observation](../../evidence/near-guard-mainnet-2026-11-11/addkey-verification.json).
The owner retains account administration and recovery. This is a trusted-operator
harness; key restriction does not establish OS/runtime credential isolation.

## Read-only verification

With `near-cli-rs` 0.30.1, inspect the finalized transactions and the account at a
recorded post-upgrade block. These commands do not sign or send transactions:

```bash
near transaction view-status BVXX3hJEmThWVZNpUknZv9HCpGezisiN8SQKsZgA3faF network-config mainnet
near transaction view-status 81DFPVN9hTa9WE5fvQFgD91hdp6KiJ49W1ookf4J7YxV network-config mainnet
near transaction view-status 72QrfNRf131kxnBTmZb8EJjQP6H3MKEWxvSBrMuyB9qw network-config mainnet
near transaction view-status 5NbFPn49edaLsdiy6FtZ2wzb73k4SZfEfDqvg5CQ2MaQ network-config mainnet
near account view-account-summary x3lang.near network-config mainnet at-block-height 224111627
```

Historical queries require an RPC provider retaining the referenced blocks.
To reproduce the raw-byte comparison, call JSON-RPC `query` with
`request_type: "view_code"`, `account_id: "x3lang.near"` and
`block_id: 224111627`; base64-decode `code_base64` and calculate SHA-256.
In a transaction's `ActionView`, `DeployContract.code` is instead the base64
encoding of the **32-byte code digest**, not the WASM itself
([nearcore conversion](https://github.com/near/nearcore/blob/2.14.0/core/primitives/src/views.rs#L1497-L1499)).
Compare that decoded digest directly with the reviewed SHA-256.

For storage preservation, query `view_state` with empty `prefix_base64` at both
recorded block hashes in the evidence. Compare the full key/value lists after
sorting by `key`. The recorded digest hashes UTF-8
`json.dumps(sorted_records, sort_keys=True, separators=(",", ":"))`.
Raw state values, wallet credentials and private quote receipts are not bundled.
RPC observations are provider-backed evidence; they are not an independently
authenticated NEAR light-client checkpoint.

## Remaining release boundaries

Owner-prepaid storage credit and the restricted delegated key are now provisioned.
No new payment registration, live quote or swap was approved or sent as part of
these four operations. They do not refresh the mandate, approve a payment or fund
a source-chain deposit. Those actions need their own reviewed inputs and
authorization. A funded agent purchase and official 1Click mainnet settlement
remain unproven by this evidence. Track registration and judging
eligibility are separate from deploying a contract.

A code rollback requires another explicitly approved deployment. Any new scoped
storage credits and outstanding withdrawal callbacks must be handled before
rolling back to code that does not understand those records. No rollback was run.
