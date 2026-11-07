# x3-guard live on NEAR testnet — accept / refuse / revoke (2026-11-06)

The guard contract is deployed to NEAR testnet and the mandate lifecycle was
exercised with real signed transactions. This closes the testnet-deploy half of
the submission's fail-closed blockers. It is **testnet**, not mainnet, and no
wallet transfer, live settlement, or cross-chain causality is claimed.

## Deploy

- Contract account: `x3lang.testnet`
- Deploy transaction: `Fx7uuvRpQMbpfgyYmCAJiP6Y7RXEB98dYTExBcHKmYRP`
- Wasm: 544,944 bytes, built with
  `cargo build --target wasm32-unknown-unknown --release --locked`
  from `x3-lang/near/x3-guard`
- On-chain code hash: `CbU8SpgfTgG1egbw8ygMTK3deozvDnoFrF8DJ8qifSe6`
  (= sha256 `ac44fa53acf514d26c7adf883b31b49a125faf5a4c1074928d3e1fe94b36f8ab`)
- `state_version`: 2 (matches `STATE_VERSION` in the repo)
- Pinned 1Click manager key: `ed25519:reYaWhvwu8Jzo3WUM3zhn6VrhuMEF4eADL17qtRVifc`

## Mandate lifecycle (owner `x3lang.testnet`, agent `x3lang-guard-20261106.testnet`)

| Step | Result | Transaction |
| --- | --- | --- |
| `create_mandate` | Mandate stored, `version: 1` | owner `x3lang.testnet` |
| Refuse: `agent_register_intent` to `attacker.near` | Refused — `mandate_recipient` | `AhVBD7oXfwsixBipupjCzuhNdqunjdXJnLz3LXnmfnN8` |
| Accept: `agent_register_intent` to `x3lang.testnet` | Accepted, `intent_registered` | `DPJfcRUGk56h8sSsFsDFFgUMQAiyBCtpMuvquF9FvTct` |
| `revoke_mandate` | Mandate now `revoked: true` | owner `x3lang.testnet` |
| Post-revoke `agent_register_intent` | Refused — `mandate_active` | refused (trap) |

## Refusal verdict (dry-run, `check_agent_intent`)

The agent intent paying `attacker.near` is refused by the recipient guard, not
by any model-level choice:

```text
guard: mandate_recipient
ok: false
detail: attacker.near, allowed ["x3lang.testnet"]
```

All other guards on that intent pass (`mandate_asset`,
`mandate_destination_asset`, `mandate_refund_to`, `mandate_per_payment`,
`mandate_daily`, `mandate_slippage`, `mandate_fees`, `mandate_deadline`), which
is exactly the point: the payment is legal except for the injected recipient,
and only `mandate_recipient` stops it.

## Reproduce

```bash
npx near-cli view x3lang.testnet get_mandate \
  '{"owner":"x3lang.testnet","agent":"x3lang-guard-20261106.testnet"}' --networkId testnet
npx near-cli view x3lang.testnet get_intent \
  '{"owner":"x3lang.testnet","nonce":"accept_demo_001"}' --networkId testnet
```

No seed phrase or private key is stored in the repository. Testnet account IDs,
transaction hashes, and the deployed code hash are public data.
