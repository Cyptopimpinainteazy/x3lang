# X3Lang — verifiable financial policies for cross-chain execution

X3Lang is an existing experimental compiler, transaction-policy VM and
cross-chain verification toolkit developed as part of the X3 blockchain. It aims to constrain trading and agent instructions with explicit authorization, recipient, fee, expiry and refund policies before funds move.

Unlike NEAR's policy-bound wallets or function-call keys, X3 mandates are a
typed language that also verifies NEAR Intents' signed quote terms before a
deposit is authorized.

**Works today** — a `.x3` mandate compiles and the same verdict is reproduced
in Python, the browser, and the NEAR guard contract; the guard's on-chain
Ed25519 check verifies a real 1Click quote's signed terms (locally tested);
refusal-first tests cover overspend, wrong recipient, replay, refund
redirection, and revocation.

**Deliberately blocked** — no wallet transfer, live settlement, or on-chain
authorization is claimed here. Cross-chain causality stays "unproven" until an
independently verifiable link exists.

**Hackathon judges:** [Reproduce NEAR tests and inspect real-proof limitations](docs/hackathon/JUDGE_GUIDE.md).
The read-only “do X, see Y” gate is `python3 scripts/judge_evidence.py --mode quick`;
public mainnet transaction links remain pending and are **not** fabricated.

**Hackathon disclosure:** the original language and its trading engine were
created **before** the current NEAR hackathon. This repository preserves their
real Git history. Start with [PRIOR_WORK.md](PRIOR_WORK.md), including its
[`baseline...main` historical comparison](https://github.com/Cyptopimpinainteazy/x3lang/compare/baseline...main).

The exact historical source is
[`xxxstar` / `feat/near-intents-adapter` @ `95587205`](https://github.com/Cyptopimpinainteazy/xxxstar/commit/95587205d5febc40c1047c845f379946c023228d).
The source was extracted with `git-filter-repo` from a **full Git clone**;
commit authorship and chronology were preserved, not condensed into a
single import commit. Review the SHA map in `docs/provenance/`.

## Try the judge console in 60 seconds

```bash
python3 -m http.server 8000 --directory x3-lang/near/demo
# Open http://localhost:8000/judge.html
```

The console shows one owner mandate deciding the allowed, approval-required
and forbidden actions shown by the checked-in X3 programs. The browser runs
the same mandate checks as the production adapter, and the page compares its
verdict with Python guard by guard.

For the terminal version:

```bash
python3 scripts/hackathon/demo.py
bash scripts/hackathon/verify.sh
```

### What a judge can inspect

| Question | Where to look |
|---|---|
| What exactly did the owner authorize? | Panel 1, the `.x3` mandate source and hash |
| What did the agent request? | Panel 2, the exact intent and changed lines |
| Was it allowed, denied, or held for approval? | Panel 3, verdict plus every guard |
| What cross-chain route would execute? | Panel 4, source/destination assets and bounds |
| What is actually proven? | Panel 5, evidence ladder and recorded test runs |
| How do I reproduce it? | `build_scenarios.py`, `collect_evidence.py`, `scripts/hackathon/demo.py` |

The scenario set covers spending caps, the approval threshold, unauthorized
recipients, refund redirection, fee and slippage limits, wrong mandate owners,
rolling spend, revocation and nonce replay. Each scenario names its judge
takeaway and whether its decisive evidence is local or contract-level.

### Evidence levels

| Level | What it means | Current demo |
|---|---|---|
| Local policy | The mandate and intent compile, and Python/browser checks agree | Proven |
| Signed quote | A live 1Click response verifies against the pinned manager key | Not part of the offline console |
| On-chain authorization | A deployed x3-guard registers and accepts the policy | Not claimed |
| Wallet transfer | The owner or authorized signer sends funds | Not claimed |
| Settlement | Source and destination chains are independently observed | Not claimed |
| Causality | One transaction is cryptographically proven to cause the other | Not proven |

The browser console is therefore a **policy-preview and refusal demo**, not a
payment executor. The terminal `verify.sh` emits a machine-readable readiness
report and fails when evidence is stale, dirty or missing.

## Contents

- **`x3-lang/`** — Python financial-intent DSL, Rust compiler, virtual machine,
  tests, examples, NEAR Intents adapter, x3-guard smart contract, and browser demo.
- **`crates/x3bc-core/`** — deterministic X3BC bytecode definitions required
  by the X3Lang compiler.
- **`docs/x3-lang/fixtures/` and `scripts/proof/`** — archived proof fixtures
  and verifier generator used by existing VM integration tests.

### User-first commercial economics (new, opt-in)

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

### Reproducible local checks

```bash
python3 -m pip install -r x3-lang/requirements.txt pytest
PYTHONPATH=x3-lang python3 -m pytest x3-lang/tests -q
(cd x3-lang && cargo test -p x3-lang-vm --locked --quiet)
(cd x3-lang && cargo fmt --all -- --check)
(cd x3-lang && cargo clippy -p x3-lang-vm --all-targets --locked -- -D warnings)
(cd x3-lang/near/x3-guard && cargo test --locked --quiet)
node --test x3-lang/near/demo/test/*.test.mjs
```

### Experimental Solana owner-controlled token vault

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

### Public browser demonstration

[**X3 Guard — signed dry-quote validation**](https://x3-guard.pages.dev/)
verifies a real 1Click quote and checks risk rules. This is **not** proof of
an authorized wallet transaction or a completed cross-chain settlement.
The local `judge.html` console is the deterministic, offline companion: it
does not contact 1Click or NEAR and can be run entirely from the repository.

### NEAR Intents integration: what works and what remains

**[X3 Guard live dry-quote demo](https://x3-guard.pages.dev/)** · [Example .x3 intent](x3-lang/examples/near_intents_sol_to_near.x3) · [1Click adapter](x3-lang/near_intents.py) · [Guard contract and trust model](x3-lang/near/x3-guard/README.md)

| Component | Demonstrable in source | Not yet established |
| --- | --- | --- |
| X3Lang compiler and trading VM | Python intent DSL, Rust compiler/VM, deterministic test receipts and fixture-host trading workflows | Trading-intent VM execution on a live chain |
| NEAR Intents 1Click adapter | Plans requests; checks signed quotes, minimum received, slippage, fees, freshness and nonces | A production solver or a completed cross-chain fill |
| NEAR Guard | On-chain contract source for policy registration, signature checks, mandates and revocation; local tests | Production security audit, independent cross-chain causality proof |
| Browser demo | Real signed *dry* quote with visible policy verdicts | Wallet authorization, funded swap or settlement |
| Evidence tooling | Guided first-swap script and chain-verification commands | Publicly verified authorized swap and refund evidence |

**NEAR Intents partnership / pilot inquiry.** We invite NEAR Intents engineers and liquidity providers to review the adapter, assess a capped pilot, and discuss whether reduced minimum solver capital, technical mentorship, a liquidity-partner introduction, or grants/delegated liquidity could be available. We propose progressing from reproducible adversarial checks and signing-boundary review to a separately authorized small live swap, independently verified settlement or refund, measured execution costs, and *only then* larger capital or solver admission. This is a proposal, **not** an existing sponsorship, solver accreditation, or a claim that minimum requirements can be waived.

Technical discussion: [open an issue titled NEAR Intents pilot](https://github.com/Cyptopimpinainteazy/x3lang/issues/new?title=NEAR%20Intents%20pilot). Review the [full adapter README](x3-lang/README.md#near-intents-adapter-near_intentspy) before executing any live commands; no testnet or wallet-free live operation is implied.

### Hackathon judge package

The offline three-track demonstration needs no wallet, network request or
funds:

```bash
python3 scripts/hackathon/demo.py
bash scripts/hackathon/verify.sh
```

It shows one owner mandate deciding an allowed payment, an approval-required
payment and a refused payment through the same X3Lang policy checker. See
[`docs/hackathon/JUDGE_PACKAGE.md`](docs/hackathon/JUDGE_PACKAGE.md), the
[three-minute walkthrough](docs/hackathon/VIDEO_WALKTHROUGH.md) and the
[submission checklist](docs/hackathon/SUBMISSION_CHECKLIST.md). Local policy
acceptance is not wallet authorization, a signed quote or settlement.

## Release boundaries

- Historical X3Lang source is not newly invented hackathon work.
- The NEAR Guard code is under review; do not authorize a wallet or deposit
  based only on dry-quote validation.
- This source **does not** include the experimental LP-pool or agent-firewall
  modules from `xxxstar` draft PR #727; those contain unresolved review issues.
- Cross-chain causality/settlement attribution and refund proofs remain
  release gates. There is no on-chain X3Lang trading execution claim here.
- The implemented controls and unresolved wallet, policy-binding, migration,
  storage, and settlement risks are tracked in the
  [X3 Shield threat model](docs/security/X3_SHIELD_THREAT_MODEL.md).
- Licensing, independent security review and exact-head GitHub Actions
  verification are required before a production release.

## What happens after the hackathon

- Release gates stay fail-closed: wallet-level enforcement, tuple-bound
  destination assets and recipients, bounded intent storage, versioned
  migration review, and explicit contract upgrade-authority lock-down.
- The next technical gate is an independent security review of the guard
  contract, the model pipeline, and the deployment configuration.
- The light client's offline checkpoint ceremony runs with real signers, then
  a funded testnet refusal and an independently verified receipt close the
  loop the offline demo deliberately leaves open.

## License

Dual-licensed under **MIT OR Apache-2.0**, matching the SPDX expressions in
the crate manifests. The MIT text is in [LICENSE](LICENSE); the Apache-2.0
text is the standard Apache License, Version 2.0.
