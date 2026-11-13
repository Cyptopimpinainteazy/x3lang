# X3 Force Field — demo video script

The current three-minute script is [VIDEO_WALKTHROUGH.md](VIDEO_WALKTHROUGH.md).
Use that script for recording and the [invoice_06 proof card](../../PROOF_CARD.md)
for transaction anchors and trust boundaries. X3 Shield is the historical name.

The earlier testnet-only script at this path predates the mainnet settlement.
Its statement that no payment or settlement was evidenced is superseded by the
committed invoice_06 records; do not use it as the current recording script.

The current demo shows, in order:

1. **Payment:** 25 USDC from Solana; 24.9952 USDC delivered to `x3lang.near`.
2. **Enforced limits:** the original mandate and payment, local rejection of eight
   unsafe edits, and 21 passing checks in the mainnet Guard acceptance receipt.
3. **Proof:** exact source hashes, executed transfer receipts, and the agent's
   recorded `Settled` outcome with `floor_met: true`.

Run `python3 scripts/hackathon/invoice06_chain.py` from the repository root to
reproduce the five links from committed evidence. This is an offline verifier,
not a new payment or a live RPC observation. The owner manually funded the
recorded deposit; Guard does not custody funds or authenticate outcome hashes,
and the provider association does not prove cryptographic cross-chain atomicity.
