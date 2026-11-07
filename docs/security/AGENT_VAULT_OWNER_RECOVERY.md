# X3 Shield pilot vault: owner-only escrow recovery

**Scope:** X3Lang's unmerged, testnet-named **native NEAR vault pilot**, stacked
on the 64-entry payment-history guard. This is not the deployed X3 Guard,
cross-chain settlement custody, or security-audited public testnet software.

## Trust and state rules

- Only the owner account named in the vault's immutable initialization may
  call `recover_unused(amount)`. The agent's limited `FunctionCall` key to
  `request_payment` cannot invoke recovery, and the contract checks the
  owner account itself if someone calls using an otherwise unrestricted key.
- Recovery is permitted only when `revoke` has been called, the owner's
  mandate has expired, or the 64-record payment history is exhausted. A
  currently active mandate without any of those conditions cannot withdraw.
- `reserved` must be zero: **no unresolved agent payment promises**.
  Recovering `available` never spends the separate pending-payment reserve.
  A missing callback can block owner recovery; there is no unreviewed
  force-finalization or administrator bypass.
- Recoverable amount must be positive and at most `available`. A second
  request while an owner withdrawal is pending is rejected. The vault checks
  runtime account balance minus near-sdk's storage-byte stake estimate,
  4,096 bytes of storage headroom, and a **1 NEAR callback/gas reserve**. These
  are conservative prototype parameters, not audited storage economics.
- Each accepted recovery consumes a monotonic `u64` sequence number, with
  one bounded latest-recovery record. Funds are removed from `available`
  *before* an asynchronous promise transfers to the **registered owner only**.
  A private callback with exactly one promise result settles `Paid` or, on
  failure, restores `available` and marks `Failed` without reusing the
  sequence. Stale or duplicate callbacks cannot refund a second time.

## Verification

Rust `cargo test --locked --offline` covers: unauthorized owner/agent
withdrawals, early withdrawal denial, expiration/history-cap release, pending
agent payout protection, failed payout callback followed by recovery, failed
owner recovery and retry, duplicate callback, one pending recovery at a time,
and insufficient on-chain storage/gas reserve.

A **real offline NEAR Sandbox** harness deploys the candidate vault WASM
and uses ephemeral `owner.testnet`, `agent.testnet` and `vault.testnet`
accounts. It verifies the scoped agent, one allowed 0.1 test NEAR payout,
denied recipient/cap/nonce/revoked actions, refuses owner early withdrawal
and non-owner withdrawal, then performs a **0.5 test NEAR** owner recovery.
The owner's actual native on-node balance must rise by more than 0.4 NEAR
after gas, the latest recovery callback must be `Paid`, and the vault's
available ledger must show 0.4 test NEAR.

A **failed owner transfer is tested with near-sdk PromiseResult::Failed in
Rust unit tests**; it has not yet been reproduced as a failed outgoing
transfer on a real NEAR node. Unit test contexts are not public-chain proof.

## Not approved for public funds

1. Prototype is **native NEAR only**. The compromised Base EOA key is still
   bypassable; no EVM smart account or Guard policy integration is proven.
2. The existing vault prototype's contract state layout changes in this
   recovery candidate. There is **no validated state-migration path** from
   a previously deployed vault; only a new disposable local deployment was
   tested. Do not attempt a live upgrade.
3. Actual storage byte measurements, deterministic reserve sufficiency under
   heavy callbacks/restarts, and worst-case gas/storage exhaustion need a
   separate independent security review. A compromised agent can still fill
   all 64 payment-history slots and deny further service.
4. Deployment must be **atomic deploy+initialize**, pinned to verified testnet
   genesis, reviewed exact WASM and explicit owner approval. No public-testnet
   contract deployment or funding was performed.
5. No final mainnet readiness, autonomous custody, or cross-VM settlement claim
   follows from this local-node result.

Owner issue: [#129](https://github.com/Cyptopimpinainteazy/x3lang/issues/129).
