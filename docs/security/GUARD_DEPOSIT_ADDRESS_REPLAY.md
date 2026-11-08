# X3 Guard: distinct signed quotes reusing a deposit address

## Reproduced security issue

The contract-wide signed-quote replay ledger introduced on the stacked draft PR #102 indexed **only the signed payload hash**. A manager can sign two *different* byte strings that contain identical deposit-address terms (e.g., different signed `timeEstimate` metadata). Both signatures can be valid while the quote hashes differ, meaning two owners or intents could each record authorization for the *same deposit destination*.

A real Rust Guard regression using the synthetic, never-funded test signing key reproduced the defect against the unpatched replay-ledger code on PR #103:
- Owner A and owner B registered independent, otherwise identical intents.
- Two separately signed live-quote payloads had different signed hashes but the same `depositAddress`.
- Both `accept_quote` calls succeeded; two `quote_accepted` events were emitted for that address.
- The new regression **failed before the patch** with exit 101, asserting that second authorization must be refused.

This is duplicate **authorization bookkeeping** in a local simulated blockchain test, **not evidence that any financial deposit, settlement, or theft took place**. Whether a 1Click manager legally reuses a deposit address must be reviewed before designating this constraint as universal; for X3 Shield the safe default is to refuse ambiguous duplicate authorizations.

## Minimal fail-closed patch

- Appends `StorageKey::ConsumedDepositAddresses` *after* `Intents=0`, `Mandates=1`, `ConsumedQuoteHashes=2`, using Borsh tag `3`. Existing prefixes remain unchanged.
- Adds a second contract-wide map from **deposit address → accepting owner/nonce**. A verified signed quote cannot be authorized if the same address is already reserved, regardless of its signed payload hash or owner.
- Writes quote-hash and deposit-address consumption records **only on successful acceptance in the same NEAR transaction** as accepted quote storage. Rejected signatures, disallowed policies and duplicate deposits must not modify either registry.
- Adds `x3.deposit-ledger.fresh.v1` marker on `Contract::new()` only. `Contract::migrate()` must never synthesize this marker; an existing installation with unknown prior deposit-address history must reject new acceptance with `X3_NEAR_DEPOSIT_LEDGER_UNINITIALIZED` until independently audited backfill or a versioned fresh-state deployment is explicitly authorized.
- Does **not** change `Contract`'s serialized state layout or `STATE_VERSION`. This is not an authenticated migration mechanism.

## Tests and evidence

- `differently_signed_quotes_cannot_reuse_a_deposit_address_across_owners` proves the red→green behavior. Both signatures are valid under an explicitly synthetic local test key; the same deposit address appears in both; the second stored quote must remain absent, and the original claimant remains recorded.
- `missing_deposit_history_marker_fails_closed_before_quote_authorization` proves unknown legacy history blocks acceptance.
- Prefix test checks storage tags 0–3. Existing rejected-policy test checks that rejected quotes do not consume a deposit address.
- Multi-payment synthetic mandate test fixtures now generate unique local-only deposit addresses per quote, avoiding accidental reuse of the one fixed demonstration destination. The intentional address-collision regression continues to reuse it.
- Rust source mutation gate includes cases for bypassing deposit-address deduplication and ignoring the new legacy-history marker. A first expanded run caught 9/10; the quote-hash bypass survived only because the independent deposit-address guard blocked the same reused quote. A separate test-only fault-injection case deliberately deletes the deposit-address index entry after a first acceptance and proves the quote-hash registry alone still rejects replay. After this stronger test, all **10/10 curated Rust mutants were killed**, zero survived/invalid/timed out. This is not a full-codebase mutation score.
- Local exact command: `cargo test --offline --locked --manifest-path x3-lang/near/x3-guard/Cargo.toml`. The 62-test suite passed locally after updating fixtures; additional exact-head CI and independent review required.

## Hard release constraints

This branch is stacked on **PR #103 → PR #102 → PR #98 → PR #94** and is not production release authority. No public deployment, real manager signing, wallet funding, external transaction broadcast or funds transfer performed. The new WASM differs from old public testnet code; do not repin prior deployed-byte evidence or suppress the required check. Existing private `xxxstar` proof mirror access and per-commit CI remain blocking where red.

The contract does **not** inherently restrict an unrestricted wallet key from depositing independently. Quote signatures alone are not proof of settlement, authenticated finality or cross-chain causality.
