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
