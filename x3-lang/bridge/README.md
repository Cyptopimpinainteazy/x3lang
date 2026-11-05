# X3 native liquidity providers — initial reservation layer

This is the **first local RFQ/reservation component**, not a deployed bridge.

- Registers provider identities and their publicly named BSC and NEAR addresses.
- Creates time-limited offers with **integer** price ratios, integer fees, and source input limits.
- Uses SQLite transactional reservations to prevent oversubscription **inside one database**.
- Rejects every newly registered provider by default.
- Prevents self-certifying a provider as wallet-authorized via the public API.
- Requires independently verified destination inventory before any executable quote can be reserved.
- Does not import keys, sign, broadcast, deploy, move money, or infer that a BSC wallet owns a NEAR account.

## Current security boundary

There is **no** production wallet-signature verification, inventory attestation,
price oracle, liquidity deposit/escrow contract, network-wide reservation lock,
or atomic swap settlement. A signed quote and reservation are not sufficient
to move funds. The example wallet address is **public reference only**, not
proof of ownership. The local DB is not a consensus source.

An internal test directly sets authorization and inventory rows solely to
exercise pricing/overbooking behavior. Production code must never treat
operator-entered rows as on-chain evidence.

## Next implementation gates

1. Verify LP controls both wallet identities with domain-separated signed challenges.
2. Independently verify NEAR vault inventory and reserves by finalized NEAR RPC.
3. Enforce liquidity reservations at the destination escrow contract, not just SQLite.
4. Develop audited BSC and NEAR HTLC contracts with matching preimage validation,
   staggered deadlines, refunds, asset controls and chain-specific gas provisions.
5. Test adversarial atomic-swap sequencing, reorgs, replay, preimage leakage, RPC
   disagreement and crash recovery.
6. Execute testnet swaps and verify both sides before an owner-approved mainnet test.
7. Restrict initial live counterparties to controlled, funded wallets.

## Tests

`python3 -m unittest discover -s x3-lang/bridge -p 'test_*.py' -v`

Avoid placing real assets into any address derived from this local module.

## BSC ↔ NEAR HTLC prototype (draft; no deployments)

Experimental native-asset contracts:
- BSC Solidity escrow: bsc/src/NativeBnbHTLC.sol. Locked native BNB is payable to a designated LP with a 32-byte SHA-256 preimage, or refundable to the depositor after expiry.
- NEAR Rust escrow: near/src/lib.rs. Locked native NEAR becomes an enforceable on-chain recipient credit on preimage claim, with asynchronous withdrawal and failure callback to restore withdrawn credit.
- protocol.py derives network-domain-separated swap parameters and enforces staggered expiries; no caller-supplied chain flags prove settlement.
- monte_carlo.py estimates downside distributions, selects minimum fee meeting modeled risk limits, and refuses stale, unverified, or insufficient inventory. All outputs remain non-executable.

Proposed sequence (not yet orchestrated): User locks BNB; LP independently verifies finalized BSC escrow and matching terms; LP locks NEAR with an earlier expiry; user verifies the finalized NEAR escrow; user claims the NEAR contractual credit and reveals a preimage; LP uses that preimage to claim BNB; funds are withdrawn or refunded according to each vault state.

CRITICAL UNRESOLVED VULNERABILITY: A secret can leak from a failed NEAR claim transaction, which could permit the counterparty to collect BNB without the recipient receiving NEAR credit. Staggered timeouts mitigate some races but are not a cryptographic guarantee against this outcome. DO NOT deploy or fund permissionless or production swaps on this prototype. Before live release, require verified destination finality as part of the source-chain claim authorization or a redesigned atomic execution mechanism, plus adversarial tests and independent security review.

Other gates: verified escrow code identities, finality-aware evidence adapters, wallet-control challenges, reserve-backed LP offers, NEAR storage/gas funding, reproducible deployments, signed quote terms, no manual oracle trust flags, testnet wallet approvals, failed-claim/mempool leakage and reorg tests, and an exact-head audited release.

Reproduction on x3star1:
- From repository root: python3 -W error::ResourceWarning -m unittest discover -s x3-lang/bridge -p 'test_*.py' -v
- From x3-lang/bridge/bsc: ~/.foundry/bin/forge test --offline -vv
- From x3-lang/bridge/near: cargo test --locked --offline
- From x3-lang/bridge/near: cargo build --locked --offline --release --target wasm32-unknown-unknown

No live wallet credentials are required for these builds.
