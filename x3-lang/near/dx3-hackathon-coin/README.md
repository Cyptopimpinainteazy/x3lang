# Dotx3 HACKATHON ONLY COIN (DX3HACK)

A demo credit for NEAR HACKATHON 2026 judges on dotx3.net. It has no value, it can't be
traded, and it burns itself one week after judging. Each of those rules is enforced by the
contract, not by a promise:

| Rule | How the contract enforces it |
|---|---|
| Can't be traded or pooled | Only allowlisted holders can register, send or receive. A DEX, bridge or random wallet is refused by `ft_transfer`, `ft_transfer_call` and `storage_deposit`. |
| Faucet can't be farmed | Only `faucet_operator_id` can mint, and it may only allowlist direct subaccounts of `holder_parent_id` (`j42.judges.x3lang.near`, never `pool.j42.judges...` or `v2.ref-finance.near`). Per-holder cooldown, a daily faucet cap, a hard max supply and a max holder count. |
| Trades limited per account | Every sender except the owner gets `max_transfers_per_day` transfers per UTC day. |
| Burns a week after the competition | At `expires_at` (2026-11-25 23:59:59 UTC) every balance and the total supply read as zero, and every state-changing method except `purge` is refused. The owner can only move the burn **earlier** (`end_now`). |
| Burn bound to an `.x3` program | The SHA-256 of `x3/dx3hack_burn.x3` is stored at deploy and can't be changed. `purge` deletes balances from storage, emits `ft_burn` events, is callable by anyone after expiry, and refuses any program text whose on-chain SHA-256 doesn't match. |

Metadata: name `Dotx3 HACKATHON ONLY COIN`, symbol `DX3HACK`, 6 decimals. NEP-141, NEP-145, NEP-148.

## The `.x3` burn program

`x3/dx3hack_burn.x3` burns the full 1,000,000,000 DX3HACK max supply on NEAR, with a refund
if the route fails. Checked against this repository at commit `470d61a`, from `x3-lang/`:

```
cargo build -p x3-tools --bin x3c
P=near/dx3-hackathon-coin/x3/dx3hack_burn.x3
./target/debug/x3c check $P                         -> 9 ops, no semantic errors
./target/debug/x3c build --out /tmp/burn.x3bc $P    -> 260 bytes (9 instructions)
./target/debug/x3c lint $P                          -> 0 errors, 0 warnings (1 info: single RPC quorum)
python3 cli.py $P                                   -> parses
```

| File | SHA-256 |
|---|---|
| `x3/dx3hack_burn.x3` (bound in the contract) | `220396425eadf9036b33ba15ebe4497687048bd385f523538d750dc0411e0c7e` |
| `x3/dx3hack_burn.x3bc` (bytecode) | `c8dca2c1e7388064afa01fdc316285e311f38045d3e9ed84402b84d4d68356eb` |

Changing even one byte of the `.x3` file changes the hash, and the contract refuses it.
The tests read the shipped file and fail if its hash or the deploy arguments drift.

## Tests

```
cargo test          # 39 unit tests: every rule above, plus the deploy args and the pinned .x3 hash
cargo clippy --all-targets
```

## Build

```
cargo install cargo-near            # once
cargo near build non-reproducible-wasm
# -> target/near/dx3_hackathon_coin.wasm
sha256sum target/near/dx3_hackathon_coin.wasm
```

## Deploy (testnet first, then mainnet)

For testnet, replace `.near` with your `.testnet` accounts in the commands and in a copy of
`deploy/init-args.json`. Keep `expires_at` and `burn_program_sha256` as they are.

Accounts:

| Account | Role | Keys after setup |
|---|---|---|
| `dx3hack.x3lang.near` | The token contract | **None.** The full-access key is deleted, so the code can never change. |
| `x3lang.near` | Owner (allowlists Guard and merchant, mints seeds, pause, `end_now`) | Yours |
| `faucet.x3lang.near` | Faucet operator, used by the runner | One function-call key: `add_holder`, `faucet_claim` on `dx3hack.x3lang.near` only |
| `judges.x3lang.near` | Parent of every judge holder | Runner creates `j<n>.judges.x3lang.near` under it |

1. Create the accounts (you sign each):

```
near account create-account fund-myself dx3hack.x3lang.near '5 NEAR' \
  autogenerate-new-keypair save-to-keychain sign-as x3lang.near network-config mainnet sign-with-keychain send
near account create-account fund-myself faucet.x3lang.near '0.5 NEAR' \
  autogenerate-new-keypair save-to-keychain sign-as x3lang.near network-config mainnet sign-with-keychain send
near account create-account fund-myself judges.x3lang.near '2 NEAR' \
  autogenerate-new-keypair save-to-keychain sign-as x3lang.near network-config mainnet sign-with-keychain send
```

The 5 NEAR on `dx3hack.x3lang.near` covers the code's storage stake plus storage for up to
500 holders, which the contract pays for when it registers them. The 2 NEAR on
`judges.x3lang.near` funds creating judge subaccounts.

2. Deploy and initialize in one transaction:

```
near contract deploy dx3hack.x3lang.near use-file target/near/dx3_hackathon_coin.wasm \
  with-init-call new file-args deploy/init-args.json \
  prepaid-gas '100 Tgas' attached-deposit '0 NEAR' \
  network-config mainnet sign-with-keychain send
```

3. Check the stored terms:

```
near contract call-function as-read-only dx3hack.x3lang.near hackathon_config json-args '{}' network-config mainnet now
near contract call-function as-read-only dx3hack.x3lang.near ft_metadata json-args '{}' network-config mainnet now
```

`burn_program_sha256` must read `220396…0c7e` and `expires_at` must read `1795651199000000000`.

4. Give the runner a faucet key that can do nothing else:

```
near account add-key faucet.x3lang.near grant-function-call-access \
  --allowance '0.5 NEAR' --contract-account-id dx3hack.x3lang.near \
  --function-names 'add_holder, faucet_claim' \
  autogenerate-new-keypair print-to-terminal network-config mainnet sign-with-keychain send
```

Put the printed private key on the runner. If it leaks, the attacker can only allowlist
`*.judges.x3lang.near` accounts and claim within the contract's caps.

5. Lock the contract. List its keys, then delete the full-access key:

```
near account list-keys dx3hack.x3lang.near network-config mainnet now
near account delete-keys dx3hack.x3lang.near public-keys <ed25519:KEY> \
  network-config mainnet sign-with-keychain send
```

After this, nobody (including you) can change the code, the burn time or the bound `.x3`
program. The dotx3.net "Who holds which key" panel shows `dx3hack.x3lang.near` with zero keys.
Do this only after the testnet run has shown every rule working, because there is no undo.

6. Allowlist the contracts that need DX3HACK and seed them (owner):

```
near contract call-function as-transaction dx3hack.x3lang.near add_holder \
  json-args '{"account_id":"merchant.x3lang.near"}' prepaid-gas '30 Tgas' attached-deposit '0 NEAR' \
  sign-as x3lang.near network-config mainnet sign-with-keychain send
```

## How judge accounts hold and spend it

NEP-141 requires exactly 1 yoctoNEAR on `ft_transfer`, and a function-call access key can't
attach a deposit. So a judge holder that moves DX3HACK must be a contract (the Guard-checked
vault calls `ft_transfer` and pays the 1 yoctoNEAR from its own balance) or an account
signing with a full-access key. The intended setup is the vault: the runner creates
`j<n>.judges.x3lang.near`, deploys the vault there with that judge's mandate, then calls
`add_holder` and `faucet_claim`. The agent only ever asks the vault, and the vault asks Guard.

Faucet flow per judge, from the runner (faucet key):

```
add_holder   {"account_id": "j42.judges.x3lang.near"}
faucet_claim {"receiver_id": "j42.judges.x3lang.near"}      -> 100 DX3HACK
```

Useful views for the site: `hackathon_config`, `is_holder`, `claim_available_at`,
`transfers_left_today`, `ft_balance_of`, `ft_total_supply`.

## After 2026-11-25 23:59:59 UTC

Nothing needs to happen: balances and supply already read zero and nothing can move. To
also clear storage and emit `ft_burn` events, anyone can run the bound program against the
holder list (up to 100 accounts per call). Holder ids come from the runner's feed or from
`ft_mint` events on NearBlocks.

```
jq -n --rawfile p x3/dx3hack_burn.x3 --argjson a '["j1.judges.x3lang.near","merchant.x3lang.near"]' \
  '{account_ids: $a, x3_program: $p}' > /tmp/purge.json
near contract call-function as-transaction dx3hack.x3lang.near purge file-args /tmp/purge.json \
  prepaid-gas '300 Tgas' attached-deposit '0 NEAR' \
  sign-as x3lang.near network-config mainnet sign-with-keychain send
```

`jq --rawfile` passes the file byte for byte, which keeps the hash identical.
