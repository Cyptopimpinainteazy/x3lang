# x3-lang — the X3 finance language

**Status: development. Not on chain yet.** Read this before the rest of the file.

## Where X3 code runs today (measured 2026-10-16)

There are two X3 instruction sets in this repository, and only one of them is on chain:

| | X3BC (chain bytecode) | x3-lang intent bytecode (this folder) |
|---|---|---|
| Source language | `fn` / `let` / loops / `atomic {}` (`crates/x3-parser`, `crates/x3-compiler`) | `atomic trade`, `risk policy`, `intent`, `asset` (`x3-lang/compiler`) |
| Instruction set | 114 opcodes, `crates/x3-backend/src/opcode.rs` | 84 opcodes, `x3-lang/spec/opcodes.yaml` |
| Engine | `crates/x3-vm` | `x3-lang/vm` |
| Runs on chain? | **Yes.** `pallet-x3-kernel` executes it through `X3Executor::execute_on_chain`, `x3-vm` in every build (#612) | **No.** No pallet loads it; it runs in this workspace's tests and `x3c` |

So a program written in x3-lang's trading syntax is compiled, verified, simulated and replayed here,
but it cannot be submitted to the chain. Compiling x3-lang to X3BC, with the trading, receipt and
oracle-firewall logic as host modules of the one engine, is the next unification step (PR 4 of the
X3 Lang mainnet roadmap).

## Source-of-truth rules

- Python files: authoritative for the shipping intent DSL and MVP pipeline.
- `x3-lang/Cargo.toml`, `compiler/`, `vm/`, `crates/x3-ast`, `crates/x3-common`: authoritative for Rust compiler, IR, opcode, and VM bytecode semantics.

## Python MVP files
- `cli.py`, `registry.py`, `typechecker.py`, `planner.py`, `simulator.py`, `emitter/`, `schema.json`, `runner.py`, `tests/`, `examples/arb_solana_eth.x3`

## Rust workspace files
- `Cargo.toml`, `compiler/`, `vm/`, `spec/`, `crates/x3-ast`, `crates/x3-common`, `crates/x3-lexer`, `crates/x3-tools`

## Trading Core v1

Trading Core v1 adds chain-qualified assets, linear debts, explicit swap
routes, risk-policy guards, debt-closure and net-profit verification,
capability-controlled atomic VM execution, and deterministic receipts.

Canonical example: `examples/trading_core_v1.x3`.

Supported today:

- parse, type-check, verify, lower, encode, decode, and disassemble trading declarations;
- checked decimal-to-base-unit accounting with explicit rounding controls;
- borrow/swap/repay/net-profit/all-debts/commit verification;
- fixture-host execution with atomic rollback and receipt verification;
- `x3c receipt inspect <receipt.json>` and `x3c receipt verify <receipt.json>`.

Explicit v1 boundaries:

- no live Aave, Uniswap, SushiSwap, bridge, or private-relay adapters;
- no route discovery or oracle aggregation;
- fixture capabilities are rejected in production/mainnet execution mode;
- no live-DEX or mainnet claim is made without real adapter evidence.

## x3-lang VM — what executes, and where

`x3-lang/vm` executes the intent instruction set below **in this workspace only** (tests, `x3c run`,
`x3c simulate`). None of it runs on a block.

| Opcode group | In `x3-lang/vm` |
|---|---|
| Arithmetic, memory, asset ops, control flow, guards, `ATOMIC_*`, capability dispatch | Executes |
| `BRIDGE` | Executes against the configured backend; the production backend fails closed without a live verifier |
| Trading core (`TRADING_*`) | Executes through `TradingHost`; **no live venue adapters** (see Trading Core v1) |
| Reserved `0x70`–`0x7B`, B-52 feature-lock payloads | Not dispatched |

What remains for mainnet is listed in the roadmap: one engine (x3-lang compiling to X3BC), live
adapters, multi-validator execution, an external audit.

## Bridge backend (x3-lang VM)

`init_production_backend()` in [`vm/src/bridge.rs`](vm/src/bridge.rs:2990) supports 4 verifier families:
- `evm-light-client` (EthereumLightClientVerifier)
- `svm-light-client` (SolanaLightClientVerifier)
- `evm-rpc` (EthereumRpcFinalityVerifier)
- `svm-rpc` (SolanaRpcFinalityVerifier)

All configured via environment variables. Never silently falls back to dry-run.

## NEAR Intents adapter (`near_intents.py`)

An `intent` whose route is one `bridge NearIntents <from> -> <to>` leg settles through the NEAR
Intents solver network via the 1Click API. The program's clauses become the quote request, and
the returned quote is checked against the program's guards before any deposit is made.

Canonical example: `examples/near_intents_sol_to_near.x3`.

```bash
python3 x3-lang/near_intents.py plan  x3-lang/examples/near_intents_sol_to_near.x3   # request only
python3 x3-lang/near_intents.py quote x3-lang/examples/near_intents_sol_to_near.x3   # dry quote + guard check
python3 x3-lang/near_intents.py quote <file.x3> --live      # deposit address, nonce consumed, receipt written
python3 x3-lang/near_intents.py submit-deposit --deposit-address <addr> --tx-hash <hash>
python3 x3-lang/near_intents.py status --deposit-address <addr> --wait --receipt <intent>.near-receipt.json
```

### BSC BNB → NEAR

`examples/bsc_bnb_to_near_wnear_dry.x3` is a 0.001 BNB dry reference using live provider asset
IDs. Its BSC refund address and `wrap.near` recipient are intentionally non-owned placeholders;
replace both before any wallet action. The adapter now checks all NEAR recipients, including
implicit accounts, on chain before asking 1Click for a quote.

```bash
PYTHONPATH=x3-lang python3 x3-lang/near_intents.py plan \
  x3-lang/examples/bsc_bnb_to_near_wnear_dry.x3
PYTHONPATH=x3-lang python3 x3-lang/near_intents.py quote \
  x3-lang/examples/bsc_bnb_to_near_wnear_dry.x3 \
  --out evidence/bsc-near-2026-11-05/quote-001-bnb.json
```

`quote --out` records the exact request on success or structured failure on refusal. A live
quote can be turned into non-broadcast Guard calls and evidence commands with
`near_intents.py prepare --receipt R --program P ...`; it requires the exact program source,
re-verifies the quote, and always reports `prepared_not_authorized`.

On 2026-11-05, the real 1Click endpoint refused 0.001 BNB with HTTP 400 and a temporary $1,000
minimum. The refusal evidence is committed; no amount was silently increased. See
[`docs/hackathon/BSC_TO_NEAR_BOOTSTRAP.md`](../docs/hackathon/BSC_TO_NEAR_BOOTSTRAP.md).

Every quote's 1Click Ed25519 signature is verified first, on dry and live quotes, against the
manager key the official SDK pins, over the exact payload the SDK signs; a forged or altered
quote fails with `X3_NEAR_INVALID_SIGNATURE` before anything else is read. Then, enforced on
every quote: freshness (≤ 5 min), echoed request, exact `amountIn`, `min_receive` floor on the guaranteed
output, slippage spread, `require fees` on the swap cost and the refund fee, and (live) a deposit
address and a deadline that fits the quote's own time estimate and the program's `timeout`. `require nonce unused` is
enforced by a local nonce store. Refused by code rather than approximated: multi-leg routes,
guards 1Click cannot enforce (`finality`, `proof_complete`, …), missing slippage or timeout,
refunds in another asset, and `on_fail halt`/`quarantine`.

First real swap: `x3-lang/near/first-swap.sh` walks one small swap end to end:
`setup` → `check` → `build` → `deploy` → `quote` (live quote, registered and accepted on NEAR) →
you send the USDC → `deposited <solana-tx>` (track, prove from both chains, record the outcome).
It never moves your funds and asks before every step that spends NEAR.

Chain evidence: `verify-deposit --receipt R` proves the finalized Solana or BSC origin transfer.
For BSC, transaction and receipt must agree on the exact native-BNB value, destination and
successful status, and the transaction block must be finalized on every configured BSC RPC.
`verify-settlement --receipt R [--refund-tx TX]` proves the NEAR delivery (balance rise from a
baseline taken at quote time) or the finalized origin-chain refund. A receipt's
`verified_outcome` is set only from chain data.

Browser demo (live at https://x3-guard.pages.dev): `x3-lang/near/demo` is a static page that compiles an intent, fetches a real
1Click dry quote, verifies its signature with WebCrypto and marks each program line with its
verdict. A parity suite keeps it identical to this adapter.

On-chain policy: `x3-lang/near/x3-guard` is a NEAR contract that stores the program's policy,
re-runs the quote guards on chain, accepts one quote per nonce and records the outcome.
`near_intents.py guard-args --receipt R --step register|quote|outcome` prints each call.

AI agents: an owner writes a `mandate` (`examples/near_mandate_agent.x3`) naming an agent's NEAR
account, the one asset it may spend, per-payment and rolling-24-hour caps, allowed recipients
and destination assets, a pinned refund address, the loosest slippage and fees, approvals above
a threshold, and an expiry. An agent's intent says `under mandate <owner>`
(`examples/near_intents_agent_payment.x3`). `near_intents.py mandate args|check|revoke`
compiles and checks it (`near_mandate.py`), and x3-guard enforces it on NEAR. The contract
checks every term when the agent registers and again when it accepts a quote, and the owner can
revoke the agent at once. The contract holds no funds; see its README for what that leaves to
the wallet.

### AI Money Agent (`near_agent.py`)

A real model writes the agent's X3 program from an English request. The model is a writer, never
a gate: what it writes is compiled and checked exactly like a hand-written program, so an unsafe
prompt-injected or mistaken model output is refused; a corrected in-mandate program may pass.

```text
English request -> model -> X3 intent (untrusted) -> compiler -> `under mandate` + issued nonce
  -> owner's mandate (file, or live from x3-guard) -> signed 1Click quote
  -> owner approval of the program's SHA-256 -> x3-guard on NEAR (guard-args steps)
```

```bash
export ANTHROPIC_API_KEY=...            # or X3_AGENT_PROVIDER=openai-compatible with
                                        # X3_AGENT_BASE_URL=http://localhost:11434/v1 X3_AGENT_MODEL=...
python3 x3-lang/near_agent.py pay "Pay x3-demo.near 25 USDC for invoice 4411" \
  --mandate x3-lang/examples/near_mandate_agent.x3 --guard-contract <x3-guard account> \
  --out approved-program.x3 --quote
python3 x3-lang/near_agent.py pay "..." --mandate ... --guard-contract ... \
  --out approved-program.x3 --approved-program approved-program.x3 \
  --live --approve <sha256 from the dry run>
python3 x3-lang/near_agent.py redteam --mandate ... --out-dir evidence/   # adversarial requests, real model
python3 x3-lang/near_agent.py register-args <program.x3> --mandate <mandate.x3> \
  --contract <x3-guard account> --signer <agent account> --key-file <agent key>
x3-lang/near/agent-demo.sh setup && x3-lang/near/agent-demo.sh show
```

The request and any `--context` document go to the model as untrusted data. The pipeline issues
the nonce and refuses a program that does not use it, and refuses one that is not
`under mandate <owner>`. Then the mandate decides who may be paid, how much, in what, and where
refunds go. A live quote needs the owner to approve this exact program by its SHA-256 (`--approve`,
or typed at a prompt). Nothing here signs or sends the deposit.

The canonical Rust tooling preserves `under mandate <owner>` in the AST and
`x3c intent` output, and passes it to the host intent resolver. It is adapter
metadata, not a VM guard: the generic VM has no mandate store and does not
claim local enforcement. The NEAR adapter rechecks the owner's mandate in
Python before quoting and x3-guard enforces it again on chain.

`redteam` sends eight attacks through the configured model: recipient substitution, overspend,
splitting a payment to get around the cap, refund redirection, prompt injection in the request and
in an attached invoice, a homoglyph recipient, and another asset. For each, it records the model's
actual output and where the attack stopped. Exit 0 means every unsafe attack was held: either refused or neutralized into a compliant program.

Keep demo payments at about 25 USDC or more under a 100 bps fee cap. 1Click's refund fee is close
to a fixed 0.25 USDC, so on 5 USDC it is about 5%, and the `fees` guard refuses the quote.

| Part | Status |
|---|---|
| Anthropic and OpenAI-compatible clients | implemented; tested against recorded request and response shapes; live-model evidence is pending an API key |
| Pipeline, checks, approval binding, register-args | implemented; regression tests in `tests/test_near_agent.py`, including eight adversarial cases |
| On-chain refusal path | `register-args` produces `agent_register_intent` arguments without a quote; contract tests confirm they are refused with `X3_MANDATE_DENIED`; no production deployment was verified |
| Guided live demo | `near/agent-demo.sh` prepares steps and asks before transactions; it can initiate owner-approved on-chain operations and requires live review |
| Deployed contract and real payment | not done: needs a deployed x3-guard account, owner-authorized wallet action and funds |

`agent-demo.sh` runs the judge-facing sequence in order: mandate, dry model run, owner-approved
live quote, agent register/accept, owner deposit, chain verification, one on-chain refusal, and
owner revocation. Store any committed red-team evidence under `evidence/agent-redteam-<date>/`.

What the agent cannot do: x3-guard holds no funds, so a holder of the origin wallet key can still
deposit directly and bypass Guard. The mandate restricts Guard methods and is meant for an agent
that is given a NEAR account but never the wallet key.

Before quoting, NEAR receivers are checked on chain (`X3_NEAR_ACCOUNT_UNKNOWN`); 1Click itself
answers a missing NEAR account with an unexplained HTTP 400.

Environment: `NEAR_INTENTS_JWT` (API key; without it the unauthenticated fee schedule applies),
`NEAR_INTENTS_BASE_URL`, `NEAR_INTENTS_REFERRAL`, `NEAR_RPC_URL` (default FastNEAR),
`BSC_RPC_URLS` (two public BSC endpoints by default; used for finalized native-BNB evidence),
`X3_NEAR_NONCE_STORE`. The adapter never signs
or sends the deposit; that is made from the sender's wallet. 1Click has no testnet, so live runs
use real funds — start small.

## Transparent optional value fees (preview-only)

The new `value_fee.py` library computes a capped, opt-in **share of positive
net route improvement**, using exact destination-token integer amounts and
fresh comparable quote inputs. It rejects unknown costs, mismatched assets,
stale references and any proposal that would erase the user's minimum gain.

```bash
python3 x3-lang/value_fee.py --preview-stdin < x3-lang/examples/fees/verified_savings_preview.json
```

This is **not** `.x3` syntax today and is not wired to NEAR 1Click `appFees`;
`collectible: false` is mandatory because a signed quote is not proof of
settlement or user authorization. See `../COMMERCIAL_MODEL.md` for the
commercial API/on-chain design and safeguards.

## Quick start

```bash
python3 x3-lang/cli.py x3-lang/examples/arb_solana_eth.x3
cargo test --manifest-path x3-lang/Cargo.toml
```

For the exact source/runtime boundaries, supported guards, proof obligations,
NEAR mandate dialect, troubleshooting and reproducibility commands, read the
[X3Lang programming guide](../docs/x3-lang/PROGRAMMING_GUIDE.md).

## Rust Production Surface

```bash
cargo run -p x3-tools --bin x3c -- parse/check/lower/build/simulate/run/explain <FILE.x3>
```

**x3-lang status: compiles, verifies and simulates trading programs in this workspace; not executable on chain until it targets X3BC.**

### Judge evidence gate limitation (2026-11-03)

`judge_evidence.py --refresh-chain` deliberately reports `unverified_receipt_chain_binding` for otherwise valid live quote receipts. A valid quote signature does **not** authenticate local receipt `leg`, token mappings, or historical settlement baseline. Until those are independently bound to signed requests and chain history, this gate refuses to claim a verified deposit/delivery/refund. Do not present this as a successful chain proof.
