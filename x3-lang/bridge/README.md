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
