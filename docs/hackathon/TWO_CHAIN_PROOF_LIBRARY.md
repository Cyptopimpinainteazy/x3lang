# X3Lang cross-VM trading proof library — evidence tiers

> **Current evidence level: baseline only. No X3Lang cross-VM trade has been executed or settled.**
> This is Tidepool community testnet, not official NEAR 1Click mainnet and not atomic HTLC settlement.

## Independent evidence levels

| Tier | Required evidence | Current status |
|---|---|---|
| 0 | Typed `.x3` frontend route, exact source SHA | Local tests pass in PR #99; not native X3VM bytecode evidence |
| 1 | Genuine maker-signed quote, hash, refund and explicit minimum | Historical signed dry/live fixtures captured, expired, **never reuse** |
| 2 | Owner-signed mandate and one-use nonce on independently hashed, deployed testnet Guard | **Not yet deployed/authorized** |
| 3 | Base Sepolia `transfer()` calldata, finalized receipt, matching Transfer event, two agreeing independent RPC providers and token bytecode hash | **Not yet executed** |
| 4 | NEAR successful finalized `ft_transfer` receipt, matching receipt-outcome ID, token and destination account, minimum output | **Not yet executed** |
| 5 | Historical finalized NEAR destination `ft_balance_of` baseline and later finalized increase covering the transfer receipt block | **Baseline only; no post-trade change** |
| 6 | Provider-independent cryptographic *causal linkage* proving NEAR receipt resulted from specific EVM deposit/quote | **Not yet implemented**; tiers 3–5 cannot alone prove trustless atomicity |
| Mainnet | Audited native atomic kernel, correct chain credentials/HTLC finality/settlement economics | **Not authorized or evidenced** |

## REAL read-only finalized baseline

`evidence/tidepool-testnet/pretrade-finalized-baseline-20261107.json` is an immutable public-chain snapshot, captured without signing. SHA256 of exact JSON bytes:

`69a835814805876acd2eafb1ca0390081e6bf4d4fca6bcc4c5347dbb609c4120`

- NEAR testnet `x3lang.testnet`, finalized state at block **276593025**, hash `A5dpq1mU5qPY5Mo71aUzkG97atz8yQ4V7grHAkxydCvU`, Tidepool test USDC 100,000,000 base units (100.000000 test USDC).
- Base Sepolia chain ID **84532**: the **same finalized block 55543788** with hash `0x3ec79c5d4f79884ca25e79443833eb12d0bf98128cfeb84ee2205001b523db8a` was independently checked via `https://sepolia.base.org` and `https://base-sepolia-rpc.publicnode.com`. Both report 100,000,000 test USDC base units, 1,000,000,000,000,000 wei (0.001 test ETH) in source, and identical token bytecode SHA256 `db77a21a647b5973bac5728bf9f6977b56933006bf2f0730a14d109d33072c83`.
- These are **pre-trade balances**. They do not establish receipt delivery, atomicity, profitability, custody safety, or mainnet readiness.

To capture another *read-only* finalized baseline, use:

```sh
PYTHONPATH=x3-lang python3 scripts/hackathon/capture_two_chain_baseline.py \
  --out evidence/tidepool-testnet/pretrade-finalized-baseline-<unique-date>.json
```

The command refuses to overwrite existing evidence; after the trade, retain the original pre-trade hash and pass its recorded NEAR `block_hash` as the `--near-balance-before-block` argument.

## Post-trade independently verifiable report (not yet runnable to success)

When a real owner-authorized swap has executed and the NEAR account received an actual Tidepool token transfer, obtain the Base Sepolia source transaction hash, the NEAR token-transfer transaction hash and **signer account ID**, and a NEW finalized NEAR block hash *after* the receipt. Then run:

```sh
PYTHONPATH=x3-lang python3 scripts/hackathon/two_chain_settlement_proof.py \
  --quote-file <exact-preserved-live-quote-response.json> \
  --guard-account x3guardlaunch2026.testnet \
  --guard-wasm-sha256 <exact-on-chain-code-sha256> \
  --nonce <owner-registered-one-use-nonce> \
  --program-sha256 <exact-sha256-of-approved-.x3> \
  --base-tx-hash <Base-Sepolia-transaction-hash> \
  --near-tx-hash <NEAR-token-transfer-transaction-hash> \
  --near-tx-signer <NEAR-signer-account> \
  --near-balance-before-block A5dpq1mU5qPY5Mo71aUzkG97atz8yQ4V7grHAkxydCvU \
  --near-balance-after-block <finalized-NEAR-post-trade-block-hash> \
  --output evidence/tidepool-testnet/<unique-live-proof-name>.json
```

**Hard requirements:** Post-trade verification accepts a historically expired quote only after rechecking its authentic signature and proof that the source transaction occurred **within** both its signed deadline and the Guard policy expiry. NEAR requires `final_execution_status == FINAL`; the before/after snapshot hashes must match the canonical block hashes by height under the current finalized NEAR head. The receipt-transfer block must also be canonical. Evidence outputs are created with *exclusive file creation*, so competing processes cannot overwrite each other. The quote must have the genuine pinned Tidepool maker signature and exact testnet route; NEAR Guard must have approved its hash under the exact source program SHA and code hash. Base Sepolia `eth_getTransactionByHash` and `eth_getTransactionReceipt` must agree with two RPC providers on source, destination, chain, token bytecode, exact amount, successful ERC-20 `Transfer` event, inclusion block and confirmation depth. Both providers must see the source inclusion block **at or before** each provider's finalized head (inclusion height <= finalized height), and at least **12 latest-head confirmations**. Provider head or finalized-height drift of up to **6 blocks** is allowed; the source inclusion block hash, exact transfer receipt, token code and ERC-20 event must agree. If their finalized heights are identical, finalized head hashes must also be identical. These are the code's actual thresholds, not a guarantee that providers' latest head heights are identical. NEAR `EXPERIMENTAL_tx_status` must show a successful `ft_transfer` action to `x3lang.testnet`, its actual matching successful `receipts_outcome` ID and block hash; the finalized balance increase must cover both the quoted minimum and receipt inclusion height.

**Fail closed:** No fake transaction hashes, no manual settlement checkboxes, no provider status standing in for on-chain receipt evidence, no partial success artifact, and no automatic retries or private keys. All unit test fixtures are clearly **synthetic**; they validate negative gates only, never serve as public transaction evidence.

The final classifier is `independent_testnet_two_chain_transfer_evidence_NOT_atomicity_or_native_x3vm_proof`. The verifier deliberately **does not claim atomicity or cryptographic origin-to-destination causality**. That requires a separate protocol-level proof tied to the exact EVM deposit and NEAR settlement, with replay/finality validation. The bundled `guard` is an owner policy gate and does not make Tidepool's trusted relay into an HTLC.
