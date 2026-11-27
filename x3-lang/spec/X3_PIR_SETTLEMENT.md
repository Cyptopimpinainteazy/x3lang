# X3-PIR-07 — blocked native settlement plans (experimental)

This is a compiler-to-native-settlement **planning and verification** milestone, not an executable cross-chain adapter. The existing NEAR–BNB escrow requires an authenticated NEAR finality verifier. Its interface documents the missing destination-chain implementation. No caller flag, matching hash, RPC assertion, owner signature, or compiler artifact may substitute for that verifier.

## Source and independently selected context

See compiler/tests/fixtures/pir_settlement_claim.x3. The canonical capability parser accepts `near_bsc_window "<64 lowercase hex window ID>" "<lock|claim|refund>"`. Root only, zero delegation depth, explicit no_asset (no funds authority), and both exact_code_identity and authenticated_finality requirements are mandatory. Other requirements, including atomic_rollback, refuse. NotFunded is intentionally not exposed in this profile; the native escrow recovery path is unchanged.

The dedicated `compile_settlement_plan(source, trusted_scope, now, &WindowTerms)` API requires independently selected terms. WindowTerms includes BSC chain ID (97 or 31337 only), exact escrow/depositor/LP addresses, positive u128 BNB wei, salt, NEAR testnet vault/initiator/recipient, nonzero hashlock, positive u128 NEAR yocto amount, ordered funding/expiry deadlines in nanoseconds, and nonzero NEAR genesis and vault code hashes. These are reviewed inputs, not authenticated observations. Amounts and identities are bound through the existing native window ID; no grant of asset authority is implied. Lock planning refuses after funding deadline. Claim/refund planning does not imply that either outcome has occurred.

The Rust compiler derives source binding, window ID and outcome commitment using the unchanged native SHA-256 formats in bridge/settlement.py, and rejects a mismatch with the source-declared window. A separate domain-separated plan digest binds exact source SHA-256, canonical grant digest, NEAR genesis/code hash, and native outcome commitment. Genesis/code identity are not in the historical native window ID; they are separately bound by the plan and checked by the existing escrow's verifier interface. The plan status is always BlockedAwaitingNativeVerifier; no execution or authorization method is present. Generic capability compilation refuses settlement resources even when passed a set claiming support.

## Local verification

From x3-lang:
```sh
cargo test -p x3-lang-compiler --test settlement_capability --locked --offline
python3 -m unittest discover -s bridge -p test_settlement.py -q
FORGE=forge bash scripts/verify_pir07.sh
```

Four Rust tests cover known native vectors, mutation of every native window term, mandatory finality/code requirements and context commitment. The integration script extracts Rust test vector outputs, copies the existing native Solidity contracts and selected tests to a temporary directory, and compares all native commitments against Solidity. A deny-all verifier proves a matching commitment cannot lock funds; claim/refund against the absent lock refuse. Existing 20 escrow tests use a modeled verifier, not live NEAR. Fixture addresses, time, genesis/code values and funds are simulated. An initial new Solidity test had an expectRevert ordering error (argument evaluation made an external call before the intended revert); corrected by precomputing the arguments.

## Release hold

No authenticated destination-chain verifier, live chain observation, actual funds transfer, proof-backed finality, recovery liveness, cross-chain atomicity, or executable settlement authorization demonstrated. Native contract behavior and permissionless recovery are unchanged. Keep this stacked draft separate; no merge, deployment or real funds authorized. Next: independent trust anchor, verified receipt inclusion and vault code/state proofs, destination-chain verifier implementation, real testnet proof lifecycle, and universal receipts that distinguish planning from verified execution.
