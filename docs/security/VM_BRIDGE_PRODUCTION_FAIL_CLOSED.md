# X3 VM production bridge: fail-closed environment gate

The November 7 re-judge found two distinct unsafe startup defaults in `x3-lang/vm/src/bridge.rs`: production EVM/SVM RPC verifiers silently selected `localhost` when their URLs were missing, and typoed numeric thresholds were silently ignored. An unset or misspelled `X3_BACKEND` also selected dry simulation.

These are **backend admission** findings, not claims of actual theft or bypassed on-chain finality.

## Required modes

- `X3_BACKEND=prod`: requires a **wired production** verifier; `resolve_bridge_backend()` itself always returns a hard error in prod. Production callers must pass their configured adapter to `resolve_bridge_backend_with()`. `init_production_backend()` requires a valid `X3_BRIDGE_VERIFIER` when prod is selected.
- `X3_BACKEND=dry`: explicit simulation mode. Simulation must never be mislabeled as chain evidence.
- Unset, mistyped, non-UTF-8, or unknown modes: hard error in `resolve_bridge_backend()`. No implicit dry-run.
- `X3_BRIDGE_VERIFIER=evm-rpc`: `X3_EVM_RPC_URL` must explicitly specify an absolute http(s) URL without embedded credentials; no default `localhost:8545`.
- `X3_BRIDGE_VERIFIER=svm-rpc`: `X3_SVM_RPC_URL` must explicitly specify an absolute http(s) URL without embedded credentials; no default `localhost:8899`.
- Optional numeric security settings `X3_EVM_MIN_BLOCK_NUMBER`, `X3_EVM_MIN_CONFIRMATIONS`, `X3_SVM_MIN_SIGNATURES`, `X3_SVM_MIN_STAKE_BPS` are strictly parsed when their corresponding verifier family is selected and the setting is present. Zero, empty, negative, non-integer, overflow and out-of-range values in that family cause `X3_BRIDGE_INIT_FAILED`, never an implicit weaker/default value. Defaults apply only when a setting is completely absent. Signature and stake bounds are 1–10,000; confirmation and block-number bounds are 1–u64::MAX.
- Explicitly configured `http://localhost` is still allowed for a deliberate **local test validator**, not inferred automatically. Presence of an RPC URL alone does not authenticate an endpoint; independent chain/receipt/finality checks are still required.

## Adversarial regression

Run from repository root:

```bash
cargo test --manifest-path x3-lang/Cargo.toml -p x3-lang-vm \
  --lib bridge::tests:: --locked -- --test-threads=1
```

The tests restore all modified environment variables on every exit; serial execution avoids process-global configuration race conditions in the test runner. They first demonstrated actual failed assertions against the old production backend and unknown/unset simulation defaults, and then passed on the corrected code.

**Release boundary:** This patch changes config parsing, not VM semantics, deployed Guard authorization, native BSC/NEAR escrow, or the Tidepool testnet trading proof. It must not be used as evidence that an unrestricted agent wallet key is non-bypassable. This remains a separate security architecture task.
