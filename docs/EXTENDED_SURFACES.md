# Extended surfaces — Solana vault, value-fee economics, NEAR Intents rollout

These are the hackathon-stage surfaces that sit alongside the core NEAR
mandate story. They are kept here so the README's first screen stays on
what works today versus what is deliberately blocked.

## User-first commercial economics (new, opt-in)

[X3Lang's value-fee arithmetic](x3-lang/value_fee.py) provides an inspectable,
**non-billable** preview: an operator can propose a capped share of measurable
positive net savings, and the user must remain better off after the proposed
fee. See [COMMERCIAL_MODEL.md](COMMERCIAL_MODEL.md) for the transparent
revenue options, IP/licensing limitations, and missing on-chain authority,
and the [NEAR value-first rollout gates](docs/plans/NEAR_X3LANG_VALUE_FIRST_IMPLEMENTATION.md)
for the planned on-chain enforcement and integration work.

```bash
python3 x3-lang/value_fee.py --preview-stdin \
  < x3-lang/examples/fees/verified_savings_preview.json
```

**This is an optional Python policy-economics primitive, not yet new `.x3`
compiler syntax, a paid API, or a wallet fee.** The example uses illustrative
unsigned quotes; it cannot authorize payments. Public source cannot be made
fork-proof by embedding a revenue check. A separate user-approved service
would be needed to collect any future fee.

## Experimental Solana owner-controlled token vault

The [X3 Solana Mandate Vault](x3-lang/solana/x3-mandate-vault/README.md)
is a new hackathon-stage, standalone Solana program for **funds already held
in its PDA-controlled classic SPL Token account**. An owner must approve
the exact destination token account and its owner, amount, quote digest,
nonce and expiry. A designated agent can then execute that single approval
using a signed `TransferChecked` CPI; revocation, recovery and true
rolling 24-hour limits are evaluated on-chain.

The vault's test harness invokes the actual X3 and SPL Token processors
under Solana `program-test`, **not public mainnet**. It has no automatic
NEAR/1Click authority and does not enforce any unrelated Solana wallet.
The source-chain event is a basis for a future evidence verifier, not
proof of destination-chain causality.

```bash
(cd x3-lang/solana/x3-mandate-vault && cargo test --locked)
(cd x3-lang/solana/x3-mandate-vault && cargo fmt --all -- --check)
(cd x3-lang/solana/x3-mandate-vault && cargo clippy --all-targets --locked -- -D warnings)
```

## NEAR Intents integration: what works and what remains

**[X3 Guard live dry-quote demo](https://x3-guard.pages.dev/)** · [Example .x3 intent](x3-lang/examples/near_intents_sol_to_near.x3) · [1Click adapter](x3-lang/near_intents.py) · [Guard contract and trust model](x3-lang/near/x3-guard/README.md)

| Component | Demonstrable in source | Not yet established |
| --- | --- | --- |
| X3Lang compiler and trading VM | Python intent DSL, Rust compiler/VM, deterministic test receipts and fixture-host trading workflows | Trading-intent VM execution on a live chain |
| NEAR Intents 1Click adapter | Plans requests; checks signed quotes, minimum received, slippage, fees, freshness and nonces | A production solver or a completed cross-chain fill |
| NEAR Guard | On-chain contract source for policy registration, signature checks, mandates and revocation; local tests | Production security audit, independent cross-chain causality proof |
| Browser demo | Real signed *dry* quote with visible policy verdicts | Wallet authorization, funded swap or settlement |
| Evidence tooling | Guided first-swap script and chain-verification commands | Publicly verified authorized swap and refund evidence |

### NEAR Intents partnership / pilot inquiry

We invite NEAR Intents engineers and liquidity providers to review the adapter,
assess a capped pilot, and discuss whether reduced minimum solver capital,
technical mentorship, a liquidity-partner introduction, or grants/delegated
liquidity could be available. We propose progressing from reproducible
adversarial checks and signing-boundary review to a separately authorized
small live swap, independently verified settlement or refund, measured
execution costs, and *only then* larger capital or solver admission. This is a
proposal, **not** an existing sponsorship, solver accreditation, or a claim
that minimum requirements can be waived.

Technical discussion: [open an issue titled NEAR Intents pilot](https://github.com/Cyptopimpinainteazy/x3lang/issues/new?title=NEAR%20Intents%20pilot). Review the [full adapter README](x3-lang/README.md#near-intents-adapter-near_intentspy) before executing any live commands; no testnet or wallet-free live operation is implied.
