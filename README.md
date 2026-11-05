# X3Lang — programmable financial policies

X3Lang is an existing experimental compiler, transaction-policy VM and
cross-chain verification toolkit developed as part of the X3 blockchain.

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

### Public browser demonstration

[**X3 Guard — signed dry-quote validation**](https://x3-guard.pages.dev/)
verifies a real 1Click quote and checks risk rules. This is **not** proof of
an authorized wallet transaction or a completed cross-chain settlement.
The local `judge.html` console is the deterministic, offline companion: it
does not contact 1Click or NEAR and can be run entirely from the repository.

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
- Licensing, independent security review and exact-head GitHub Actions
  verification are required before a production release.
