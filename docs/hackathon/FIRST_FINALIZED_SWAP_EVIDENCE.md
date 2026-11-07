# First Guard-authorized X3Lang testnet transfer — immutable evidence

**Status: finalized testnet transfer; genuine on-chain receipts. NOT trustless atomic cross-chain settlement, NOT canonical native X3VM bytecode execution, and NOT mainnet-ready.** The Tidepool testnet relayer retains custody/trust assumptions. Do not claim this demonstrates cryptographic causation between two chains without an independent protocol-level bridge proof.

## Real recorded event

- Origin: **Base Sepolia chain 84532**, **1.000000 test USDC**, sender `0x5aF0c1BF95B3A158f6Fc48e3DED35903b2fa0721`, recipient/quote deposit `0x9195A7547f057FF2790601f7bE4b58ABcB372185`. Actual transaction `0x46c21dc305a9271c4e74c6d0734fb50f847a1e53d3cad87976133d66fd6728d9` in block **55557170**, hash `0xa592deccd3e32b3a47dc819cb5daffe1c8d950ebe6c39ed29c26e8dc70231828`. Two independent RPCs confirmed canonical finalized inclusion and exact ERC-20 transfer event.
- Onchain Guard on **NEAR testnet**: `x3guardlaunch2026.testnet`, exact on-chain WASM SHA-256 `1d7a8b1ee97b7cdc500db1c19b8f3a2bef06e986576ec428f62f8c3ea1b5a145`. Historical canonical final approval at NEAR block **276595201**, hash `9uNGKrtSN8dDQS9HYbVNZ7SmYasF52Vm1o8Bxt5kbbYq`, before Base inclusion. Signed quote SHA-256 `131f314c2532965f2678e79f3f7cc87cfe8238a549da1405e802f4c90db1034c` and `.x3` source SHA-256 `1398f64e7e208d3796883aa59cb169187c1042ac1af3781bd6efc014755df860` are bound to the exact owner nonce `x3_base_testnet_trade_001`.
- NEAR maker-signed settlement tx `EBEBxJNgV9U5tBFBLj18ZzukgArqwXH5rBsBStFyCm1d`, signer `maker.tidepool-sandbox.testnet`, `final_execution_status=FINAL`. Its receipt `3svyUVK2tG6MR7qBfpkZtBk6pSqqxiLEpbyjeTgW6eSJ` performs successful token `ft_transfer` of **997,400 base units = 0.997400 test USDC** to `x3lang.testnet` on `usdc.tidepool-sandbox.testnet` (block 276595461).
- Independently finalized balance increased **100,000,000 → 100,997,400 base units** on NEAR; anchored pre/post snapshots at block heights 276593025 and 276595590.

## Immutable, reproducible artifacts

| Artifact | SHA-256 | Nature |
|---|---|---|
| `evidence/tidepool-testnet/pretrade-finalized-baseline-20261107.json` | `69a835814805876acd2eafb1ca0390081e6bf4d4fca6bcc4c5347dbb609c4120` | Onchain read-only baseline (previously merged) |
| `evidence/tidepool-testnet/first-finalized-two-chain-certificate-20261107.json` | `8e1baca995f4aa534821432c5e177f406eab6923f998473a36b4d5d9d5540a39` | Successful full original two-chain verifier output recorded 2026-11-07T08:08:34Z |
| `evidence/tidepool-testnet/first-signed-live-quote-20261107.json` | `7718da27680bbb35cbf199caaae06ee77987dd5daf60f2dfd1ca175a08da7535` | Historical maker-signed quote **includes no signing keys** and must never be reused for a new deposit |

Run from repository root (offline, no accounts, network, or keys required):

```bash
PYTHONPATH=x3-lang:scripts/hackathon python3 scripts/hackathon/verify_first_swap_archive.py
PYTHONPATH=x3-lang:scripts/hackathon python3 -m unittest scripts/hackathon/test_first_swap_archive.py -v
```

Optional **read-only live historical** re-verification:

```bash
PYTHONPATH=x3-lang:scripts/hackathon python3 scripts/hackathon/verify_first_swap_archive.py --live
```

The initial proof generator requires two providers' *current* finalized heights to be within **6 blocks**. That is valuable for live admission, but provider synchronization can drift long after an original finality certificate was created. The separate archival verifier **does not relax the initial proof's rules**; it validates the exact preserved certificate and maker-signed quote, checks BOTH providers' **historical canonical block hashes at the certificate's original finalized checkpoint**, and independently fetches and verifies original ERC-20 transaction/calldata/event/code, NEAR final `ft_transfer` receipt, historical Guard authorization, and historical finalized recipient balance snapshots. The original certificate's SHA-256 is immutable. There is no signed transaction or wallet access in this verifier.

**Caution:** Archived source and destination receipts, even if both finalized and amount-consistent, do not constitute a trust-minimized interchain causal proof. This is an authentic Tidepool-mediated testnet cross-chain delivery example, with explicit trusted-relayer limitations. Synthetic mutation tests demonstrate refusals; they are not chain evidence.
