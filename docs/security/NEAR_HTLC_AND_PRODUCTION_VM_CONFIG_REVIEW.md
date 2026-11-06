# NEAR HTLC deployment and X3 VM production configuration hardening

Independent judge audit follow-up to the 2026-11-06 report against commit `136aa88`. This is **not** a security audit sign-off.

## Finding B1: NEAR HTLC initialization had no account/network restriction

`x3-lang/bridge/near/src/lib.rs` now refuses `new()` unless the current contract account's name ends in `.testnet`. The regression first failed because initialization succeeded on `x3-htlc.near`; after the fix the mainnet-style account test rejects, and all existing tests run using explicit `.testnet` test contexts. `#[init]` remains guarded by NEAR's initialization machinery.

**Important residual risk:** near-sdk does not expose an authenticated `chain_id` for this contract. A `.testnet` suffix is **only a naming restriction**, not an attestation of which NEAR blockchain executed the WASM. Do NOT claim guaranteed mainnet prevention on a chain where a matching account name can exist. No code was deployed. The HTLC still uses root `BTreeMap` state and doesn't charge per-lock storage or prune terminal swaps, so permissionless funding/withdrawal and storage exhaustion remain separate release-blocking concerns. Do NOT deploy or fund this HTLC with real value.

## Finding B2: Silent production verifier configuration fallback

`x3-lang/vm/src/bridge.rs::init_production_backend()` now:

- Requires explicit valid `X3_EVM_RPC_URL` or `X3_SVM_RPC_URL` when the corresponding RPC verifier is selected. The initializer refuses omitted, blank, non-HTTP(S), or malformed endpoints. Explicit localhost URLs are permitted for authorized local testing, but never inserted as a default.
- Rejects present-but-invalid `X3_EVM_MIN_BLOCK_NUMBER`, `X3_SVM_MIN_SIGNATURES`, `X3_SVM_MIN_STAKE_BPS`, and `X3_EVM_MIN_CONFIRMATIONS` with `X3_BRIDGE_INIT_FAILED`. A truly omitted optional field may use a documented default. `X3_SVM_MIN_STAKE_BPS` cannot be lowered below 6667 by configuration.
- Rejects malformed Unicode environment values for these security-sensitive settings.

This prevents parser typos from silently lowering verifier requirements. It **does not** prove that any external RPC is trustworthy, that validator keys are authenticated, or that a real NEAR checkpoint exists. Production signing and financial settlement remain disabled by the separate unresolved proof/trust requirements.

## Reproduce

```bash
(cd x3-lang/bridge/near && cargo test --locked && cargo fmt --check)
(cd x3-lang && cargo test -p x3-lang-vm --locked && cargo fmt --check && cargo clippy -p x3-lang-vm --all-targets --locked -- -D warnings)
```

Security gates still blocked: independently authenticated NEAR checkpoint, code hash and terminal escrow storage, BSC ↔ NEAR causality, storage economics, audit and authorized deployment approval. The changes are draft-only and must receive independent review before merge.
