# x3-guard

The X3 policy of a NEAR Intents swap, enforced on NEAR.

`x3-lang/near_intents.py` lowers an X3 intent to a 1Click quote request and checks the quote
against the program's guards. This contract makes those decisions public:

| Call | What it does |
|---|---|
| `register_intent(nonce, policy)` | Stores the program's policy under `(caller, nonce)`: program SHA-256, assets, amount, `min_receive`, slippage bound, fee cap, receivers, deadline. A nonce registers once. |
| `accept_quote(nonce, signed_payload, signature)` | Verifies 1Click's Ed25519 signature **on chain** against the pinned manager key, reads the quote out of the verified bytes, runs every guard against the stored policy and records the quote with its signed payload. A policy accepts **one** quote, so a nonce opens one deposit address. A forged or failing quote changes nothing. |
| `record_outcome(nonce, status, …)` | Records how the swap ended, once, and whether a settlement met the guaranteed floor. |
| `get_intent`, `is_nonce_used`, `check_quote`, `get_manager_public_key` | Views. `check_quote` dry-runs the signature check and the guards. |

Events are NEP-297 (`standard: "x3guard"`): `intent_registered`, `quote_accepted`,
`outcome_recorded`.

**Trust model.**

- Every value a quote guard reads comes from the payload whose signature verified under the
  1Click manager key, `ed25519:reYaWhvwu8Jzo3WUM3zhn6VrhuMEF4eADL17qtRVifc`, the key the
  official SDK (`@defuse-protocol/one-click-sdk-typescript` 0.1.26) pins. The signed message is
  `base58(sha256(payload))` as UTF-8, exactly as the SDK's `verifyQuoteSignature` checks it. A
  caller cannot change an amount, receiver, deposit address or deadline without the
  signature failing. The tests verify two real 1Click-signed quotes on chain under this key.
- The key is set by `new()` and has no setter. The contract has no owner or admin, so a key
  rotation means deploying new code.
- Only a live quote is accepted. A dry quote does not sign a deposit address, deadline, time
  estimate or refund fee.
- The outcome is the owner's attestation of `GET /v0/status`. 1Click does not sign status
  responses, and the contract does not observe the destination chain. It does record whether a
  settlement met the *signed* guaranteed minimum.
- Every write's storage is paid by the caller from the attached deposit, and the excess is
  refunded in the same call.

## Test

```bash
cd x3-lang/near/x3-guard
cargo test          # 27 unit tests on near-sdk's mocked chain
cargo clippy --all-targets -- -D warnings
```

The tests include two real 1Click-signed quotes, verified under the production key, and the
exact arguments `near_intents.py guard-args` generates (`x3-lang/tests/fixtures/x3_guard_args.json`,
signed with a test key both suites derive from the same seed), so the adapter and the contract
are tested against each other.

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

Order: live quote → `register` → `quote` (the contract verifies the signature and re-checks
every guard) → deposit from your wallet → `status --wait` → `outcome`. The program's `timeout` is
the policy deadline, and a quote is fresh for 5 minutes, so register and accept promptly.
