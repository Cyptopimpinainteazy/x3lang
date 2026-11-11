# NEAR mainnet Guard upgrade, recovery and payment authorization

Observed November 11, 2026. This records the four completed setup/recovery operations
and the subsequently owner-approved intent registration and authorization on NEAR
mainnet. It does not authorize another transaction or establish official
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

[PR #211](https://github.com/Cyptopimpinainteazy/x3lang/pull/211), head
`1ad2982139aedeab9dc1300e79d10ef4bff70731`, pins the exact offline CLI transaction
bytes and verifies the signature before one RPC submission. Its local evidence
records 774 Python tests passed (one skipped), 88 Node tests passed, and synthetic
installed-CLI offline tests. The first suppressed CLI failure's precise cause
remains unknown; these tests do not reconstruct that failure.

[PR #212](https://github.com/Cyptopimpinainteazy/x3lang/pull/212), head
`4594496a79eda2796eb1670c825e44e00367094f`, checks live receipt-gas purchase costs
before signing and accepts an explicit gas limit from 1 through 30 Tgas. It
neither changes gas nor increases allowance or retries automatically. Its local
[evidence](https://github.com/Cyptopimpinainteazy/x3lang/blob/4594496a79eda2796eb1670c825e44e00367094f/audit-artifacts/agent-gas-admission-20261111/verification.json)
records 92 focused tests, 796 applicable Python tests passed (one skipped), and
88 Node tests passed. A real local Sandbox registered the candidate-shaped policy
at **10 Tgas**, with **3,001,220,252,593 gas burnt**, against the unchanged deployed
WASM. The Sandbox used disposable accounts, a relative deadline, a simpler test
mandate and a synthetic key allowance; it was not a clone of mainnet state.
The tested file hashes match the two exact commits. Both PRs remain draft and
unmerged at this observation; local results do not establish hosted CI success.

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

## Registration refusal and corrected gas purchase

Before the successful registration, a 30 Tgas attempt produced transaction hash
`32csQESKgA7Ux7Dy7dG4ZEvPvUv6fmZ5GwrvDYvCRa38`; submission returned
`INVALID_TRANSACTION`, with `InvalidAccessKeyError` / `NotEnoughAllowance`.
At finalized blocks **224119953** (official RPC) and **224119949** (archival
FastNear RPC), all **18 independent unchanged-state checks** passed: the intent
was absent, key nonce and allowance were unchanged, and account balances,
**0.02891 NEAR** credit and all eight stored records were unchanged. Both this
hash and the previous attempt's hash returned `UNKNOWN_TRANSACTION` with
`wait_until: NONE`. These observations establish no finalized effects at those
blocks; they do not prove global mempool absence.

The earlier estimate priced all gas at the current block price. Protocol 88
instead purchases receipt gas at `max(current_gas_price, min_gas_purchase_price)`:
**1,000,000,000 yoctoNEAR/gas**, versus the observed current price of
**100,000,000**. Including action and receipt overhead, the exact registration
required **0.030922689800803 NEAR** upfront at 30 Tgas, exceeding the delegated
key's **0.02 NEAR** allowance. At 10 Tgas it requires
**0.010922689800803 NEAR**, within that unchanged allowance.

Upfront purchase is a temporary reservation. Unused gas and gas-price differences
can be refunded; the final charged fee must be checked from finalized outcomes
and balances. The owner's **0.005 NEAR network-fee budget** is distinct from the
revised **0.012 NEAR upfront-reservation ceiling**. Burning all registration gas
at the observed price would cost **0.0011223375128354 NEAR** at 10 Tgas; that is a
price-constant calculation, not a guarantee about future prices. See the
[historical admission observation](../../evidence/near-guard-mainnet-2026-11-11/registration-admission-observation.json)
and [nearcore cost calculation](https://github.com/near/nearcore/blob/2.14.0/runtime/runtime/src/config.rs#L451-L513).

## Finalized registration and owner authorization

After explicit approval of the revised **10 Tgas** limit, the agent registered
`mainnet_demo_nov11_invoice_04` with one zero-deposit `agent_register_intent` call:
[`EfBwTeb4rHNZzDjXfiJGD2J5SwL4QSayaBLaLJUqWA7k`](https://nearblocks.io/txns/EfBwTeb4rHNZzDjXfiJGD2J5SwL4QSayaBLaLJUqWA7k).
The transaction and all receipts finalized successfully. Independent verification
passed **27 checks** against the exact approved argument bytes, restricted key and
transaction nonce **224117214000001**. The stored policy binds source SHA-256
`c922179f25423e62f09e037a4d4096cf805420cb752dd29a5154df60b1cb8b21`
and mandate version **3**. This records policy admission, not a token transfer.

The actual agent fee was **0.0002845527244807 NEAR**, matching both account and
key-allowance debits after refunds. Registration added **499 bytes**, debiting
**0.00499 NEAR** of prepaid storage credit and leaving **0.02392 NEAR** unused.
Seven previous records were preserved; the credit record changed and one intent
record was added, for nine records. Guard native balance and code, agent storage,
restricted method scope and owner FullAccess recovery key were preserved.
See the [registration observation](../../evidence/near-guard-mainnet-2026-11-11/registration-verification.json),
pinned to post-registration block **224120779** before the owner approval.

The separately approved owner self-call then finalized:
[`6i8ur3A9FtaLnSz6uk1pCUt3TSp4uhHBPkDKQfLs3GYk`](https://nearblocks.io/txns/6i8ur3A9FtaLnSz6uk1pCUt3TSp4uhHBPkDKQfLs3GYk).
It called `approve_intent` for the same nonce with **30 Tgas** and **0.02 NEAR**
attached deposit. Independent verification at block **224120956** passed
**24 checks**: only the intent's owner approval changed, eight other records were
preserved, and account storage grew **15 bytes**. The **0.02392 NEAR** unused
credit, mandate, deployed code and agent account/key state were unchanged.

The owner's upfront gas purchase plus deposit was
**0.0509190548336098 NEAR**, within the available unencumbered account surplus.
After refunds, the owner's account decreased only by the charged fee,
**0.0003077702207735 NEAR**. Remaining unencumbered surplus was
**0.0653760817990934 NEAR**, after storage reserve and unused-credit liability.
See the [owner authorization observation](../../evidence/near-guard-mainnet-2026-11-11/owner-authorization-verification.json).
The observed intent had approval from `x3lang.near`, **no accepted quote and no
recorded outcome**. These two transactions do not establish a source-chain deposit
or official 1Click settlement.

## Read-only verification

With `near-cli-rs` 0.30.1, inspect the finalized transactions and the account at a
recorded post-upgrade block. These commands do not sign or send transactions:

```bash
near transaction view-status BVXX3hJEmThWVZNpUknZv9HCpGezisiN8SQKsZgA3faF network-config mainnet
near transaction view-status 81DFPVN9hTa9WE5fvQFgD91hdp6KiJ49W1ookf4J7YxV network-config mainnet
near transaction view-status 72QrfNRf131kxnBTmZb8EJjQP6H3MKEWxvSBrMuyB9qw network-config mainnet
near transaction view-status 5NbFPn49edaLsdiy6FtZ2wzb73k4SZfEfDqvg5CQ2MaQ network-config mainnet
near transaction view-status EfBwTeb4rHNZzDjXfiJGD2J5SwL4QSayaBLaLJUqWA7k network-config mainnet
near transaction view-status 6i8ur3A9FtaLnSz6uk1pCUt3TSp4uhHBPkDKQfLs3GYk network-config mainnet
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

Owner-prepaid storage credit and the restricted delegated key are provisioned;
the new mandate-v3 intent is registered and owner-approved. No accepted quote or
outcome was present at the final observation. A live quote, its admission and any
source-chain deposit remain separate steps with their own reviewed inputs and
authorization. A funded agent purchase and official 1Click mainnet settlement
remain unproven by this evidence. Track registration and judging eligibility are
separate from deploying a contract.

A code rollback requires another explicitly approved deployment. Any new scoped
storage credits and outstanding withdrawal callbacks must be handled before
rolling back to code that does not understand those records. No rollback was run.
