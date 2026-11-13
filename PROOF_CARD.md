# Proof card: invoice_06 on NEAR and Solana mainnet

One page for judges. Every value below is in the committed evidence at
[`4f60ddb`](https://github.com/Cyptopimpinainteazy/x3lang/tree/4f60ddb652188784ed44f64d8172cd2cd84da422)
([CI run 43376851817](https://github.com/Cyptopimpinainteazy/x3lang/actions/runs/43376851817):
python-and-browser, provenance, rust-vm and near-guard all passed). This card adds no new claim;
the evidence bundle is [`evidence/near-guard-mainnet-invoice06-2026-11-12/`](evidence/near-guard-mainnet-invoice06-2026-11-12/README.md).

**In one sentence:** the owner wrote a mandate in `.x3`, a restricted agent key got one payment
past the mainnet Guard's 21 checks on a signed 1Click quote, the owner funded 25 USDC on Solana,
`intents.near` paid 24.9952 USDC to `x3lang.near`, and the agent recorded the outcome on Guard.

## Accounts, keys and code

| What | Value | Note |
|---|---|---|
| Guard contract | `x3lang.near` | NEAR mainnet |
| Guard code hash | `BPdpJAw8R4SDh6Wi7dPcXTs5T7ncJDVKCLgzuqZFb15T` | Deployed in [`BVXX3hJE…3faF`](https://nearblocks.io/txns/BVXX3hJEmThWVZNpUknZv9HCpGezisiN8SQKsZgA3faF); WASM built from reviewed commit `9dcea3c` ([deployment evidence](evidence/near-guard-mainnet-2026-11-11/deployment-verification.json)) |
| Agent account | `x3-agent.x3lang.near` | Two keys on the account: |
| Agent key that signed | `ed25519:J3NiU8J63HAfz2MTDu1zQVhFRYS5Pio21h1kNtJdP11x` | FunctionCall, receiver `x3lang.near`, only `agent_register_intent`, `agent_accept_quote`, `agent_record_outcome` |
| Agent recovery key | `ed25519:8teaXGYFELKQN5QA7gQqeA4B7URrcMQqaczZ5D68c23u` | **FullAccess, still present.** Signed none of the calls below |
| Funding wallet (Solana) | `58tbMfadmi8HWzUndLqkX3prBbkiZMEaYMTDYSzhB3cQ` | Owner-operated (first-party records); also the mandate's `refund_to` |
| Payment program | [`payment-proposed.x3`](evidence/near-guard-mainnet-invoice06-2026-11-12/source-recovery/payment-proposed.x3) · SHA-256 `9ee29a5603cfa1e7d9fdbd866cf330ce35e6b5f0907c6844dd2ef1a213768ba0` | Equals `get_intent` → `policy.program_sha256` |
| Mandate program | [`mandate-proposed.x3`](evidence/near-guard-mainnet-invoice06-2026-11-12/source-recovery/mandate-proposed.x3) · SHA-256 `eea021d5cc1118801f4e1d3b9c0efe89fc59a7f654031f9ff1951aa1f2f3d515` | Equals `get_mandate` → `terms.program_sha256` (version 4) |

## The five transactions

| # | Step | Transaction | Block / slot | Signer | What it proves | What it does **not** prove |
|---|---|---|---|---|---|---|
| 1 | Owner approval (`approve_intent`) | [`GFdQLrzT…fsK6`](https://nearblocks.io/txns/GFdQLrzT5tnPPdZwKLtmwpNvSwrxVHDvVg2C3WX5fsK6) | NEAR 224148959 | `x3lang.near` | The owner approved this intent before any agent call | Anything about the agent's later behaviour |
| 2 | Guard accepts the quote (`agent_accept_quote`) | [`Eg75uehw…u153`](https://nearblocks.io/txns/Eg75uehwU4YJ86ZZPEPKiBqnjkr6PxwPmV23HFAgu153) | NEAR 224149400 | Agent FunctionCall key | Guard verified the 1Click manager signature and returned 21 guards, all `ok` (receipt `SuccessValue`), then stored the quote: 25 USDC in, 24.9952 out, guaranteed minimum 24.770243 against the intent floor of 24.75 | That any funds moved |
| 3 | Deposit | [`4zhGYyWN…RkXo5`](https://solscan.io/tx/4zhGYyWNabYQ5gixTUzoVHVbgFKWAsQssSpCv3B1J1LjCRisEJW6wm3zuS2GVuDotAT93S8wm1qezHwsKX2RkXo5) | Solana slot 468354078 | Funding wallet `58tbMf…B3cQ` | 25.000000 USDC reached the quoted deposit address, finalized | That Guard or the agent sent it: the owner signed it manually |
| 4 | Payout (`execute_intents`) | [`8jfbACLS…w43P`](https://nearblocks.io/txns/8jfbACLSxrzi3fnajfAVGc4Fgo6XLCWA1mpiJWXPw43P) | NEAR 224150052 | `intents.near` (solver) | 24.995200 native USDC reached `x3lang.near`, 84 seconds after the deposit | A cryptographic link to the deposit: the association is 1Click status, timing and amount |
| 5 | Outcome (`agent_record_outcome`) | [`9sTn9pJ1…zqE`](https://nearblocks.io/txns/9sTn9pJ1ZbCVazBm9Wz9oEDh6aWw4FUVMoAJEKCCzuqE) | NEAR 224319990 | Agent FunctionCall key | Guard stored `Settled`, `amount_out` 24995200, `floor_met: true` and both settlement hashes ([readback](evidence/near-guard-mainnet-invoice06-2026-11-12/outcome-recording/README.md)) | That the hashes are true: Guard does not authenticate them |

## Seven limits

1. The owner manually signed the Solana deposit. First-party records identify the funding wallet
   as the owner's; the chain shows the signer, not who controls it.
2. Guard does not hold or move the USDC. A holder of the funding wallet's key could pay without Guard.
3. The agent account still has the owner's FullAccess recovery key next to the restricted key.
4. The outcome is the agent's report. Guard stores the supplied hashes and compares the amount to
   the floor; it does not authenticate the hashes.
5. The deposit and the payout are linked by 1Click status, timing and amount, not by a
   cryptographic cross-chain proof. The two legs are not atomic.
6. Guard checks stored `.x3` hashes; it does not compile or run `.x3`. X3VM is not on the
   deployed authorization path.
7. One settlement of about $25. It does not establish production reliability.

## Check it yourself

```bash
git clone https://github.com/Cyptopimpinainteazy/x3lang && cd x3lang
git checkout 4f60ddb652188784ed44f64d8172cd2cd84da422
cd evidence/near-guard-mainnet-invoice06-2026-11-12/source-recovery && sha256sum --check SHA256SUMS && cd -
python3 scripts/hackathon/observe_mainnet_invoice06.py --out /tmp/observation.json   # read-only, public RPC
```

The observer prints 18 passed, 0 failed and 1 not readable on chain (who controls the funding
wallet). It predates the outcome call; step 5 has its own [readback](evidence/near-guard-mainnet-invoice06-2026-11-12/outcome-recording/README.md#read-only-reproduction).
### Link by link, offline

On current `master` (the script was added after `4f60ddb`; it reads only the committed evidence):

```bash
python3 scripts/hackathon/invoice06_chain.py          # --json for the full report
```

It runs the shipped `.x3` compiler on the two mainnet files and shows that its output equals what
Guard stores on mainnet: all 10 policy fields on the intent and all 13 mandate terms. It then edits
the same payment eight ways and shows where each edit stops: paying `attacker.near`, ten times the
amount, a redirected refund or looser slippage are refused by the mandate check; dropping the
slippage bound, the timeout or the nonce, or adding a second swap leg, fail to compile. Finally it
reads the 21 passing Guard checks, the deposit and payout, and the recorded outcome from the
committed receipts, and prints where the chain is not cryptographic (the limits above).

Testnet and local evidence (the attacker refusal, the Base Sepolia trade, the red-team runs) is
listed separately in the [README](README.md) and is not mainnet.

## Reproduce the mainnet code hash

The Guard bytes on `x3lang.near` rebuild exactly from this repository's source. Needs Rust
1.99.1 (pinned by the folder's `rust-toolchain.toml`) plus its `rust-src` component:
`rustup component add rust-src --toolchain 1.99.1`. No key, wallet or RPC.

```bash
cd x3-lang/near/x3-guard
RUSTFLAGS="--cfg near --remap-path-prefix=$(rustc --print sysroot)/lib/rustlib/src/rust=/rustc/58cf8f9c35b355199727627d07ae34a4e2846d67 --remap-path-prefix=${CARGO_HOME:-$HOME/.cargo}/registry/src=/home/lojak/.cargo/registry/src" \
  cargo build --locked --release --target wasm32-unknown-unknown
python3 - <<'PY'
import hashlib
wasm = open("target/wasm32-unknown-unknown/release/x3_guard.wasm", "rb").read()
digest = hashlib.sha256(wasm).digest()
alphabet = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"
n, b58 = int.from_bytes(digest, "big"), ""
while n:
    n, r = divmod(n, 58)
    b58 = alphabet[r] + b58
print(len(wasm), digest.hex(), b58)
PY
```

Expected: `572248 9a614640ac8bc39bbbcb1674ce0f4fe2f51610ac7ba828c9c4574be92712a632
BPdpJAw8R4SDh6Wi7dPcXTs5T7ncJDVKCLgzuqZFb15T`. The last value is the code hash nearblocks
shows for `x3lang.near`. The two `--remap-path-prefix` flags only rewrite compiler
source-path strings to the ones used in the original build; without them the bytes differ
([why](docs/hackathon/GUARD_WASM_BYTE_REPRODUCIBILITY.md#why-a-plain-rebuild-originally-produced-696-additional-bytes)).
CI runs the same rebuild and hash assertion in the "Reproduce reviewed scoped-storage upgrade
bytes" step of [`x3guard-real-wasm-sandbox.yml`](.github/workflows/x3guard-real-wasm-sandbox.yml),
against the [reviewed manifest](audit-artifacts/guard-scoped-storage-20261110/verification.json).
