# x3-guard

The X3 policy of a NEAR Intents swap, enforced on NEAR.

`x3-lang/near_intents.py` lowers an X3 intent to a 1Click quote request and checks the quote
against the program's guards. This contract makes those decisions public:

| Call | What it does |
|---|---|
| `register_intent(nonce, policy)` | Stores the program's policy under `(caller, nonce)`: program SHA-256, assets, amount, `min_receive`, slippage bound, fee cap, receivers, deadline. A nonce registers once. |
| `accept_quote(nonce, signed_payload, signature)` | Verifies 1Click's Ed25519 signature **on chain** against the pinned manager key, reads the quote out of the verified bytes, runs every guard against the stored policy and records the quote with its signed payload. A policy accepts **one** quote, so a nonce opens one deposit address. A forged or failing quote changes nothing. |
| `record_outcome(nonce, status, …)` | Records how the swap ended, once, and whether a settlement met the guaranteed floor. |
| `get_intent`, `is_nonce_used`, `check_quote`, `get_manager_public_key`, `state_version` | Views. `check_quote` dry-runs the signature check and the guards. |

Events are NEP-297 (`standard: "x3guard"`, version 1.3.0): `intent_registered`, `quote_accepted`,
`outcome_recorded`, `mandate_created`, `mandate_revoked`, `intent_approved`, `state_migrated`.

## Agent mandates

An owner can let an AI agent's NEAR account swap in the owner's name, inside limits the owner
sets once. The mandate is written in X3 (`x3-lang/examples/near_mandate_agent.x3`) and
compiled by `near_intents.py mandate args`.

| Call | Who | What it does |
|---|---|---|
| `create_mandate(agent, terms)` | owner | Stores the terms for that agent. Replacing them increases the version, which voids every intent the agent registered under the old terms and has not had accepted. |
| `revoke_mandate(agent)` | owner | Stops the agent at once: nothing it registered can be accepted any more. A quote already accepted is not undone, since its deposit address exists. |
| `agent_register_intent(owner, nonce, policy)` | agent | Registers an intent under the owner, after checking every term. |
| `approve_intent(owner, nonce)` | an approver | Signs off on a payment above `approval_above`. The agent is never an approver. |
| `agent_accept_quote(owner, nonce, signed_payload, signature)` | agent | Checks the mandate again (revocation, expiry, version, approvals, rolling spend), then the quote exactly as `accept_quote`, and counts the payment. |
| `agent_record_outcome(owner, nonce, …)` | agent | As `record_outcome`; the owner can use `record_outcome` itself. |
| `get_mandate`, `check_agent_intent` | anyone | Views. `check_agent_intent` dry-runs the terms for a policy. |

The terms checked, in order (`mandate::check_mandate`, mirrored check for check by
`near_mandate.py` and the browser demo):

| Check | Refuses |
|---|---|
| `mandate_active` | a revoked or expired mandate |
| `mandate_asset` | spending anything but the mandate's one origin asset |
| `mandate_destination_asset`, `mandate_recipient` | paying out in an asset, or to an account, not on the owner's lists |
| `mandate_refund_to` | a refund address other than the owner's. Without this, an agent could name itself as the refund address and fail the swap on purpose |
| `mandate_per_payment` | a payment over the per-payment cap |
| `mandate_daily` | going over the cap in **any rolling 24 hours**, or a 65th payment in 24 hours |
| `mandate_slippage`, `mandate_fees` | a looser slippage bound, or a missing or looser fee cap, than the owner allows |
| `mandate_deadline` | an intent whose deadline outlives the mandate |
| `mandate_approvals` (at acceptance) | a payment above `approval_above` without N of M approvals |

A payment is counted when its quote is accepted, because that is when a deposit address is
authorized. A refunded swap is not credited back, so the counter errs toward spending less.
The owner cannot accept an agent's intent with `accept_quote`, so the counter cannot be
bypassed from either side.

**What a mandate does not do.** The contract holds no funds. The deposit is sent from the
owner's wallet on the origin chain. A mandate binds any wallet or runtime that deposits only to
an address x3-guard accepted, which is what `near_intents.py` reports as
`eligible_for_wallet_authorization`. It cannot stop someone who holds the origin-chain wallet
key from sending funds directly, so give the agent a NEAR account and never the wallet key.

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
cargo test          # 42 unit tests on near-sdk's mocked chain
cargo clippy --all-targets -- -D warnings
```

The tests include two real 1Click-signed quotes, verified under the production key, and the
exact arguments `near_intents.py guard-args` generates (`x3-lang/tests/fixtures/x3_guard_args.json`,
signed with a test key both suites derive from the same seed), so the adapter and the contract
are tested against each other. The mandate tests do the same with
`x3-lang/tests/fixtures/x3_guard_mandate_args.json`: the adapter's `create_mandate` and agent
calls run the full lifecycle on the contract, and the contract must reach the adapter's verdict
on every policy in it.

## Build

```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/near/cargo-near/releases/latest/download/cargo-near-installer.sh | sh
cd x3-lang/near/x3-guard
cargo near build non-reproducible-wasm      # -> target/near/x3_guard.wasm
```

This folder's `rust-toolchain.toml` uses `stable` with the `wasm32-unknown-unknown` target,
overriding the repository's 1.90 pin. cargo-near checks the rustc version against what NEAR's
protocol accepts and says so if the toolchain is too new.

## State upgrades

The current schema is `STATE_VERSION = 2`. `migrate` accepts both persisted layouts that
predate versioning:

- the original Guard state with intents and the manager key, whose records become unbound
  owner intents;
- the mandate-era state with intents, the manager key and mandates, preserving agent bindings,
  mandate versions and approvals.

The migration preserves both `LookupMap` prefixes and the pinned manager key, creates the
mandate map when absent, records a `state_migrated` event, and refuses an already-current or
future schema. Unit tests serialize both old layouts, run the migration, reload the new state,
and check that intents, authorizations and mandates survive.

Upgrading a deployed contract is an operator action and has not been performed by this
repository. Snapshot state first, then deploy the replacement code and call `migrate` as the
initializer. Do not call `new()` over existing state:

```bash
near contract deploy <contract-account> use-file target/near/x3_guard.wasm \
  without-init-call network-config mainnet sign-with-access-key-file <key-file.json> send
near contract call <contract-account> migrate '{}' --gas 30Tgas --deposit 0 \
  network-config mainnet sign-with-access-key-file <key-file.json> send
```

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
every guard) → deposit from your wallet → `status --wait` → `outcome`.

As an agent, the owner sends the mandate once, and the agent's receipts select the agent
methods by themselves:

```bash
near_intents.py mandate args mandate.x3 --contract <contract-account> --key-file <owner-key.json>
near_intents.py mandate check mandate.x3 intent.x3 --guard-contract <contract-account>   # exit 6 if refused
near_intents.py quote intent.x3 --live --mandate mandate.x3 --guard-contract <contract-account>
near_intents.py guard-args --receipt <receipt> --step register|approve|quote|outcome \
  --contract <contract-account> --signer <agent-or-approver> --key-file <key.json>
near_intents.py mandate revoke mandate.x3 --contract <contract-account> --key-file <owner-key.json>
```

A live quote for an intent `under mandate` needs `--mandate`, and with `--guard-contract` it
reads the mandate the contract holds (revocation, version, what was spent in the last 24 hours).
A payment the mandate refuses is never quoted, so it never gets a deposit address. The program's `timeout` is
the policy deadline, and a quote is fresh for 5 minutes, so register and accept promptly.

## Dependency allocator safety

NEAR SDK 5.30 enables the abandoned `wee_alloc` allocator by default for
WebAssembly. This contract explicitly disables that SDK feature (including
its test dependency) and uses Rust's default `wasm32` allocator. The
corresponding dependency lockfile removes `wee_alloc` and its unused
transitive packages. The CI gate `bash scripts/check-near-allocator.sh`
checks both the wasm32 dependency graph and `Cargo.lock` against regression.

The original advisory [RUSTSEC-2022-0054](https://rustsec.org/advisories/RUSTSEC-2022-0054.html)
is an **unmaintained allocator warning**, classified informational by RustSec;
GitHub displays it as critical. Removing this one dependency does not
constitute a security audit or prove the contract safe to deploy.
