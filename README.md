# X3Lang — verifiable policies for cross-chain execution

**Experimental financial-intent language, compiler, transaction-policy VM, and NEAR Intents integration.** X3Lang is being developed as part of the X3 blockchain ecosystem. The objective is to turn explicitly constrained trading and agent instructions into verifiable execution policies—not to let AI output or a quoted price authorize a transfer on its own.

**Current status:** research and development; **not a production solver, audited custody system, or demonstrated atomic cross-chain trading engine**. The public demo checks signed *dry* quotes; it does not execute a funded swap.

[Try the X3 Guard demo](https://x3-guard.pages.dev/) · [NEAR adapter and capabilities](x3-lang/README.md#near-intents-adapter-near_intentspy) · [NEAR Guard contract](x3-lang/near/x3-guard/README.md) · [Prior work & provenance](PRIOR_WORK.md)

## Why this exists

An ordinary swap quote says what a route *might* deliver. X3Lang aims to specify **what a trading agent is allowed to do** before funds move: assets, addresses, minimum received, maximum slippage and fees, expiry, refunds, and delegated-agent limits.

The NEAR-specific prototype compiles a policy to a NEAR Intents **1Click** request, validates the returned signed quote, and can generate arguments for an on-chain policy contract. Users retain control of the origin-chain wallet and send deposits separately.

### Demonstrable today vs. still to prove

| Area | In this repository | Boundary |
| --- | --- | --- |
| X3Lang source and compiler/VM | Python intent DSL, Rust compiler and VM, tests and examples; transaction and receipt logic against fixture hosts | The trading-intent VM does **not** execute on-chain |
| NEAR Intents / 1Click adapter | Quote planning, dry/live quote handling, Ed25519 quote-signature checks, amount/slippage/fee/expiry rules, nonce tracking, status/receipt tooling | A signed quote is **not** proof of settlement; the adapter never signs or sends a deposit |
| X3 Guard (NEAR contract source) | Policy registration, signed-quote checks, nonce enforcement, agent mandates and revocation; contract tests | Contract source and tests do **not** establish an audited production deployment; outcome recording is not independent cross-chain proof |
| Browser demonstration | Real 1Click **dry** quote validation with line-by-line policy verdicts | No authorized wallet payment, solver fill, or completed cross-chain settlement is demonstrated |
| Chain evidence tooling | Guided first-swap flow and deposit/delivery/refund verification commands | End-to-end solver causality and refund proofs remain release gates |
| Liquidity-provider / solver integration | Opportunity for integration and pilot collaboration | **No active production solver service, guaranteed capital facility, or verified market-making volume** |

## Live demo and code entry points

**[Open the X3 Guard browser demo](https://x3-guard.pages.dev/)** — inspect the intent-policy checks against a signed dry quote. The page is a demonstration of validation, not a transaction terminal.

- [Canonical Solana USDC → NEAR USDC intent](x3-lang/examples/near_intents_sol_to_near.x3): route, minimum output, fee and slippage limits, timeout and refund policy. The example contains **placeholder receivers**; do not send funds to them.
- [NEAR Intents adapter](x3-lang/near_intents.py): quote and policy validation, guarded live-quote workflow, receipt/status commands.
- [NEAR Guard contract](x3-lang/near/x3-guard/README.md): source, trust model, tests, agent-mandate permissions, and deployment instructions.
- [Guided first swap](x3-lang/near/first-swap.sh): opt-in small-trade workflow; **spending requires user authorization**.
- [Trading Core v1 details](x3-lang/README.md#trading-core-v1): fixture-host execution, debt closure and deterministic receipts, with explicit limits.
- [Historical provenance](PRIOR_WORK.md): what predates the hackathon and how that code reached this repository.

## NEAR Intents collaboration / pilot proposal

We are interested in working with the **NEAR Intents** team and ecosystem liquidity providers on a small, risk-limited integration pilot.

**What X3Lang could contribute:** a programmable policy layer for 1Click orders and delegated trading agents, signed-quote screening, explicit risk bounds, reproducible verifier tests, and a path toward more robust settlement evidence.

**What we'd like to discuss:**

1. A technical review of the adapter and Guard's security assumptions.
2. Developer or solver-pilot access, and whether **reduced minimum capital requirements** are available for controlled testing.
3. Introductions to liquidity providers, market makers, or solver operators willing to explore a narrowly scoped trial.
4. Whether ecosystem grants, technical mentorship, or **sponsored/delegated liquidity** programs are available—subject to custody, risk and compliance review.

**Suggested trial milestones:** (1) reproducible local validation and adversarial tests; (2) independently reviewed signing, nonce, wallet, and refund boundaries; (3) a separately authorized small live trade with independently verifiable deposit and delivery/refund evidence; (4) measured quote quality, failures and costs; (5) only then discuss capital scale-up or solver admission.

This is an **invitation to evaluate a prototype**, not a claim of endorsement by NEAR, a funding commitment, or an assertion that minimums can be waived. Any test using real assets requires explicit approval and operational safeguards.

**Technical contact:** open a [GitHub issue](https://github.com/Cyptopimpinainteazy/x3lang/issues) referencing “NEAR Intents pilot” or contact the repository maintainer through GitHub.

## Reproduce local checks

From the repository root, use an isolated Python environment and install dependencies:

```bash
python3 -m venv .venv
source .venv/bin/activate
python -m pip install -r x3-lang/requirements.txt pytest

PYTHONPATH=x3-lang python -m pytest x3-lang/tests -q
node --test x3-lang/near/demo/test/*.test.mjs
(cd x3-lang && cargo test -p x3-lang-vm --locked --quiet)
(cd x3-lang && cargo fmt --all -- --check)
(cd x3-lang && cargo clippy -p x3-lang-vm --all-targets --locked -- -D warnings)
(cd x3-lang/near/x3-guard && cargo test --locked --quiet)
(cd x3-lang/near/x3-guard && cargo check --target wasm32-unknown-unknown --locked)
bash scripts/check-near-allocator.sh
```

These are **commands to run**, not a claim that the latest commit has passed every check. The Guard build requires the WebAssembly target. Consult [AGENTS.md](AGENTS.md) for safety and exact-head verification rules.

### Read-only quote exploration

```bash
python3 x3-lang/near_intents.py plan x3-lang/examples/near_intents_sol_to_near.x3
python3 x3-lang/near_intents.py quote x3-lang/examples/near_intents_sol_to_near.x3
```

Replace the placeholder wallet receivers with addresses you control before requesting actionable quotes. **Do not use `--live`, deploy contracts, or send deposits merely to try the README.** NEAR Intents 1Click has no testnet in this adapter's documented flow; live execution can involve real funds.

## Hackathon history and release boundaries

The original X3Lang language and trading VM **predate the current NEAR hackathon**. Read [PRIOR_WORK.md](PRIOR_WORK.md), the [`baseline...main` history comparison](https://github.com/Cyptopimpinainteazy/x3lang/compare/baseline...main), and the [pinned original source commit](https://github.com/Cyptopimpinainteazy/xxxstar/commit/95587205d5febc40c1047c845f379946c023228d). The repository preserves historical authorship and chronology; new NEAR-specific work is identified separately.

The experimental LP-pool and agent-firewall modules in `xxxstar` draft PR #727 are **not** included here and have unresolved issues. The Guard does **not custody origin-chain funds** or prevent a wallet keyholder from transferring independently. A signed quote is not a wallet authorization; a recorded outcome is not independently proven settlement. Production use needs independent security review, exact-head CI, permission and key-boundary review, reliable cross-chain causality/refund evidence, licensing review, and explicit release approval.

**Project:** [Cyptopimpinainteazy/x3lang](https://github.com/Cyptopimpinainteazy/x3lang) · **Parent ecosystem:** [X3 / xxxstar](https://github.com/Cyptopimpinainteazy/xxxstar)
