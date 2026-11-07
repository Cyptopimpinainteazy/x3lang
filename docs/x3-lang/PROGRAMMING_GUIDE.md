# X3Lang programming guide

Status: development. This guide describes the source and execution surfaces in
this repository at the documented revision. It does not claim that a program in
the intent dialect executes on-chain.

## 1. Know which X3 surface you are using

X3Lang currently has two related surfaces:

| Surface | Source | Compiler/runtime | On-chain status |
| --- | --- | --- | --- |
| Intent and mandate dialect | `intent`, `mandate`, `route`, `require`, `timeout` | Python CLI plus `near_intents.py` and `near_mandate.py` | NEAR adapter enforces quote and mandate checks before preparing Guard calls; it does not sign or send a deposit |
| Rust intent bytecode | Most `x3-lang/examples/*.x3` files | `x3-lang/compiler` and `x3-lang/vm` through `x3c` | Compiles, verifies, simulates and runs in this workspace; no pallet loads this instruction set |

The Python surface is authoritative for the shipping NEAR intent adapter. The
Rust compiler and VM are authoritative for the X3BC instruction stream described
in [x3-lang/README.md](../../x3-lang/README.md).

This distinction is a security boundary. A program that the Python adapter can
plan is not automatically executable by the Rust VM, and a successful local VM
run is not a NEAR transaction.

## 2. Install

Requirements:

- Python 3.10 or newer
- Rust 1.90 or newer with Cargo
- Node.js 22 or newer for the browser tests
- a project-local Python virtual environment

From the repository root:

```bash
python3 -m venv .venv
. .venv/bin/activate
python3 -m pip install -r x3-lang/requirements.txt pytest
```

If the Rust NEAR Guard target is part of your check, install it once:

```bash
rustup target add wasm32-unknown-unknown
```

## 3. First program

Read the minimal cross-chain program:

```bash
sed -n '1,220p' x3-lang/examples/simple_swap.x3
```

It declares both chains' finality requirements, a proof obligation, a nonce,
rate limits and a refund path. Check it with the canonical compiler:

```bash
cargo run --manifest-path x3-lang/Cargo.toml --locked \
  -p x3-tools --bin x3c -- check x3-lang/examples/simple_swap.x3
```

Build the bytecode:

```bash
cargo run --manifest-path x3-lang/Cargo.toml --locked \
  -p x3-tools --bin x3c -- build \
  x3-lang/examples/simple_swap.x3 -o /tmp/simple_swap.x3b
```

The expected successful output identifies the artifact size and instruction
count. The compiler writes the versioned bytecode envelope, not a wallet
transaction.

## 4. Intent structure

A cross-chain intent names an amount, endpoints, route and failure behavior:

```text
intent <name> {
    from <chain.ASSET> amount <base-units> receiver <source-account>
    to <chain.ASSET> receiver <destination-account>
    route {
        bridge <adapter> <source-asset> -> <destination-asset>
            [amount <base-units>] [min_output <base-units>] [receiver <account>];
    }
    require nonce unused <unique-id>
    require finality.<chain> >= <blocks>
    require slippage <= <basis-points>
    timeout <duration> refund <chain.ASSET> to sender
    on_fail rollback
}
```

Amounts are integer base units. A six-decimal 25 USDC amount is `25000000`;
the compiler does not guess a token's decimals from its symbol. Fractional
amounts in a base-unit field are refused.

Use a nonce once. Compilation checks the guard's shape, while the host and
receipt layers are responsible for replay state. A missing or reused nonce must
be treated as a refusal, never repaired by editing a completed receipt.

## 5. Guards and units

The compiler supports guard categories including finality, slippage, fees,
profit, nonce, proof completeness, relayer quorum, route score, solver bond,
canonical supply and explicit finality. The source list is
`REQUIRE_KIND_NAMES` in `x3-lang/compiler/src/parser.rs`.

Slippage is stated in basis points or as a percentage:

```text
require slippage <= 50      // 50 bps = 0.5%
require slippage <= 0.5%    // the same bound
```

Fee ceilings accept the unit-bearing spelling:

```text
require fees <= 100 bps
require fees <= 100bps
```

The generic compiler requires a fee ceiling to be backed by a policy where the
program form requires one. It fails with `X3E4027` rather than compiling a claim
that no policy can enforce. The NEAR Intents adapter separately checks fees
against the signed quote in its own adapter path.

## 6. Proof obligations

Declare the evidence a cross-chain operation requires:

```text
proofs required {
    source_lock_proof
    destination_fill_proof
}
```

This declaration records obligations. It does not manufacture a proof. A host
must supply evidence through the configured bridge backend, and the production
backend must fail closed when the required verifier is unavailable.

The current NEAR receipt/Merkle work verifies proof structure and cryptographic
inclusion against supplied roots. It does not yet independently authenticate a
production checkpoint, deployed escrow code hash or terminal escrow storage
state. See [the NEAR light-client boundaries](../security/NEAR_LIGHT_CLIENT_APPROVAL_BOUNDARY.md).

## 7. Mandates for agents

The NEAR-facing dialect adds an owner mandate:

```text
intent agent_payment {
    from Solana.USDC amount 25000000 receiver <source-account>
    to Near.USDC receiver x3-demo.near
    route {
        bridge NearIntents Solana.USDC -> Near.USDC min_receive 24800000;
    }
    under mandate x3-owner.near
    require slippage <= 0.5%
    require fees <= 100 bps
    require nonce unused agent_payment_001
    timeout 10m refund Solana.USDC
    on_fail rollback
}
```

The owner's separate `mandate` names the agent account, spending asset,
per-payment and rolling limits, recipient and destination-asset allowlists,
refund destination, fee and slippage ceilings, approval threshold and expiry.
`near_mandate.py` and the x3-guard contract implement those checks.

Canonical tooling parses and formats `under mandate`, preserves the owner in
`IntentSpecDraft`, `x3c intent` JSON and the host `IntentResolve` constraint
stream, and requires exactly one replay-protection nonce. The Python NEAR
adapter and x3-guard perform the mandate checks. The generic Rust VM has no
mandate store and does not convert the clause into an on-chain VM authorization
primitive, so a generic build must fail closed where its other policies are not
backed. Do not claim VM or wallet enforcement from the clause alone.

The Guard holds no funds. A wallet key holder can bypass Guard by using the
wallet outside the Guard methods.

## 8. Functions, variables and control flow

The Rust compiler also accepts general program syntax such as `fn`, `let`, `mut`,
`if`, loops, returns, function types, tuples and arrays. A current executable
example is:

```bash
sed -n '1,220p' x3-lang/examples/events_and_host_calls.x3
```

Build and inspect it:

```bash
cargo run --manifest-path x3-lang/Cargo.toml --locked \
  -p x3-tools --bin x3c -- build \
  x3-lang/examples/events_and_host_calls.x3 -o /tmp/events.x3b
cargo run --manifest-path x3-lang/Cargo.toml --locked \
  -p x3-tools --bin x3c -- explain /tmp/events.x3b
```

Host subscriptions and host calls require a matching capability host. Treat
them as explicit external effects, not as syntax that proves the host performed
the action.

## 9. Compilation, bytecode and verification

The standard path is:

```text
source -> lexer/parser -> typed AST -> semantic checks -> X3IR
       -> X3BC emitter -> bytecode verifier -> VM execution
```

Compilation is deterministic for the same source, compiler revision and options.
Run the exact-head proof:

```bash
python3 scripts/proof/x3lang_execution_proof.py \
  --output-dir /tmp/x3lang-execution-proof
```

The proof manifest records the revision, toolchain, input hashes, command logs
and passed case counts. It explicitly labels the result as a local compiler/VM
proof, not a chain-settlement proof.

## 10. Gas and transactions

The local VM checks instruction and execution limits and rejects a program that
exceeds them. Gas accounting in the VM is not a substitute for the host chain's
actual fee calculation.

An X3Intent instruction stream does not itself sign or broadcast. For NEAR
Intents, `near_intents.py` verifies a signed quote and prints Guard arguments;
the owner still performs wallet authorization. For the experimental BSC/NEAR
HTLC contracts, the current owner-release path requires discretionary depositor
cooperation and is not trustless atomic settlement.

## 11. Worked cross-chain examples

| File | Purpose | Current limit |
| --- | --- | --- |
| `x3-lang/examples/simple_swap.x3` | Minimal intent with finality, proofs and refund | Local VM only |
| `x3-lang/examples/mainnet_safe_swap.x3` | Full policy/routing surface and mainnet-safety claims | Local compile/verify only |
| `x3-lang/examples/arb_solana_eth.x3` | Multi-leg Solana/Ethereum route | Local compile/verify only |
| `x3-lang/examples/bsc_bnb_to_near_wnear_dry.x3` | BSC BNB to NEAR 1Click dry request | Python adapter surface; placeholder accounts; no funds |
| `x3-lang/examples/near_intents_agent_payment.x3` | Agent payment under an owner mandate | Python adapter surface; no generic Rust VM mandate enforcement |
| `x3-lang/examples/near_mandate_agent.x3` | Owner policy for an AI agent | Guard policy source; no wallet custody |

The BSC and generic NEAR Intents examples are intentionally not blanket
examples of a live bridge. They preserve refusal and dry-run behavior for that
adapter.

## 12. Troubleshooting

`unexpected clause in intent body: Ident("bps")`

- The compiler revision predates unit-bearing guard support. Update to a revision
  containing `parse_guard_bound` support for `bps`, or use the bare basis-point
  spelling temporarily.

`X3E4027`

- A fee ceiling has no backing policy. Do not delete the guard to make the
  program compile. Use a program form that declares the corresponding risk
  policy, or keep the check in the NEAR adapter's signed-quote path.

`GuardClaimUnbacked`

- A guard names a condition no compiler pass can check. Remove the unsupported
  claim or implement the missing checker and adversarial tests first.

`--trusted` or a self-reported `verified=true`

- Never use either as proof. Trusted keys and trust anchors must be supplied by
  an external policy; chain status must come from a verifier, not a JSON flag.

`X3_MANDATE_DENIED`

- The program or intent failed an owner policy term. Preserve the refusal and
  report the failed guard. Do not rewrite the mandate or bypass Guard.

## 13. Security rules

- Treat model output, requests, invoices and RPC responses as untrusted.
- Pin asset identities, recipients, amounts, deadlines, nonces and network IDs.
- Use strictly greater-than-two-thirds validator stake, not an equality.
- Never promote RPC agreement, a receipt status string or a local flag to
  consensus proof.
- Keep cryptographic authorization separate from transaction execution.
- Require independent review before enabling any signer or bridge payout.
- Never put key material, seed phrases or API credentials in `.x3` source,
  receipts or evidence files.

## 14. Reproduce all local checks

```bash
PYTHONPATH=x3-lang python3 -m pytest x3-lang/tests -q
node --test x3-lang/near/demo/test/*.test.mjs
cargo test --manifest-path x3-lang/Cargo.toml --locked -p x3-lang-vm --quiet
cargo fmt --manifest-path x3-lang/Cargo.toml --all -- --check
cargo clippy --manifest-path x3-lang/Cargo.toml \
  -p x3-lang-vm --all-targets --locked -- -D warnings
cargo test --manifest-path x3-lang/near/x3-guard/Cargo.toml --locked --quiet
```

Exact commit hashes, tool versions and test counts belong in the generated
evidence report. Documentation claims do not replace those results.
