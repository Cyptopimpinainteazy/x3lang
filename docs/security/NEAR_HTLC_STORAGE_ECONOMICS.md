# Experimental native NEAR HTLC: storage economics (issue #71)

Contract: `x3-lang/bridge/near/src/lib.rs`. It is still a testnet research prototype. Nothing here authorizes deployment, funding, signing or a production or trustless atomic-swap claim.

## Reproduced finding

Under the previous layout, every window, swap, credit and in-flight withdrawal lived in a `BTreeMap` inside the root contract state. `lock()` is permissionless, required 0.001 NEAR of principal, and charged nothing for storage.

A reproduction against `origin/master` (325890e) serialized the root state before and after one real `lock()`. It used `lp.testnet` → `alice.testnet` and a 64-hex swap ID:

| Measure | Value |
|---|---|
| Root-state growth per lock | 160 bytes |
| Storage stake for that growth | 0.0016 NEAR (1.6×10²¹ yocto) |
| Minimum deposit at the time | 0.001 NEAR |

So each lock consumed more storage stake than the caller paid, taking it from the vault's balance. The reviewer's figure of about 167 bytes matches this once account names are a few characters longer. The whole root value was also re-serialized on every call.

## Change

1. **Per-record storage.** `windows`, `swaps`, `credits` and `withdrawals_inflight` are now `near_sdk::store::LookupMap`s with stable one-byte `BorshStorageKey` prefixes. The root state has a fixed size. A test checks that it does not grow across locks. Live-record limits now use O(1) counters instead of scanning storage, so execution cost no longer grows with the number of records.
2. **The creator pays the storage stake, separately from principal.**
   - `lock(id, recipient, hashlock, expires_at_ns, amount)` takes the principal explicitly. The attached deposit must be at least `amount` plus the storage cost. The storage cost covers the swap record plus a reserve for the payee's credit entry and a concurrent in-flight withdrawal entry, priced at `env::storage_byte_cost()`.
   - `fund_window` works the same way, with the window's exact principal.
   - `register_window` is now `#[payable]`, and the registrar pays for the window record.
   - Any excess deposit is refunded to the caller.
   - An underfunded call panics before any record, liability or counter changes.
   - Storage stake is recorded per record (`storage_deposit`) and in total (`storage_deposits()`). It is never added to `liability`.
3. **Verified accounting.** After each write the contract flushes and requires that `env::storage_usage()` grew by no more than the bytes that were paid for. If the protocol's per-record overhead ever changes, this check fails closed rather than under-charging. A test shows that the record formula equals the runtime's own accounting exactly, and that later payout entries stay inside the prepaid reserve.
4. **Exact-deposit views.** `lock_deposit_required(initiator, recipient, amount)` and `window_deposit_required(initiator, recipient)` return the deposit each call needs.
5. **Terminal records.** Claimed and refunded swaps, and funded and not-funded windows, are kept as their full records. They serve as anti-replay tombstones, preserve the read-only `get` and `get_window` views, and leave the claim, refund and withdraw paths unchanged. Their storage was paid in full when they were created, so keeping them costs the vault nothing. Pruning them (shrinking to a smaller tombstone and crediting back the freed stake) is a possible later optimization. It is deliberately not done here, because it would weaken those historic views for a saving of a few dozen bytes.
6. **Migration.** The contract has never been deployed, and nothing in this repository calls it, so there is no on-chain state to migrate. The new layout is incompatible with the old Borsh root by design. Any future deployment starts with `new()`.

## Tests (`cargo test --locked`, 30 pass)

The 21 existing tests were kept and adapted to the new API. The new tests are:

- root state size stays constant across locks;
- the record-bytes formula equals the runtime's storage accounting exactly, and payout entries fit the reserve;
- an underfunded lock fails before any record, liability, stake or counter changes, including the old principal-only deposit and a deposit one yoctoNEAR short;
- with minimum principal, storage is paid separately and alone exceeds the old 0.001 NEAR minimum;
- an excess deposit is refunded to the caller, as an exact transfer receipt;
- extreme 64-character account IDs are charged for their length;
- an under-charged write fails closed;
- window registration and funding charge storage, and principal-only funding is refused;
- permissionless lock spam: 256 concurrent swaps with maximum-length names, then a mix of claims and refunds, and withdrawals where one in three fail. After every transition, storage growth is prepaid and `vault balance ≥ liabilities + storage stake`. Terminal IDs cannot be reused.

Mutation checks:

- Removing the payout reserve fails 3 tests.
- Disabling the post-write storage check fails the fail-closed test.

`cargo fmt --check` and `cargo clippy --all-targets --locked -- -D warnings` pass. The WASM release build runs in CI (`x3-bridge-security.yml`).

## Still open

- Gas and storage were measured in near-sdk unit tests, not in a NEAR sandbox or workspaces. Measure them on testnet before any funded use.
- Item 6 of the issue still stands: the full contract needs an independent audit before any funded or mainnet use. No deployment, signing or fund movement is permitted without explicit owner approval.
