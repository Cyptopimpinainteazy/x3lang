# x3-guard local NEAR Sandbox test

This test deploys the **actual `x3-guard` contract WASM** into a local
[`near-sandbox`](https://github.com/near/near-sandbox-rs) node and exercises
the mandate boundary against a real NEAR runtime:

1. `create_mandate` by the owner.
2. `agent_register_intent` for an allowed recipient — accepted.
3. `agent_register_intent` for `attacker.near` — the read-only
   `check_agent_intent` view reports `mandate_recipient: false`, and the
   transaction fails on-chain. NEAR reports the WASM trap; the trap does not
   carry the Rust panic string, so the view names the exact guard.
4. `revoke_mandate`, then a post-revocation registration — the view reports
   `mandate_active: false` and the transaction fails on-chain.

It prints the sandbox transaction/receipt outcomes for inspection.

## Scope

**Local only.** Every account, transaction hash and receipt is sandbox-local.
This is **not** a public testnet or mainnet transaction and must never be
presented as one. It spends no public funds.

Not covered: the 1Click quote-accept path. That needs a fresh, live
pinned-manager-signed quote; the contract's quote guards are covered by the
Rust unit tests. This test covers the deployed-WASM mandate accept/refuse path.

## Run

From the repository root:

```bash
bash scripts/guard-sandbox-test.sh
```

Or manually:

```bash
(cd x3-lang/near/x3-guard && cargo build --locked --release --target wasm32-unknown-unknown)
(cd x3-lang/near/x3-guard/sandbox-test && cargo test -- --nocapture)
```

The test reads `X3_GUARD_WASM` if set, otherwise
`x3-lang/near/x3-guard/target/wasm32-unknown-unknown/release/x3_guard.wasm`.
The first run downloads a local `near-sandbox` binary. The wasm target must be
installed:

```bash
rustup target add wasm32-unknown-unknown
```

No wallet, key file, public RPC or public network is used by the test. The
sandbox genesis key is the public test key from `near-sandbox`; it is not a
treasury key.
