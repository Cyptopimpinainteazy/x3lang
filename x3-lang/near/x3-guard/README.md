# x3-guard

The X3 policy of a NEAR Intents swap, enforced on NEAR.

`x3-lang/near_intents.py` lowers an X3 intent to a 1Click quote request and checks the quote
against the program's guards. This contract makes those decisions public:

| Call | What it does |
|---|---|
| `register_intent(nonce, policy)` | Stores the program's policy under `(caller, nonce)`: program SHA-256, assets, amount, `min_receive`, slippage bound, fee cap, receivers, deadline. A nonce registers once. |
| `accept_quote(nonce, quote)` | Re-runs every quote guard on chain against the stored policy and records the quote (response hash, 1Click signature, deposit address). A policy accepts **one** quote, so a nonce opens one deposit address. A failing quote changes nothing. |
| `record_outcome(nonce, status, …)` | Records how the swap ended, once, and whether a settlement met the guaranteed floor. |
| `get_intent`, `is_nonce_used`, `check_quote` | Views. `check_quote` dry-runs the guards. |

Events are NEP-297 (`standard: "x3guard"`): `intent_registered`, `quote_accepted`,
`outcome_recorded`.

**Trust model.** The contract proves the submitted quote facts satisfy the policy and binds
them to the 1Click response by SHA-256 and signature. It does not verify 1Click's signature
(the signed message format is not published) and does not observe the destination chain: the
outcome is the owner's attestation of `GET /v0/status`. Every write's storage is paid by the
caller from the attached deposit; the excess is refunded in the same call.

## Test

```bash
cd x3-lang/near/x3-guard
cargo test          # 23 unit tests on near-sdk's mocked chain
cargo clippy --all-targets -- -D warnings
```

The tests include a real 1Click quote and the exact arguments `near_intents.py guard-args`
generates (`x3-lang/tests/fixtures/x3_guard_args.json`), so the adapter and the contract are
tested against each other.

## Build

```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/near/cargo-near/releases/latest/download/cargo-near-installer.sh | sh
cd x3-lang/near/x3-guard
cargo near build non-reproducible-wasm      # -> target/near/x3_guard.wasm
```

This folder's `rust-toolchain.toml` uses `stable` with the `wasm32-unknown-unknown` target,
overriding the repository's 1.90 pin. cargo-near checks the rustc version against what NEAR's
protocol accepts and says so if the toolchain is too new.

## Deploy

Deploying costs storage: about 1 NEAR per 100 KB of wasm, and cargo-near prints the size. Use
a dedicated account (e.g. `x3guard.near`) so the contract's name is what judges see.

```bash
near contract deploy <contract-account> use-file target/near/x3_guard.wasm \
  with-init-call new json-args '{}' prepaid-gas '30 Tgas' attached-deposit '0 NEAR' \
  network-config mainnet sign-with-access-key-file <key-file.json> send
```

## Use with the adapter

After `near_intents.py quote --live` has written a receipt, each step prints its exact call:

```bash
near_intents.py guard-args --receipt <intent>.near-receipt.json --step register \
  --contract <contract-account> --signer <your-account> --key-file <key-file.json>
# then --step quote, before depositing; --step outcome after `status --wait --receipt`
```

Order: live quote → `register` → `quote` (contract re-checks it) → deposit → `status --wait`
→ `outcome`. The program's `timeout` is the policy deadline, so register and accept within it.
