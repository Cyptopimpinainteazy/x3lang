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

CI builds it: the **DX3HACK Hackathon Coin (Unsigned Build)** workflow
(`.github/workflows/x3-dx3hack-coin.yml`) runs fmt, clippy and the tests, checks the burn
program with `x3c` (and that its bytecode matches `x3/dx3hack_burn.x3bc`), builds the wasm
with `--cfg near`, confirms it imports `env.panic_utf8` so refusal reasons reach the chain,
and uploads `dx3_hackathon_coin.wasm`, `SHA256SUMS` and `PROVENANCE.txt`. No keys, no chain
activity. Download that artifact; its `SHA256SUMS` is what you pass to `setup.sh`.

To build locally instead, from this directory (its `.cargo/config.toml` sets `--cfg near`;
`src/lib.rs` refuses to compile for wasm32 without it):

```
cargo build --locked --release --target wasm32-unknown-unknown
sha256sum target/wasm32-unknown-unknown/release/dx3_hackathon_coin.wasm
```

## Set it up (testnet, then mainnet)

Three owner-run scripts. They need the near CLI (near-cli-rs; commands checked against
0.31.1) with `x3lang.<network>` in its keychain, plus `jq`, `curl`, `openssl` and
`python3`. Every signing step prints its cost and waits for you to type `yes`. Every read
goes straight to NEAR RPC, and each step is verified before the next. Re-running skips
what's already done.

```
deploy/setup.sh testnet ./dx3_hackathon_coin.wasm <sha256 from SHA256SUMS>
deploy/smoke.sh testnet
# ...check everything, then the same two on mainnet...
deploy/lock.sh  mainnet        # permanent; only after the mainnet smoke test passed
```

**`setup.sh`**

- *Preflight:* the wasm hash matches; the wasm reports refusal reasons; the args file binds `x3/dx3hack_burn.x3`; the burn is in the future; the accounts in the args file are the expected ones.
- *Accounts it creates:* `dx3hack.x3lang.<network>` (7 NEAR: about 4.1 locks the 412 KB code, the rest stores up to 500 holders), `faucet.x3lang.<network>` (0.5 NEAR) and `judges.x3lang.<network>` (2 NEAR).
- *Deploy:* deploys with `init-args.json` (mainnet) or `init-args.testnet.json` (testnet), then reads `hackathon_config` and `ft_metadata` back and compares every field.
- *Faucet key:* generates the faucet operator key with openssl into `~/.x3/dx3hack/faucet-key.<network>.json` (mode 600, never overwritten), adds it to the faucet account scoped to `add_holder` and `faucet_claim` on the token only, and verifies that scope on chain.
- *Extra holders:* set `DX3HACK_EXTRA_HOLDERS="merchant.x3lang.near ..."` to allowlist owner-approved accounts.

**`smoke.sh`**

Signs six transactions with the faucet key (the CLI only signs; the script broadcasts with
`send_tx`, so refused calls still yield a hash):

- *Should pass:* add a judge holder; one faucet claim.
- *Should be refused:* a second claim inside the cooldown, a lookalike parent account, a nested subaccount, and the Ref Finance DEX account.

Each outcome and refusal reason is checked against RPC, and the hashes go to
`~/.x3/dx3hack/smoke-<network>.json`, ready to add to the judge site's evidence.

**`lock.sh`** deletes every full-access key on the token account after you type its name.
After that nobody, including you, can change the code, the burn time or the bound `.x3`
program. Owner calls from `x3lang.<network>` (pause, `end_now`, `add_holder`, `mint`) keep
working. There is no undo.

Accounts after setup:

| Account | Role | Keys after setup |
|---|---|---|
| `dx3hack.x3lang.near` | The token contract | None, after `lock.sh` |
| `x3lang.near` | Owner | Yours |
| `faucet.x3lang.near` | Faucet operator, used by the runner | Its own key plus one function-call key: `add_holder`, `faucet_claim` on `dx3hack.x3lang.near` only |
| `judges.x3lang.near` | Parent of every judge holder | Runner creates `j<n>.judges.x3lang.near` under it |

If the faucet key leaks, the holder can only allowlist `*.judges.x3lang.near` accounts and
claim within the contract's caps, from a 0.5 NEAR gas allowance.

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
