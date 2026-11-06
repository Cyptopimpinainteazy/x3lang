# NEAR testnet: genuine two-epoch cryptographic verification (UNTRUSTED START)

This is a **security development milestone**, not a verified mainnet light client. The implementation in `x3-lang/bridge/lightclient-core/src/epoch.rs` verifies real signatures and epoch commitments **relative to an initially untrusted RPC-supplied checkpoint**. There is NO independently approved checkpoint provenance, NO authenticated header root, NO durable trusted checkpoint storage, and NO ability to authorize BSC settlement. All returned `authenticated` and `authorized_to_sign` values remain `false`. Do not equate successful signature verification with consensus authentication when the initial validator set could have been substituted by an attacker.

## Provenance and reproducible real-chain evidence

Two NEW testnet JSON-RPC response-object fixtures (parsed then serialized with sorted keys; **not verbatim HTTP bytes**) were obtained READ-ONLY from `https://rpc.testnet.near.org` on 2026-11-06 UTC through `next_light_client_block`:

| Historical input `last_block_hash` | Fixture | SHA-256 | Advertised next epoch |
|---|---|---|---|
| `8a9NLEi7bAH4Uc1GXYTHLywBY2ZeeseEESQxVgtuMFEd` | `tests/fixtures/hbm-announces-p27.json` | `acf673ba657bc0b90f75e17bf5fd1f407ae81103f8cbfa6f6c1fd74713f8a5e0` | p27 epoch |
| `8udbn8ipZxVrZAE1t13M3fx1fLAf4ptGb1SLeBpdtrjZ` | `tests/fixtures/p27-announces-gbz.json` | `dca4ca9cf83d838630cd06ae7ecdb17f23e3639c97c91798b9cf966e6fdc68cd` | GbZ epoch |

The third existing view is the **real RPC response already recorded in PR #57** at `evidence/near-proof/testnet/sha256/c4/c47f0c381dbd9a37ac432aa4f77cf9d47ea8ccc452bfe4ff2a67b868953caed7.json`; SHA-256 `c47f0c381dbd9a37ac432aa4f77cf9d47ea8ccc452bfe4ff2a67b868953caed7`. It is a GbZ-epoch block view at height `276420402`.

The three candidate views have heights `276349252` (HbM epoch), `276392452` (p27 epoch) and `276420402` (GbZ epoch). The first view's next validator set is used to verify the **second view's** 12 signed approvals, and the second view's next validator set is used to verify the **third view's** 19 signed approvals. On the latter, 19 real Ed25519 signatures correspond to signed stake `588424816164625374350075972122329` of total stake `592425651545485319272531395151441`, which exceeds the required strict two-thirds threshold. The signer weight includes the full 20-validator denominator, not just the validators who signed.

Historical `next_light_client_block` RPC calls can return newer candidates if repeated; fixture SHA-256 integrity and reproducible offline verification are the actual test guarantees. No live RPC calls occur in the tests or CLI.

## Run locally

```bash
cd x3-lang/bridge/lightclient-core
cargo test --locked
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo run --locked --bin check_candidate_epochs -- \
    tests/fixtures/hbm-announces-p27.json \
    tests/fixtures/p27-announces-gbz.json \
    ../../../evidence/near-proof/testnet/sha256/c4/c47f0c381dbd9a37ac432aa4f77cf9d47ea8ccc452bfe4ff2a67b868953caed7.json
```

The binary prints an `ACCEPTED RPC candidate` result for each of two later blocks, **never** `authenticated=true`. It exits nonzero on invalid RPC JSON, wrong epochs/commitments, stale/replayed heights, timestamp rollback, bad validator stakes/keys, approval message, fake Ed25519 signatures, or missing strict approval quorum.

## Trust boundaries and missing release work

1. **Initial checkpoint is untrusted:** the starting HbM view and advertised p27 validator set came from one RPC service. Merely pinning its hash or recording SHA-256 does not prove it belongs to canonical NEAR. The Rust type is deliberately `UntrustedEpochChain`, and there is no `Trusted` conversion or settlement signing API.
2. The implemented approval reconstruction uses `SHA256(next_block_inner_hash || current_block_hash)` and Borsh `Endorsement` tag + signed `height + 2`, verifies real Ed25519 signatures, and enforces signed stake strictly >2/3 of **preceding epoch** committed validators. It validates the V1 `next_bps` Borsh commitment to the candidate lite header, and ensures epoch progression. Unsupported V2/future validator variants fail closed pending protocol-specific review.
3. **Checkpoint provenance and durable antirollback:** independently verify/pin testnet genesis, network identity, trusted epoch BPs and checkpoint header through a sound trust process. Then implement durable, rollback-safe trusted state and follow real NEAR fork/finality rules. No claim of this property is made here.
4. **Outcome/state proof:** PR #58 has real outcome-Merkle verification against a caller-provided head root. Neither that proof nor the approvals here independently authenticates finalized X3 escrow storage, expected code hash, a claim/refund, or a BSC payout.
5. Independent audit, multi-epoch NEAR protocol upgrades, validator slashing changes, testnet escrow deployment, and adversarial funded evidence are still required.

**FINANCIAL SECURITY GATE: NO MAINNET FUNDS, NO EIP-712 CERTIFICATE SIGNING, NO TRANSFERS, NO DEPLOYMENT, NO MERGE WITHOUT APPROVAL.**

References: https://github.com/near/nearcore/blob/master/docs/ChainSpec/LightClient.md (check current nearcore implementation for protocol upgrades); https://nomicon.io/ChainSpec/LightClient.html (background; may be outdated); https://github.com/Cyptopimpinainteazy/x3lang/issues/56 (release-blocking work).
