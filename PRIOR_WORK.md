# Prior work and history disclosure for the NEAR hackathon

**This X3Lang repository is a history-preserving extraction of an existing project.**
The compiler, Rust trading VM, canonical X3BC core, and many examples/tests
predate the NEAR hackathon. This repository's import or merge date does **not**
represent the date the underlying code was written.

## Exact source and migration procedure

- **Original:** [`Cyptopimpinainteazy/xxxstar`](https://github.com/Cyptopimpinainteazy/xxxstar), branch `feat/near-intents-adapter`, **pinned source HEAD** [`95587205d5febc40c1047c845f379946c023228d`](https://github.com/Cyptopimpinainteazy/xxxstar/commit/95587205d5febc40c1047c845f379946c023228d).
- **Method:** full, non-shallow, non-partial clone from GitHub; `git-filter-repo` v2.47.0. The resulting 389 commits preserve the authored/committed historical dates, authors and order, with new hashes because unrelated monorepo paths were removed.
- **Filter invocation** (from a fresh full source clone, not the old `x3lang` snapshot branch):

  ```bash
  git filter-repo --force \
    --path x3-lang/ \
    --path crates/x3bc-core/ \
    --path docs/x3-lang/fixtures/ \
    --path scripts/proof/generate_eth_bridge_fixture.py
  ```

  The three additional paths provide the canonical bytecode crate and archived
  Ethereum/Solana verification fixtures required for reproducible VM tests;
  they are not unrelated product features. See
  [`docs/provenance/xxxstar-to-x3lang-commit-map.tsv`](docs/provenance/xxxstar-to-x3lang-commit-map.tsv)
  for the old-to-filtered SHA mapping.
- **Filtered source tip:** [`cb3e48a78bcdd5101048d5e57bcb7010196169ed`](https://github.com/Cyptopimpinainteazy/x3lang/commit/cb3e48a78bcdd5101048d5e57bcb7010196169ed). The existing `x3lang/master` initialization commit `7feb7eca5b1d65ff34ccb06efec2b90afcba0647` is retained by a merge commit with **two parents**, rather than flattened into a new root snapshot.

## Auditable historical cutoff

**Cutoff: October 19, 2026, 00:00 UTC.** The annotated tag
[`baseline`](https://github.com/Cyptopimpinainteazy/x3lang/tree/baseline)
points to **filtered commit** `a0a711e7381943fbe5544f6c0da3842e3a3bc89a`,
committed **October 18, 2026, 09:49:23 MDT**, the latest commit by committer
timestamp before the cutoff in the filtered ancestry. It maps to **original
xxxstar** commit [`d3783944c52883b16e86760f1fe410ad5e834a2c`](https://github.com/Cyptopimpinainteazy/xxxstar/commit/d3783944c52883b16e86760f1fe410ad5e834a2c).

**Canonical comparison:** [**`baseline...master`**](https://github.com/Cyptopimpinainteazy/x3lang/compare/baseline...master).
Until this draft import is merged, use the [**review-branch comparison**](https://github.com/Cyptopimpinainteazy/x3lang/compare/baseline...feat/x3lang-preserved-history-20261103).
The canonical `baseline...master` comparison becomes meaningful once `master`
contains the filtered ancestry; it does not before then.

**Crucial ancestry caveat:** six other commits with committer timestamps
*before* the cutoff were authored on different branches and merged into the
history **after** the chronological baseline. They appear in a `baseline...HEAD`
comparison, but are **also prior work**, not hackathon-created features.
They remain in their original positions in the filtered graph; we have not
rewritten author dates or faked a linear cutoff. Their filtered SHAs are:

- `dfd74188a13e3c424d251f8ffc7b08c39e7a2574`
- `2c2820dfa7640d5c9738b71baf3afc47c285823c`
- `2ce425224bcc56c5cbd82547f58d7be70ab269ce`
- `6284e01b69c834892e87dd5e776612f6cd8fe6fd`
- `8aa4d77abb88b52e6c9832b87c0babb0403dd300`
- `554aee117fd25cd4cfd1eb5b5d042ab125d44780`

## NEAR-specific implementation timeline (not the prior language)

These are original xxxstar commits reachable from the pinned source branch:

| Date | Original source commit | Work |
|---|---|---|
| Nov 2, 2026 | [`dd3a426`](https://github.com/Cyptopimpinainteazy/xxxstar/commit/dd3a426c4d0859b47d4296c482caf411b4ef5a9b) | NEAR Intents / 1Click integration started |
| Nov 2, 2026 | [`1e21473`](https://github.com/Cyptopimpinainteazy/xxxstar/commit/1e21473c1abb0bea1e43be2d6aed3d059645f9b6) | NEAR x3-guard contract |
| Nov 2, 2026 | [`7e73f4a`](https://github.com/Cyptopimpinainteazy/xxxstar/commit/7e73f4a670a84151360268a3f8cdab5f3c3b5613) | 1Click signed quote verification |
| Nov 3, 2026 | [`6008ae7`](https://github.com/Cyptopimpinainteazy/xxxstar/commit/6008ae74af477a5321cd40f304b3cba93c2bcef9) | Browser-signed quote validation demo |
| Nov 3, 2026 | [`525ff6a`](https://github.com/Cyptopimpinainteazy/xxxstar/commit/525ff6a8c4dec65e3ef1c5e660853fdea77c6d4c) | Chain evidence checks |
| Nov 3, 2026 | [`9558720`](https://github.com/Cyptopimpinainteazy/xxxstar/commit/95587205d5febc40c1047c845f379946c023228d) | Guided first-swap script |
| Nov 3, 2026 | [`ffae47f`](https://github.com/Cyptopimpinainteazy/xxxstar/commit/ffae47f5d6f08b0365ff948742ac14062d4f3536) | Agent mandates enforced by x3-guard (applied here as a patch after the import, not through filter-repo) |

This table identifies particular NEAR-window changes. Other X3Lang commits
after October 19 may be independent core development and are **not** claimed
as hackathon-created merely because of their dates.

## Explicitly excluded

Draft PR [`xxxstar #727`](https://github.com/Cyptopimpinainteazy/xxxstar/pull/727)
adds non-NEAR liquidity shadow-pool and agent-firewall experiments. Their
`x3-lang/vm/src/liquidity.rs` and `x3-lang/vm/src/agent_firewall.rs` modules
are **not present** in the pinned source branch or this import. The reported
borrower-selected flash-fee and first-depositor share-inflation issues remain
separate, unresolved review items. Do not present this import as fixing them.

**Security status:** signed dry quotes are not wallet-signed transactions;
independent Solana and NEAR transfer observations are not proof that the
solver causally linked both legs. No mainnet execution, token custody, or
release approval is established by this repository import.
