# invoice_06 public mainnet RPC replay

`invoice06-mainnet-rpc.json` archives public read-only observations fetched on
November 13, 2026. The exact capture time, public endpoints, RPC methods and
parameters are recorded in the document. No credentials, wallet files, signing,
new payments or contract changes were used to capture it.

SHA-256: `6aa00f559a5a1a5d30226f985622d4d88bd7808f152a097dc07bf8c048f239d3`

The offline test invokes the production `source_transfer` and
`destination_transfer` functions on the raw Solana deposit and NEAR payout,
using the existing invoice_06 evidence bundle for expected amounts, quote time
window, accounts and transaction identifiers. It also corrupts copies to test
rejection. The original evidence bundle remains unchanged.

This verifies transfer parsing against recorded public responses. It does not
authenticate RPC consensus, establish current endpoint availability, rerun the
complete signed quote verifier, prove Guard authorization, or prove cross-chain
causality/atomicity. Mutated copies are negative test inputs, not chain evidence.

Run offline from the repository root using the pinned judge environment:

```sh
PYTHONPATH=x3-lang:scripts/hackathon python -m pytest \
  scripts/hackathon/test_verify_mainnet_payment.py \
  scripts/hackathon/test_mainnet_rpc_replay.py -q
```

Keep this capture immutable. Add a separate, dated capture and checksum when
refreshing evidence; do not replace this archive to hide a regression.
