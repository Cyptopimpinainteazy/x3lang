# NEAR mainnet Guard upgrade and native recovery

Observed November 11, 2026. This records two separately owner-approved operations
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

## Read-only verification

With `near-cli-rs` 0.30.1, inspect both finalized transactions and the account at a
recorded post-upgrade block. These commands do not sign or send transactions:

```bash
near transaction view-status BVXX3hJEmThWVZNpUknZv9HCpGezisiN8SQKsZgA3faF network-config mainnet
near transaction view-status 81DFPVN9hTa9WE5fvQFgD91hdp6KiJ49W1ookf4J7YxV network-config mainnet
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

The upgraded contract supports owner-prepaid agent storage, but deployment alone
does not provision credit, restrict agent keys, refresh mandates, approve a new
payment or fund a source-chain deposit. Those actions need their own reviewed
inputs and authorization. A funded agent purchase and official 1Click mainnet
settlement remain unproven by this evidence. Track registration and judging
eligibility are separate from deploying a contract.

A code rollback requires another explicitly approved deployment. Any new scoped
storage credits and outstanding withdrawal callbacks must be handled before
rolling back to code that does not understand those records. No rollback was run.
