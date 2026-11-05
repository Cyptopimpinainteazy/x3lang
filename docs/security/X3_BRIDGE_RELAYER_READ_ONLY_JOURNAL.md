# Stage 2: crash-persistent read-only bridge watcher

`x3-lang/bridge/relayer_watch.py` is a SQLite-backed **testnet-only watch queue**, not an execution relayer. It performs no HTTP requests, does not sign attestations, does not broadcast or transfer, cannot certify settlement, and has no transaction authority. `Watch.authorized_to_sign` always remains `False`.

The journal registers swap ID, destination vault and an operator-supplied deadline. It uses `BEGIN IMMEDIATE` to allocate bounded worker leases; another worker cannot claim the same job until the lease expires. In-flight tasks are therefore recoverable after a process crash. Missing state and RPC failures are recorded as retriable errors; they never authorize BSC refunds or a `NotFunded` outcome. Observed `Claimed` or `Refunded` states move a watch to `REVIEW_REQUIRED` (not `SETTLED`), and conflicting terminal observations are refused. Registered swaps cannot have their deadline silently changed.

## Remaining failures and design limits

- An observer can lie or a worker can submit forged observations. The journal does not authenticate RPC responses, NEAR validator consensus or deployed escrow contract code and must not feed a financial signer.
- The supplied deadline is **not enforced by the current NEAR vault's `lock` method**. It is informational only; expiry or a missing swap is not evidence that no future NEAR lock can be made.
- Lease-expiry retry is at-least-once, not exactly-once. A slow worker may continue external side effects after lease expiry; no external financial side effects are allowed in this module.
- A SQLite instance cannot enforce cross-host consensus or quorum anti-equivocation. There is no trusted multi-signer service or double-sign evidence.
- No authenticated `NotFunded` certificate can be emitted. A future NEAR protocol revision must enforce an irrevocable lock-by deadline and expose a finalized on-chain nonfunding state, bound to the exact BSC escrow terms. Signers must authenticate it independently.
- State-machine integration, work scheduling, metrics, concurrency stress, reorg handling, and two-chain testnet deployments remain outstanding.

**Release status:** `NO_MAINNET_FUNDS`. This is deliberately noncustodial and read-only.
