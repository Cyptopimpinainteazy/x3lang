# Source provenance and historical-work disclosure

The initial standalone import originates from the public repository
[`Cyptopimpinainteazy/xxxstar`](https://github.com/Cyptopimpinainteazy/xxxstar),
exact **source commit** [`4425b009615e61c7bac1f92b492413a9f666cd81`](https://github.com/Cyptopimpinainteazy/xxxstar/commit/4425b009615e61c7bac1f92b492413a9f666cd81)
(`feat/x3-liquidity-shield-20261103`, November 3, 2026).

Tracked source paths extracted **without copying build artifacts or home-directory files**:

- `x3-lang/**`: existing Python and Rust language tooling, tests, NEAR proof/demo components, examples, and WIP security research.
- `crates/x3bc-core/**`: canonical X3 bytecode core, used by `x3-lang/compiler` through a path dependency.
- `scripts/proof/generate_eth_bridge_fixture.py` and
  `docs/x3-lang/fixtures/**`: minimal archived proof fixtures needed for the
  X3Lang VM's existing Ethereum/Solana bridge verification tests.

The initial export also normalizes trailing whitespace in a handful of
historical examples/specifications and adds a standalone-disclosure banner
to `x3-lang/README.md`; these editorial changes are not new language features.

The new repository's **root README**, `SOURCE_PROVENANCE.md`, `.gitignore`,
and CI configuration describe the import, not newly authored compiler features.
The original repository is authoritative for commit-by-commit historical
attribution. A clean import commit here must **not** be presented as evidence
that every imported line was written during a particular hackathon window.

Related work in the monorepo: draft PR #727 (X3 Shield), draft PR #725 (NEAR
proof checks). Neither is a public mainnet security approval.

No GitHub workflows or CI results in this new repository establish that token
custody, flash-lending, solver attribution, or on-chain policy execution is
production-ready. Evidence must be independently reproduced on this repo's
**exact commit**, with explicit deployment and finality proofs where applicable.
