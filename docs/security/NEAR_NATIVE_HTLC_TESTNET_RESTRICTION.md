# Experimental native NEAR HTLC — testnet deployment-name guard

This experimental escrow's `new()` initializer previously accepted any account ID, including mainnet-style `*.near` and 64-character implicit accounts, without an explicit testnet guard.

The contract now rejects initialization unless `env::current_account_id().as_str().ends_with(".testnet")`. This aligns the experimental HTLC with the existing Tidepool Guard's name-based testnet restriction and prevents accidental initialization on ordinary mainnet-named accounts.

## Mandatory caveats

**This is a naming restriction, NOT cryptographic network authentication.** A `.testnet` suffix does not independently authenticate the NEAR genesis hash. Before any deployment, the deployment/owner signing tool MUST independently query NEAR testnet chain/genesis through trusted, mutually consistent RPCs, bind the exact WASM SHA-256 and intended account ID, and refuse absent, wrong-network or mismatched responses. Do not copy a Testnet deployment artifact into mainnet.

No deployment, funding, admission of public liquidity providers or mainnet readiness is authorized by this change. The HTLC remains an **experimental trust-boundary pilot**. The native BNB↔NEAR bridge still has unresolved finality/attestation/atomicity and liquidity safety gates (issue #42).

## Verification

The red test `native_near_htlc_refuses_mainnet_named_contract_at_init` failed before the guard (old code initialized successfully). After the guard, tests reject `native-x3-vault.near` and a 64-hex implicit account, accept `x3-vault.testnet` with the correct registrar, and preserve the funding-window, claim, refund, asynchronous withdrawal and storage-limit invariants.

```bash
cargo test --manifest-path x3-lang/bridge/near/Cargo.toml --locked
cargo fmt --manifest-path x3-lang/bridge/near/Cargo.toml -- --check
cargo clippy --manifest-path x3-lang/bridge/near/Cargo.toml --all-targets --locked -- -D warnings
cargo build --manifest-path x3-lang/bridge/near/Cargo.toml --release --target wasm32-unknown-unknown --locked
```

This does not deploy the escrow, share credentials, sign, or move funds.
