# NEAR reference light-client vectors (near/near-light-client @ 522f940)

Vendored from <https://github.com/near/near-light-client> at commit
`522f94003fbbedfdc1e7a7de6284f5a1d25d8134` (2024-07-16). `PROVENANCE.json`
records the upstream URL and SHA-256 of every file; the test
`test_vendored_vectors_match_provenance_manifest` re-checks them.

| File | Contents |
|---|---|
| `test_0.json` → `test_2.json` | Real NEAR **testnet** `next_light_client_block` views, heights 154654776 → 154697976 → 154741176; each is in the epoch the previous one announced (two epoch transitions). |
| `main_0.json` → `main_2.json` | Real NEAR **mainnet** views, heights 86629892 → 86673092 → 86716292 (two epoch transitions). `main_0.next_bps` slots 17 and 42 (`nearcrowd.poolv1.near`, `nc2.poolv1.near`) share one consensus key. |
| `old.json`, `new.json` | Real testnet `light_client_proof` results (SuccessValue receipts). The reference pins their head block Merkle root as `WWrLWbWHwSmjtTn5oBZPYgRCuCYn6fkYVa4yhPWNK4L` (`crates/protocol/src/experimental.rs`). |
| `protocol_lib_rs_test_outcome_root.json` | The raw-string `light_client_proof` literal from `crates/protocol/src/lib.rs` line 547 (`test_outcome_root`), extracted byte-for-byte. Its `timestamp` number was float-rounded upstream (`…535000`) while `timestamp_nanosec` keeps the exact `…535094`; only the exact value hashes to the recorded block hash. |
| `LICENSE-MIT`, `LICENSE-APACHE` | Upstream license files (`Cargo.toml` declares MIT). |

## What these vectors are and are not

- The validator approvals are real Ed25519 signatures from the published NEAR
  producers. They verify against the `next_bps` committed one epoch earlier,
  which cannot be produced without those producers' keys.
- They are **not** a trusted checkpoint. Tests anchor each chain at
  `*_0.json` with a clearly labeled **synthetic test checkpoint signature**
  and leave the anchor epoch's own producers unknown (`current_validators:
  null`), so only the next-epoch successors can verify.
- The proofs in `old.json` / `new.json` are checked against the reference's
  pinned head root, not against an authenticated head: no matching real head
  view is published.

Cross-checks against the reference implementation:

- `main_0.next_bps` approving `main_1`: total stake
  `512915271547861520119028536348929`, approved
  `345140782903867823005444871054881` — the exact values asserted by the
  reference test `test_next_invalid_signatures_stake_isnt_sufficient`.
- `main_1`, `main_2` and `test_2` record the block hash they were requested
  from; the X3 lite-header hash reproduces each one.
