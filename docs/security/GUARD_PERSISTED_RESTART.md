# Guard local persisted-chain restart and unknown-transaction reconciliation

This is an **isolated, real NEAR Sandbox node restart** regression, using the exact compiled Guard WASM from a PR #108-derived source branch. It does **not** stop or modify an existing X3 validator. It does not use public testnet/mainnet RPC, any user wallet, deposits, or real funds.

## Exact boundaries and evidence

The Rust executable `x3-lang/near/guard-sandbox-tests/src/bin/persisted_restart.rs` launches only its own sandbox child under the **fixed container-private** `/x3-persist/guard-node-home`. Its wrapper executes in a Docker `--network none` namespace, with the pinned NEAR Sandbox 2.14.0 binary SHA-256 `797b9d7a9a23907048d68a427e86a55c94f1de232dbfb8b8a1eec09ecef3bf13`. The host mount exposes one dedicated disposable node database; existing validator homes and ports are never mounted. The harness refuses any other home or binary path and refuses existing node databases. A separate first-time genesis key is the **public sample/test-only NEAR Sandbox key**, never a funded user credential.

The real initial node processes signed **local on-chain** transactions to deploy and initialize the production Guard WASM, then registers a genuine Guard intent. It records the real account code identity and reads the owner/nonce record from the node, with quote and approvals still empty.

### Restart #1 — confirmed persisted state
- Terminate **only** the directly spawned local neard process.
- Restart a new neard process against the *same* home / RocksDB state.
- Require identical on-chain code identity, unchanged registered intent, persisted nonce usage and both fresh quote/deposit ledger initialization markers.
- Submit the already-consumed owner/nonce and require the actual chain transaction to fail.

### Restart #2 — outcome unknown during interruption
- Start an asynchronous owner-signed local registration, then terminate the owned neard process before awaiting the result. The test records whether the future was still pending.
- Restart exactly the same persisted chain home and **query** `is_nonce_used` for the previously uncertain nonce.
- If it is committed, never resubmit. If it is absent, submit it once through the real local chain. Assert a valid unique on-chain record and refuse any later replay.
- This tests **state-first reconciliation** after an interrupted local attempt. It cannot guarantee that the aborted future had reached the node's mempool or that an RPC receipt was lost after actual broadcast.

This is a **real node, real database persistence** test; it is not a simulator or fabricated state result. It still does **not** verify public NEAR consensus finality, arbitrary DB crash points, distributed quorum, multi-validator reorgs, cross-VM settlement or transaction delivery guarantees.

## Quote replay boundaries

This harness verifies that the fresh quote-hash and deposit-address ledger *markers* remain initialized after two local node restarts. The deployed Guard pins the real 1Click manager's public key, whose private signing key is not available or used. Consequently, the harness **does not create a live accepted 1Click quote** and does not prove survival of nonempty consumed quote/deposit ledger entries. Those require a separate, explicitly test-key-only contract build or authorized provider test fixtures and their own provenance.

The local WASM SHA must be captured from the actual run. Historical public testnet hashes and authenticated provenance are not transferred to this new code merely because a local hash matches an older test artifact.

## Run (from X3Lang repository root)

```bash
# Build the Guard for this exact source branch first:
(cd x3-lang/near/x3-guard &&
 cargo build --locked --release --target wasm32-unknown-unknown)

# Host must have Docker, Ubuntu 24.04 image, and reviewed sandbox binary:
X3_SANDBOX_BIN_CACHE="$HOME/.cache/x3lang/near-sandbox-2.14.0/near-sandbox" \
NEAR_NETWORK_ID=sandbox bash scripts/hackathon/guard_persisted_restart_smoke.sh
```

The wrapper requires at least **25 GiB of free disk space**, verifies the pinned sandbox SHA, runs under Docker network isolation with CPU/memory/process limits, and only cleans up its own `/tmp/x3-guard-node-restart.XXXXXXXX` disposable directory. On failure it outputs the last sandbox log lines before cleaning its own temp files; no existing assets are deleted. It creates a new, never-before-used chain home for each run.

For a local x3star1 reproducibility run, an explicitly audited built artifact from the PR #108 base may be supplied through `X3_RESTART_GUARD_WASM` and `X3_RESTART_GUARD_SHA256` while the source branch matches that base; this is **not** a substitute for fresh-machine exact-head WASM builds. The GitHub CI workflow `x3guard-persisted-restart.yml` rebuilds the actual Guard on the exact proposed commit and verifies the sandbox binary SHA before launching the isolated test.

## Release restrictions

- Test branch depends on draft security PR #108 and its upstream Guard replay/migration PR stack. **No mainnet approval or merge authority** follows from these tests.
- The private `xxxstar` mirror check, source provenance, deployed-WASM identity, quote history migration and independent review remain fail-closed release gates.
- No test controls a real public validator or signs a financial transaction. No test claims a live accepted quote or authenticated settlement.
- The scripted second interruption is a conservative unknown-state branch: it is not a reproducible proof of mempool-delivery race timing.
