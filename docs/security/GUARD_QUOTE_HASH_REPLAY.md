# X3 Guard: contract-wide signed-quote replay boundary

## Reproduced defect (security release blocker)

Prior to this change, `accept_verified` verified a quote signature and checked whether *that one intent record* had already accepted a quote. It did **not** prevent the **identical signed payload and signature** from authorizing a second intent with a different nonce or a different owner.

Red-team reproductions use the real Rust Guard implementation with a deterministic **test-only signing key** and a synthetically signed live-quote fixture. Two distinct authorization records were accepted and emitted the same `quote_hash` and `deposit_address`. These tests establish **duplicate authorization bookkeeping**, **not** a funded 1Click transaction, destination-chain transfer, or wallet theft.

Tracked in GitHub issue #100. The two regression tests first failed on the original source with explicit duplicate `quote_accepted` events.

## Proposed fix and boundaries

After pinned-manager Ed25519 verification, the Guard consults a contract-wide `ConsumedQuoteHashes` map keyed by the exact signed quote hash. Once **all** policy guards pass and the accepted quote is stored, the contract records `quote_hash → owner/nonce` in the same NEAR transaction. A subsequent acceptance attempt—under another nonce, or even another owner—must fail with `X3_NEAR_QUOTE_REPLAY`.

Refused policies, invalid signatures, and dry quotes must not consume hashes. The read-only `get_quote_consumer` method exposes the original claim for audit/debug purposes.

The implementation appends a new `BorshStorageKey` variant (retaining existing Intents/Mandates discriminants) and stores the index in a separate `near_sdk::store::LookupMap`. It **does not alter the serialized `Contract` state layout**, so the existing `STATE_VERSION` remains 2.

### Critical migration caveat

**This index contains only quotes accepted after code containing the new map is deployed.** Existing `LookupMap`-backed intents are not enumerable without off-chain evidence. Therefore, an in-place upgrade of an existing contract cannot be claimed to block replays of **historically accepted quote hashes** without an authenticated backfill, complete historical provenance and an independent audit. A new fresh-state deployment is a separate option requiring explicit owner approval, contract/storage review, new WASM identity, and new testnet proof.

**New fail-closed upgrade safeguard:** `Contract::new()` is the only constructor that initializes an explicit `x3.quote-ledger.fresh.v1` storage marker. `Contract::migrate()` deliberately does **not** set it. `accept_verified` checks the marker *after signature verification but before consuming a signed quote*. Without it, all new authorizations fail with `X3_NEAR_QUOTE_LEDGER_UNINITIALIZED`; a legacy contract upgraded in place cannot quietly authorize using an empty replay map. The read-only `is_fresh_quote_ledger_initialized` view exposes this state. The legacy-state simulation was verified **RED** on the parent #102 implementation and **GREEN** after this guard, and v11/v12 migration tests confirm no implicit marker. **This is a protective denial, not a backfill**. Any production migration or public deployment still requires separately authorized, audited procedures.

The index guards repeated **identical signed quote bytes**. It does not prove that two *different* manager-signed quotes with different hashes cannot reuse the same deposit address, nor does it prove a finished cross-chain settlement. Those cases need a separate provider protocol and security review.

## Verification

The regression sequence in `x3-lang/near/x3-guard/src/tests.rs` covers:

- One live **test-key-signed** quote reused across two different nonces: reject the second, preserve both records.
- Same quote attempted by two different owners: reject the second, preserve state.
- Rejected policy must **not consume** an otherwise legitimate quote; the legitimate intent may subsequently accept it.
- `get_quote_consumer` must report the original accepting owner and nonce.

Run:

```bash
cargo test --locked --manifest-path x3-lang/near/x3-guard/Cargo.toml
cargo check --locked --target wasm32-unknown-unknown --manifest-path x3-lang/near/x3-guard/Cargo.toml
python3 scripts/security/guard_rust_mutations.py --output /tmp/x3-guard-rust-mutants.json
```

The mutation gate now includes seven replay/security mutations plus an eighth that disables the legacy-upgrade marker check; the applicable regression must kill each. Counts are **curated source mutations only**, not whole-contract coverage. Exact-head CI and an independent review must confirm the proposed PR.

### Measured x3star1 local checks (before final PR CI)

- **59/59** Guard native unit tests passed, including replay, upgrade fail-closed, agent rolling-spend, and storage-prefix regressions.
- **8/8** curated Rust mutants were killed, including deliberate bypass of the legacy-upgrade marker check, with no survivors, invalid mutants or timeouts.
- Rustfmt, Clippy with `-D warnings`, and `wasm32-unknown-unknown` compilation passed.
- The parent replay-only source previously built a local unsigned WASM with SHA-256 `f5f91586b0c760e5980d559f15405d31272b9ddbbb279cfaa04a1f9c2cdad6f2`. The **current source with fresh-only upgrade protection** builds local WASM SHA-256 `e3e2dd47bcd396423af73f801475128f1337cbdde8b402d13e2f5645f605753f` through `scripts/hackathon/guard_sandbox_smoke.sh`. These are separate source revisions and neither hash establishes a deployed public contract, signed release, or independently verified byte-for-byte reproducibility. A fresh-machine exact-head release build remains required.

## Release prohibitions

No automatic merge, no public chain deployment, no wallet signing or funding. The changed Guard WASM will not match the previously deployed public NEAR testnet hash; do not silently repin old evidence, disable exact-byte proof, or claim authenticated finality. The existing private `xxxstar` cross-repository mirror gate must also remain fail-closed.
