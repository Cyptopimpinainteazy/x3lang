# X3 Shield USDC vault candidate

This separate contract is for a fresh testnet deployment. The owner pins a
six-decimal NEP-141 token, a merchant, an agent, spending limits, an expiry and
the owner's .x3 source digest at initialization. The vault enforces its stored
terms directly. The digest identifies the mandate for evidence. The contract
does not compile or run .x3, call the separate X3 Guard or route swaps.

The build checks the testnet account suffix. An operator must separately
verify network identity and the token's code and metadata before deploying.
Deployment and initialization must be atomic. The vault account's deployment
key can change code and must stay outside the agent runtime. Use a dedicated
testnet agent with a FunctionCall key for the vault's `execute` method and
zero attached deposit. Register vault, merchant and withdrawal recipient with
the pinned token before testing. Only owner transfers via `ft_transfer_call`
with an empty message credit escrow. Uncredited direct transfers are outside
the accounting ledger.

## Call interface

| Method | Caller | Contract behavior |
| --- | --- | --- |
| `new({mandate})` | Vault deployment account | Pins immutable roles and limits. Amounts and expiry are decimal strings. |
| `execute({nonce, recipient, amount})` | Agent | Calls the pinned token's `ft_transfer` to the fixed merchant. Nonce and amount are decimal strings. |
| `set_paused({paused})`, `revoke({})` | Owner | Blocks new payments. Revocation is permanent. |
| `withdraw({amount})` | Owner | Transfers available escrow back to the owner when paused, revoked or expired. |
| `next_nonce`, `pending`, `available`, `spent`, `mandate` | Read-only | Exposes the execution cursor, reservation, credited escrow, lifetime settled agent spend and pinned terms. |

The ceilings are 100,000 units per payment and 10,000,000 units per rolling 24
hours. A deployment can choose smaller caps. The owner supplies a separate
lifetime cap. The rolling window starts for each payment at its successful
callback, which conservatively accounts for payout latency. There are at most
128 settled payment records in the window. Only one transfer can be pending,
including owner withdrawals. The nonce is consumed at reservation and stays
consumed on an explicit token failure. A missing or ambiguous callback holds
the reservation and blocks the vault. Pause or revocation cannot cancel an
already scheduled payout. There is no force-unlock method.

## Verification

From this directory run:

```sh
cargo fmt --check
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
cargo build --locked --release --target wasm32-unknown-unknown
```

A unit-test pass covers the local contract logic. Before public testing,
exercise compiled WASM on NEAR Sandbox with a NEP-141 token, verify the actual
token receipts and balance changes, and inspect scoped-key rejection at the
protocol layer. Confirm gas, callback execution and storage for the exact
WASM. Public testnet and mainnet deployment require separate owner-signed
transactions. Use independent review for custody, token selection and upgrade
authority before any deployment.
