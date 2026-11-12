# What the X3 language does in X3 Shield

## .x3: intents with compiler checks and Guard enforcement

**.x3 is an owner-controlled, typed intent language for AI-agent payments.**
An intent describes the desired outcome; an owner mandate defines the permitted
recipients, assets, amounts and execution bounds. X3 Shield combines compiler
checks with deterministic authorization of signed quote terms.

| Stage | What it does | Evidence boundary |
|---|---|---|
| Intent and mandate source | Express the payment outcome and the owner's constraints in `.x3` | Inspect the checked-in programs; an English task is not authorization |
| Python compiler frontend and type checker | Reject invalid programs and required-safety-field violations | Run `python3 scripts/hackathon/x3_wont_compile.py`; its nine mutations exercise compile-time checks |
| Mandate evaluation | Compare a proposed intent with owner policy | Run `python3 scripts/hackathon/demo.py`; policy refusal is separate from a compiler error |
| NEAR Guard | Verify signed quote bytes and enforce stored intent/policy terms on Guard-mediated calls | Inspect the separately labeled testnet refusal and allowed-control evidence |
| NEAR Intents / 1Click | Supply the supported quote and execution route | Quote authorization is not deposit or settlement proof |

The integration uses NEAR Intents as its execution infrastructure. .x3 adds a
typed source language and an inspectable compiler-to-policy workflow around
the owner's intent. This is complementary to NEAR Intents; it does not claim
NEAR lacks constraints or that X3 invented outcome-based execution.

**Three-track story:** Agent Economy supplies the agent's payment job;
AI Security & Defense shows compiler rejection and policy refusal;
Unstoppable DeFi shows the separately evidenced cross-chain route.
The historical Base Sepolia → NEAR testnet transfer used a trusted Tidepool
relay. It is separate from the official 1Click quote demonstration, which
stopped before funding.

The compiler cannot enforce external chain behavior by itself. Guard does not
custody funds, and an unrestricted wallet key can bypass it. The historical
trade used the Python frontend, not Rust X3VM bytecode execution on-chain.
Timeout, refund and rollback syntax alone does not prove cross-chain rollback.

**Judge takeaway:** “The agent proposes an intent. .x3 compiles the rules.
Guard checks the signed terms. The execution route carries out the payment.”


X3 is a typed language for cross-chain payment intents. A `.x3` program says what moves,
from which chain to which recipient, within which bounds (slippage, fees, timeout, refund,
one-use nonce). In this submission `.x3` does three jobs. Each has a check you can run.

| Job | What you can check | Command or record |
|---|---|---|
| [Specified a real cross-chain transfer](#1-an-x3-program-moved-test-usdc-from-base-sepolia-to-near-testnet) | The program's source hash is the one bound on-chain before the transfer | `sha256sum` + the offline archive verifier |
| [Is the owner's rulebook](#2-the-owners-rules-are-an-x3-program-that-compiles-to-guards-on-chain-terms) | The mandate compiles to Guard's `create_mandate` terms, carrying its own source hash | `near_intents.py mandate args` / `mandate check` |
| [Refuses unsafe payments at compile time](#3-unsafe-payments-do-not-compile) | Nine unsafe edits to the agent's payment are refused before any check or chain call | `scripts/hackathon/x3_wont_compile.py` |

All commands run from the repository root, offline, with Python 3 and
[`x3-lang/requirements.txt`](../../x3-lang/requirements.txt). None signs, deploys or sends anything.

## 1. An X3 program moved test USDC from Base Sepolia to NEAR testnet

[`x3-lang/examples/base_sepolia_to_near_testnet_1usdc.x3`](../../x3-lang/examples/base_sepolia_to_near_testnet_1usdc.x3)
is the whole specification of the transfer:

```x3
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

What happened, in order (from the checksummed
[two-chain certificate](../../evidence/tidepool-testnet/first-finalized-two-chain-certificate-20261107.json)):

1. **The source was hashed and bound on NEAR.** The program's SHA-256 is
   `1398f64e7e208d3796883aa59cb169187c1042ac1af3781bd6efc014755df860`. The owner `x3lang.testnet`
   registered it under nonce `x3_base_testnet_trade_001` on the Guard at `x3guardlaunch2026.testnet`
   (WASM SHA-256 `1d7a8b1e…a145`), together with the maker-signed quote hash `131f314c…034c`.
   That record is in finalized NEAR block **276,595,201**, before the Base transfer was included.
   In the [Tidepool Guard source](../../x3-lang/near/x3-tidepool-guard/src/lib.rs), `authorize`
   refuses with `signed owner program hash mismatch` unless the program hash presented equals the
   one the owner registered.
2. **Base Sepolia sent 1.000000 test USDC.** Transaction
   [`0x46c21dc3…28d9`](https://sepolia.basescan.org/tx/0x46c21dc305a9271c4e74c6d0734fb50f847a1e53d3cad87976133d66fd6728d9),
   block **55,557,170**, 1,000,000 base units, confirmed finalized by two independent RPCs.
3. **NEAR testnet delivered 0.997400 test USDC to `x3lang.testnet`.** Transaction
   [`EBEBxJNg…yCm1d`](https://testnet.nearblocks.io/txns/EBEBxJNgV9U5tBFBLj18ZzukgArqwXH5rBsBStFyCm1d),
   `ft_transfer` of 997,400 base units in block **276,595,461**, above the program's
   `min_receive 980000`. The recipient's finalized balance went from 100.000000 to 100.997400.

**Check it yourself**

```bash
sha256sum x3-lang/examples/base_sepolia_to_near_testnet_1usdc.x3
# 1398f64e7e208d3796883aa59cb169187c1042ac1af3781bd6efc014755df860

PYTHONPATH=x3-lang:scripts/hackathon python3 scripts/hackathon/verify_first_swap_archive.py
# exit 0; prints the bound hashes, both transaction hashes, 1000000 sent, 997400 delivered,
# and a "not_proven" list
```

`verify_first_swap_archive.py --live` re-reads the historical records from public RPCs
(read-only). Full timeline and evidence:
[judge walkthrough](FIRST_SWAP_JUDGE_WALKTHROUGH.md) and
[finalized-swap evidence](FIRST_FINALIZED_SWAP_EVIDENCE.md).

**What this is not**

- **Not trustless.** The link between the Base transfer and the NEAR delivery is a trusted
  Tidepool community testnet relay, not an atomic or cryptographic bridge.
- **Not X3VM execution.** The Python X3 frontend parsed and type-checked the program; the Rust
  compiler and X3VM bytecode were not used on-chain.
- **Not a check that the program produced the quote.** Guard binds the exact source hash to the
  authorization; the contract's own comment says the maker signature authenticates the quote,
  not the program.
- **One leg, testnet only.** No mainnet funds or authorization.

## 2. The owner's rules are an X3 program that compiles to Guard's on-chain terms

The mandate in [`near_mandate_agent.x3`](../../x3-lang/examples/near_mandate_agent.x3) lets the
agent `x3-agent.near` pay only `x3-demo.near`, in Solana USDC → NEAR USDC, up to 50 USDC per
payment and 200 per day, with owner approval above 30 USDC. It compiles to the exact
`create_mandate` call Guard takes:

```bash
python3 x3-lang/near_intents.py mandate args x3-lang/examples/near_mandate_agent.x3 \
    --tokens x3-lang/tests/fixtures/near_intents_tokens.json
```

The output's `terms.program_sha256` is the mandate file's own SHA-256 (`629c0086…12d4`), so the
on-chain rules name the source they came from. Checking the agent's attack intent against it:

```bash
python3 x3-lang/near_intents.py mandate check x3-lang/examples/near_mandate_agent.x3 \
    x3-lang/examples/near_intents_agent_denied.x3 \
    --tokens x3-lang/tests/fixtures/near_intents_tokens.json
# mandate_recipient: ok false ("attacker.near, allowed ['x3-demo.near']"); exit 6
```

The same mandate checks ran on NEAR testnet:
[refused payment to `attacker.near`](https://testnet.nearblocks.io/txns/AhVBD7oXfwsixBipupjCzuhNdqunjdXJnLz3LXnmfnN8)
and [allowed control](https://testnet.nearblocks.io/txns/DPJfcRUGk56h8sSsFsDFFgUMQAiyBCtpMuvquF9FvTct)
([evidence](../../evidence/near-guard-testnet-live-2026-11-06.md)).

## 3. Unsafe payments do not compile

An agent writes its payment as X3. The compiler refuses programs that are missing a bound,
are shaped in a way 1Click cannot settle, or ask for a guard it cannot enforce, before the
mandate check, a quote request or Guard ever sees them:

```bash
python3 scripts/hackathon/x3_wont_compile.py
```

It starts from the agent's real payment program
([`near_intents_agent_payment.x3`](../../x3-lang/examples/near_intents_agent_payment.x3)),
confirms it compiles, then applies one edit at a time:

| Agent tried to | Refused with |
|---|---|
| drop `require slippage` | `X3_NEAR_SLIPPAGE_REQUIRED` |
| drop the timeout and refund | `X3_NEAR_TIMEOUT_REQUIRED` |
| drop the one-use nonce | `X3_NEAR_NONCE` |
| add a second swap leg | `X3_NEAR_ROUTE_SHAPE` |
| add a finality guard 1Click can't enforce | `X3_NEAR_GUARD_UNENFORCEABLE` |
| refund in a different asset | `X3_NEAR_REFUND_ASSET` |
| pay a Cyrillic look-alike of `x3-demo.near` | `X3_NEAR_TYPECHECK` (`X3_INVALID_RECEIVER`) |
| send from Ethereum with a Solana address | `X3_NEAR_TYPECHECK` (`X3_INVALID_RECEIVER`) |
| pay an amount of 0 | `X3_NEAR_TYPECHECK` (`X3_INVALID_AMOUNT`) |

The command exits non-zero if any edit compiles or is refused with a different code.

## What the hackathon path deliberately leaves out

The NEAR path accepts one program shape: a single `bridge NearIntents` leg. The language also
has `atomic_choice`, bounded `route_fallback`, `parallel` multi-VM plans, `finality_policy` and
proof obligations, and a Rust compiler, X3BC bytecode and X3VM
(see [`x3-lang/examples`](../../x3-lang/examples)). None of those is part of this submission's
evidence, and none ran on-chain here.
