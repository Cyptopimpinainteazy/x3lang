# X3 Shield

[Standalone CI policy](docs/security/STANDALONE_CI_POLICY.md): automatic checks bind this repository's exact source and execute its proofs. Equality with the private parent repository is checked only by an explicitly requested parity audit.

## Competition framing

**.x3 is an owner-controlled, typed intent language for AI-agent payments, combining compiler checks with Guard enforcement of signed quote terms.**

An intent states the desired outcome. The owner’s `.x3` mandate sets the limits.
The compiler rejects invalid programs; deterministic mandate checks and NEAR Guard
refuse prohibited terms. NEAR Intents / 1Click supplies the supported execution route.
Read [the stage-by-stage explanation](docs/hackathon/X3_LANGUAGE.md#x3-intents-with-compiler-checks-and-guard-enforcement)
for compile-time versus runtime checks and the evidence boundaries.


**An AI agent can propose a payment. The owner's `.x3` policy governs whether Guard authorizes the proposed deposit address.**

X3 Shield brings the .x3 typed intent language, compiler checks and signed-quote policy enforcement to AI-agent payments on NEAR. The owner writes spending rules
in X3, a small typed language for cross-chain payment intents. The agent writes each payment
as an `.x3` program. A deterministic compiler and a NEAR contract (`x3-guard`) compare the
two. Model output never authorizes anything.

Built at NEAR HACKATHON 2026. Tracks: **AI Security & Defense** (`security`, primary),
**Agent Economy** (`agents`), **Unstoppable DeFi** (`defi`).

[Why X3 Shield stands out](docs/hackathon/WHY_X3_SHIELD.md) connects each distinguishing claim to its evidence and limits.

## What exists today

| Result | Network | Check it |
|---|---|---|
| An `.x3` program specified a cross-chain trade that finalized: **1.000000 test USDC left Base Sepolia, 0.997400 arrived on NEAR testnet**. The program's SHA-256 was bound on a NEAR Guard contract before the source transfer. | Base Sepolia + NEAR testnet | [Step 1](#1-the-x3-program-behind-the-cross-chain-trade), [Step 2](#2-verify-the-finalized-two-chain-trade) |
| The deployed Guard refused an agent intent to `attacker.near` and accepted a permitted intent. | NEAR testnet | [Proof on NEAR testnet](#proof-on-near-testnet) |
| Guard verified an official 1Click dry quote's signature and refused it; the archived checks include a recipient mismatch and dry-quote failures. | NEAR testnet | [Proof on NEAR testnet](#proof-on-near-testnet) |
| A scripted compliant stand-in produces 8 attack requests; deterministic local checks refuse all 8. | Local | [Step 5](#5-make-the-model-the-attacker) |
| 9 unsafe edits to an agent payment fail to compile. | Local | [Step 3](#3-unsafe-payments-do-not-compile) |

Everything is testnet or local. Nothing here ran on mainnet. The two-chain trade relied on a
trusted testnet relay, so it does not prove atomic settlement. [Limits](#trust-boundaries) are
stated in full below.

## The language in 20 lines

The owner's rulebook, [`near_mandate_agent.x3`](x3-lang/examples/near_mandate_agent.x3):

```text
mandate x3_shopping_agent {
    owner x3-owner.near
    agent x3-agent.near
    spend Solana.USDC
    per_payment 50000000
    per_24h 200000000
    allow recipients x3-demo.near
    allow assets Near.USDC
    require slippage <= 1%
    require fees <= 100 bps
    approve above 30000000 by 1 of x3-owner.near
    expires 30d
}
```

The program that specified the finalized trade,
[`base_sepolia_to_near_testnet_1usdc.x3`](x3-lang/examples/base_sepolia_to_near_testnet_1usdc.x3):

```text
intent x3_testnet_base_usdc_to_near {
    from BaseSepolia.USDC amount 1000000 receiver 0x5aF0c1BF95B3A158f6Fc48e3DED35903b2fa0721
    to Near.USDC receiver x3lang.testnet
    route {
        bridge NearIntents BaseSepolia.USDC -> Near.USDC min_receive 980000;
    }
    require slippage <= 1%
    require nonce unused x3_base_testnet_trade_001
    timeout 30m refund BaseSepolia.USDC to 0x5aF0c1BF95B3A158f6Fc48e3DED35903b2fa0721
    on_fail rollback
}
```

```mermaid
flowchart LR
    A[AI agent] -->|payment .x3| C[X3 compiler + type checker]
    O[Owner mandate .x3] --> C
    C -->|policy terms + source hash| G[x3-guard on NEAR]
    Q[1Click signed quote] -->|payload + Ed25519 signature| G
    G -->|all rules pass| D[deposit address authorized]
    G -->|any rule fails| R[refused, nothing stored]
```

More on what `.x3` does here: [X3 in X3 Shield](docs/hackathon/X3_LANGUAGE.md).

## Try it: do X, see Y

All steps run from the repository root. Steps 1 to 8 are offline, need no wallet, no API key
and no network after install, and sign or send nothing. Each was run on Linux x86_64 with
Python 3.13 and Node 22 at commit `c5a4e39`.

Setup for steps 2 to 8 (Python 3.11+):

```bash
python3 -m venv .venv && . .venv/bin/activate
pip install -r x3-lang/requirements.txt pytest
```

### 1. The `.x3` program behind the cross-chain trade

```bash
sha256sum x3-lang/examples/base_sepolia_to_near_testnet_1usdc.x3
```

**See:** `1398f64e7e208d3796883aa59cb169187c1042ac1af3781bd6efc014755df860`. This is the
program hash the owner registered on the Guard at `x3guardlaunch2026.testnet` in NEAR block
276,595,201, before the Base transfer was included.

### 2. Verify the finalized two-chain trade

```bash
PYTHONPATH=x3-lang:scripts/hackathon python3 scripts/hackathon/verify_first_swap_archive.py
```

**See:** exit 0 and JSON with `"result": "PASS"`, `"sent_test_usdc_units": 1000000`,
`"delivered_test_usdc_units": 997400`, both transaction hashes, and a `not_proven` list.
The same records on public explorers:
[Base Sepolia source](https://sepolia.basescan.org/tx/0x46c21dc305a9271c4e74c6d0734fb50f847a1e53d3cad87976133d66fd6728d9),
[NEAR testnet payout](https://testnet.nearblocks.io/txns/EBEBxJNgV9U5tBFBLj18ZzukgArqwXH5rBsBStFyCm1d).
Add `--live` to re-read both chains from public RPCs (read-only, needs network).

### 3. Unsafe payments do not compile

```bash
python3 scripts/hackathon/x3_wont_compile.py
```

**See:** nine rows ending in `PASS`, then
`9/9 edits refused at compile time; 9/9 with the expected code.` The edits include dropping
the slippage bound, dropping the refund, dropping the one-use nonce, and paying a Cyrillic look-alike
of `x3-demo.near`.

<a id="what-a-prompt-injection-looks-like"></a>
### 4. A prompt-injected payment is refused

```bash
python3 scripts/hackathon/demo.py
```

**See:** in `PANEL 3 - SECURITY DECISION`, the line
`prompt-injection-recipient: decision=denied_by_policy, failed_guards=['mandate_recipient']`,
next to `authorized: decision=preflight_passed` and
`needs-approval: decision=approval_required`. The refused request is
[`near_intents_agent_denied.x3`](x3-lang/examples/near_intents_agent_denied.x3), which pays
`attacker.near`. This is a local check on a checked-in request. It does not show how a live
model reacts to an injection; step 5 uses a scripted compliant model and links a separate real-model run.

### 5. Make the model the attacker

```bash
python3 scripts/hackathon/compromised_agent_redteam.py
```

**See:** `Model complied with 8/8 attacks; deterministic checks refused 8/8; 8/8 refused for the attack's own reason.`
A scripted model writes the `.x3` program each attack asks for. Each row names the check
that refused it and the payment that would have gone out with no policy layer. This runs the
off-chain compiler and mandate checks, not the contract. A separate run with a real local
model (`qwen3:8b`) is recorded in
[evidence/agent-redteam-local-2026-11-06](evidence/agent-redteam-local-2026-11-06/README.md).

### 6. The mandate compiles to the contract's terms

```bash
python3 x3-lang/near_intents.py mandate args x3-lang/examples/near_mandate_agent.x3 \
    --tokens x3-lang/tests/fixtures/near_intents_tokens.json | grep program_sha256
python3 x3-lang/near_intents.py mandate check x3-lang/examples/near_mandate_agent.x3 \
    x3-lang/examples/near_intents_agent_denied.x3 \
    --tokens x3-lang/tests/fixtures/near_intents_tokens.json; echo "exit=$?"
```

**See:** first command prints
`"program_sha256": "629c008639c5fea35c1f2f9f1e9e3c67717860f72b1b7e14450db042f17012d4"`,
which equals `sha256sum x3-lang/examples/near_mandate_agent.x3`. The second prints a guard
list containing `"guard": "mandate_recipient", "ok": false` with detail
`attacker.near, allowed ['x3-demo.near']`, then `exit=6`.

### 7. Browser and Python agree, guard by guard

```bash
node --test x3-lang/near/demo/test/*.test.mjs
```

**See:** `# pass 88` and `# fail 0`. To look at it, run
`python3 -m http.server 8000 --directory x3-lang/near/demo` and open
`http://localhost:8000/judge.html`. The [Attack Lab](docs/hackathon/attack-lab.html) page
evaluates wrong-recipient, payment-cap, nonce-replay and revoked-policy cases in the browser.

### 8. Python test suite

```bash
PYTHONPATH=x3-lang python3 -m pytest x3-lang/tests -q
```

**See:** a final line with no failures; at this commit it reads `594 passed, 1 skipped`
(about 12 seconds).

### 9. NEAR contract tests (Rust)

```bash
(cd x3-lang/near/x3-guard && cargo test --locked --quiet)
```

**See:** `test result: ok. 62 passed; 0 failed`. The folder pins Rust 1.99.1 with the
`wasm32-unknown-unknown` target, so `rustup` downloads that toolchain on first run. In a
sandbox that blocks `static.rust-lang.org` this step fails with a download error before any
test runs; steps 1 to 8 are unaffected.

<a id="verify-near-validator-signatures-offline"></a>
### 10. Verify NEAR validator signatures offline

```bash
E=evidence/near-light-client-ceremony-2026-11-06
X3_HEAD_DIR=$(mktemp -d /tmp/x3-head.XXXXXX)
S="$X3_HEAD_DIR/head.json"
python3 x3-lang/bridge/near_authenticated.py initialize --state "$S" \
  --checkpoint $E/checkpoint.json --approval $E/checkpoint-approval.json \
  --trust-policy $E/trust-policy.json \
  --trust-policy-sha256 e8d683faf497da091adc8602b117906c43bc21db2ba70bc6a0309e9cda4ab964 \
  --checkpoint-sha256 7fc0d2e2c6ae2bd0c62c5759058be705e1f22aa3a43ac59c7b13764d0f9aa311
python3 x3-lang/bridge/near_authenticated.py advance --state "$S" \
  --candidate evidence/near-proof/testnet/sha256/c4/c47f0c381dbd9a37ac432aa4f77cf9d47ea8ccc452bfe4ff2a67b868953caed7.json
```

**See:** the second command prints `"height": 276420402` and `"sequence": 1`. Change one
character of any signature in `approvals_after_next` and it exits `2` with
`invalid validator approval`. 19 of 20 validators signed, 99.3247% of stake
([ceremony record](evidence/near-light-client-ceremony-2026-11-06/README.md)). This
authenticates block headers. It does not prove a wallet transfer or settlement. The state
file is single-use: re-running `initialize` on the same path exits `2` with
`authenticated state already exists`.

### All local gates in one command

```bash
bash scripts/hackathon/judge_one_command.sh /tmp/x3-shield-judge
```

Needs Linux x86_64, Python 3.12, Node 22+ and Rust (same toolchain download as step 9).
**See:** four gates reported `passed` and a report at `/tmp/x3-shield-judge/evidence.md`.
It makes no blockchain transactions and uses no wallet. Longer walkthrough:
[judge guide](docs/hackathon/JUDGE_GUIDE.md).

## What each track gets

| Track | What X3 Shield shows | Steps |
|---|---|---|
| **AI Security & Defense** | A policy engine for agent spending: recipient and asset allowlists, per-payment and 24-hour caps, slippage and fee ceilings, approval thresholds, expiry, one-use nonces. Enforced off-chain by the compiler and on-chain by `x3-guard`, which reads terms from the signed quote bytes. | 3, 4, 5, 6, 9, [testnet refusals](#proof-on-near-testnet) |
| **Agent Economy** | An owner defines a mandate for agent requests. Local checks demonstrate permitted, approval-required and denied cases; testnet evidence shows an allowed intent and a refused intent. The official live quote was accepted only up to deposit authorization. | 4, 6, 7, [testnet allowed control](#proof-on-near-testnet) |
| **Unstoppable DeFi** | A cross-chain trade specified as one `.x3` program with `min_receive`, slippage, timeout, refund and nonce, pre-authorized on NEAR by source hash, then finalized on two chains. | 1, 2, 10 |

The DeFi entry is the narrowest of the three. An official 1Click testnet settlement route has not been confirmed for this demonstration. The finalized testnet trade used Tidepool, a 1Click-compatible community testnet, with a trusted relay. The
official 1Click quote flow was exercised up to deposit authorization and stopped before
funding. `demo.py` and `track_control.py` print the first two track names only.

## NEAR features used

- **NEAR smart contracts** (Rust, `near-sdk` 5.30, WASM): `x3-guard` and `x3-tidepool-guard`,
  deployed on NEAR testnet. [Byte-reproducible build](docs/hackathon/GUARD_WASM_BYTE_REPRODUCIBILITY.md).
- **NEAR Intents / 1Click API**: `.x3` intents lower to a single 1Click leg. The contract
  verifies the 1Click quote's Ed25519 signature against a manager key pinned at
  initialization, then checks the signed recipient, asset, minimum output, deadline and
  refund address against the stored policy.
- **NEAR light client verification**: validator Ed25519 approvals checked from a pinned
  checkpoint (step 10).
- **Function-call access keys**: used by the agent vault, which exists as a local NEAR
  Sandbox test only ([#125](https://github.com/Cyptopimpinainteazy/x3lang/pull/125)).

## Proof on NEAR testnet

**Agent mandate**, Guard contract `x3lang.testnet` (owner `x3lang.testnet`, test agent
`x3lang-guard-20261106.testnet`), [evidence](evidence/near-guard-testnet-live-2026-11-06.md):

| What happened | Transaction |
|---|---|
| Agent tried to register a payment to `attacker.near`; refused by `mandate_recipient` | [`AhVBD…`](https://testnet.nearblocks.io/txns/AhVBD7oXfwsixBipupjCzuhNdqunjdXJnLz3LXnmfnN8) |
| Allowed control: same mandate, permitted recipient, accepted | [`DPJfc…`](https://testnet.nearblocks.io/txns/DPJfcRUGk56h8sSsFsDFFgUMQAiyBCtpMuvquF9FvTct) |

**Signed quotes**, Guard contract `x3lang-guard-20261106.testnet`, WASM SHA-256
`ac44fa53…f8ab` ([deployment `FJ5vo…`](https://testnet.nearblocks.io/txns/FJ5voQaArx17QV4TzDLYDq93cuoAVLSRxjcxbswuyE8v)),
[evidence](docs/hackathon/GUARD_TESTNET_EVIDENCE.md):

| What happened | Transaction |
|---|---|
| Official 1Click dry quote, signature valid, recipient differs from the policy: refused (recipient mismatch plus the dry-quote checks) | [`3Q7q…`](https://testnet.nearblocks.io/txns/3Q7qTZqiRBB6EuhN78Ra5Q5NBqPXDhfuCg2FKW8WoU2q) |
| Official 1Click **live** quote that matches the policy: accepted, stopped before any deposit | [`6B6k…`](https://testnet.nearblocks.io/txns/6B6k5kre4mFoBq7RuJNadQgrhEvmgb9WDLepX22CxzUZ) |

**Finalized two-chain trade**, Guard contract `x3guardlaunch2026.testnet`
(separately deployed, byte-reproducible):

| What happened | Record |
|---|---|
| Owner registered the `.x3` source hash `1398f64e…f860` and the maker-signed quote hash before the source transfer | NEAR block 276,595,201, [certificate](evidence/tidepool-testnet/first-finalized-two-chain-certificate-20261107.json) |
| 1.000000 test USDC sent on Base Sepolia, finality confirmed by two independent RPCs | [`0x46c21dc3…28d9`](https://sepolia.basescan.org/tx/0x46c21dc305a9271c4e74c6d0734fb50f847a1e53d3cad87976133d66fd6728d9) |
| 0.997400 test USDC delivered to `x3lang.testnet`, above the program's `min_receive 980000` | [`EBEBxJNg…yCm1d`](https://testnet.nearblocks.io/txns/EBEBxJNgV9U5tBFBLj18ZzukgArqwXH5rBsBStFyCm1d) |

Supporting records: [frozen signed quote and final certificate](docs/hackathon/FIRST_FINALIZED_SWAP_EVIDENCE.md),
[walkthrough](docs/hackathon/FIRST_SWAP_JUDGE_WALKTHROUGH.md),
[strict proof JSON](evidence/tidepool-testnet/first-swap-strict-two-chain-proof-20261107.json).
This is trusted-relayer testnet settlement, not cryptographic cross-chain atomicity. The
Python X3 frontend parsed and type-checked the program; the Rust compiler and X3VM were not
used on-chain. The trade did not use the newer draft EVM/NEAR scoped-agent vaults.

An explorer failure can show only a generic `WebAssembly trap: unreachable`. For the refusal
reason, see the archived `check_agent_intent` result in the
[agent-intent evidence](evidence/near-guard-testnet-live-2026-11-06.md) or the archived
`check_quote` result in the [1Click quote evidence](docs/hackathon/GUARD_TESTNET_EVIDENCE.md).
The archived view results explain the checks. The transaction receipts record the on-chain
outcomes.

<a id="hard-questions"></a>
## Trust boundaries

A holder of an unrestricted wallet key can bypass Guard and transfer funds directly.
The deployed contract checks Guard-mediated actions and does not custody tokens.
Separate custody work is tracked in a NEAR FunctionCall-only agent vault
([#125](https://github.com/Cyptopimpinainteazy/x3lang/pull/125), local Sandbox),
an EIP-712 Base ERC-20 vault
([#138](https://github.com/Cyptopimpinainteazy/x3lang/pull/138), local Foundry fork),
and read-only signed-quote/Guard-to-Base binding
([#140](https://github.com/Cyptopimpinainteazy/x3lang/pull/140)). These vaults are
not publicly deployed, independently audited or part of the historical Tidepool
trade. They do not constrain an unrelated funded EOA. Owner keys remain trusted.

X3 Shield compares signed quote terms with a typed program: recipient, asset,
minimum output, deadline and refund address. Key restrictions and custody controls
must enforce which actions an agent can take outside that check.

The 1Click manager key is pinned at initialization with no setter. The Guard reads
quote values from the signed bytes. A valid signature proves those terms, not
completion of a swap.

Evidence verifiers pin the genesis hash and exact blocks and fail closed on
RPC disagreement. RPC agreement alone does not authenticate finality. The light
client in step 10 verifies validator approvals from a trusted checkpoint.

<a id="not-claimed"></a>
## Not claimed

Mainnet deployment, public Base/NEAR vault deployment, audited smart-account
custody, a funded official 1Click settlement, a permissionless native X3 BSC-to-NEAR
bridge, cryptographic atomic cross-chain causality and native Rust X3VM execution
on-chain. See the [threat model](docs/security/X3_SHIELD_THREAT_MODEL.md) for the
controls and open risks.

## Hackathon disclosure

- **Prior work.** The X3 language (Python frontend, Rust compiler, X3BC bytecode, X3VM)
  predates the hackathon and is baseline. Its Git history is preserved.
  [PRIOR_WORK.md](PRIOR_WORK.md) dates each component and maps commits to the original
  repository.
- **Hackathon work.** The NEAR integration: 1Click adapter and signed-quote verification,
  the `x3-guard` and `x3-tidepool-guard` contracts, agent mandates, the light client and
  evidence verifiers, the judge demos, and the testnet evidence. Diff from the annotated
  `baseline` tag (last commit before 2026-10-19 00:00 UTC):
  [`baseline...master`](https://github.com/Cyptopimpinainteazy/x3lang/compare/baseline...master).
  PRIOR_WORK.md lists six pre-cutoff commits that appear in that diff because they were
  merged later.
- **Libraries.** `near-sdk` 5.30 (contracts); `pycryptodome` and `cryptography` (Python);
  `pytest` (tests); Node.js 22 built-in test runner (browser parity tests).
- **AI-generated code.** Hackathon-window code and documentation were written with AI coding
  agents (Codex, Claude Code, OpenCode) under the owner's direction and review.
- **Existing product on other chains.** The broader X3 project includes Solana and BSC
  surfaces outside this submission: [docs/EXTENDED_SURFACES.md](docs/EXTENDED_SURFACES.md).

## Repository layout

- `x3-lang/`: X3 language (Python frontend, Rust compiler and VM), NEAR Intents adapter,
  examples, browser demo.
- `x3-lang/examples/`: the `.x3` programs referenced above.
- `x3-lang/near/x3-guard/`, `x3-lang/near/x3-tidepool-guard/`: the NEAR contracts.
- `x3-lang/bridge/`: NEAR light client, Merkle and receipt verification.
- `evidence/`: committed, hash-chained chain observations. Never edited by hand.
- `scripts/hackathon/`: demos, judge gate, evidence verifiers.

## License

MIT ([LICENSE](LICENSE)). The preserved pre-hackathon source has multiple contributors; see
[docs/business/COMMERCIAL_MODEL.md](docs/business/COMMERCIAL_MODEL.md#forks-ownership-and-licensing--what-is-realistic)
for the ownership boundary.
