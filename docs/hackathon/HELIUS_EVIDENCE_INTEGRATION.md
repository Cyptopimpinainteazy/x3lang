# X3Lang Helius Solana Evidence — shipping boundary

**Implementation:** x3-lang/helius_evidence.py, optional `verify-deposit --helius` on the existing signed-live-quote receipt.

## Why this is useful

The existing X3 SolanaRpc evidence gate calls `getTransaction` at `finalized` commitment and checks the signature, successful result, slot, deadline, SPL mint, receiving token owner, and exact base-unit balance delta. Helius adds a *second* parsed-source check. It cannot replace finalized RPC or establish why a NEAR recipient received assets.

Helius **Parsed Events** supersedes its legacy Enhanced Transactions API on Solana mainnet. The implementation POSTs a single signature to `/v1/parsed-events/transactions` with `rawTransaction=none`. It requires one `parserStatus: OK` response with a matching signature, `parsed.transactionStatus: OK` and no error, exactly matching slot and block time, and the correct mint, owner, and net raw-token amount. This net amount must equal the finalized RPC delta.

The provider response is hashed with SHA-256 for audit. A bad/missing API key, transport error, missing transaction, incorrect recipient, wrong mint, timestamp, amount, or failed transaction produces an unverified receipt. Helius's API key is not printed in the receipt.

## Usage

Load a Helius API key into the process environment securely. Avoid storing it in repository files, command arguments or shell history.

    export HELIUS_API_KEY=<loaded securely from your own credential manager>

Then verify an **actual 1Click live** receipt with its **actual finalized Solana deposit transaction**:

    python3 x3-lang/near_intents.py verify-deposit --receipt /path/to/receipt.json --tx <SOLANA_SIGNATURE> --helius

The guided `near/first-swap.sh` and `near/agent-demo.sh` workflows use `X3_REQUIRE_HELIUS=1` to pass `--helius` automatically. A failed deposit proof halts before attempting a settlement proof.

Without `--helius`, the existing finalized-Solana-RPC verifier behaves exactly as before. With `--helius` the additional Helius provider gate becomes mandatory; it never silently falls back to RPC-only evidence.

For RPC redundancy in production, set `SOLANA_RPC_URLS` to comma-separated independent provider endpoints. Helius is not independent finality by itself; at least one independently controlled Solana RPC read is advisable and endpoint disagreement must reject.

## Evidence required to claim a real success

- Owner-authorized, actually finalized source transaction
- Valid X3 mandate, signed live 1Click quote, and bound amount, mint, recipient, refund route
- Source-chain receipt verified against Solana RPC and (when enabled) Helius Parsed Events
- Actual NEAR destination account and token contract observed after delivery
- Explicit causality limitations; NEAR balance increase is not itself proof that the particular source transaction caused it
- Refund evidence and replay controls independently verifiable where used

No transfer, solver integration, contract deployment, Helius API call, or successful cross-chain settlement is asserted by tests alone.

Official docs: https://www.helius.dev/docs/api-reference/parsed-events/transactions
