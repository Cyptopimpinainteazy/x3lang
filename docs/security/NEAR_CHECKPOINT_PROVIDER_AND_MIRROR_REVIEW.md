# X3Lang checkpoint RPC cross-check and exact-head mirror gate

This documentation is a **read-only audit record**, not authorization to approve a checkpoint, sign settlement certificates, deploy, pay, or merge. Both release gates remain explicitly blocked pending separate approval.

## Historical NEAR Testnet cross-check, 2026-11-06 UTC

`evidence/near-checkpoint-review/p27/two-provider-crosscheck.json` is a **real live read-only observation** from separate `https://rpc.testnet.near.org` and `https://test.rpc.fastnear.com` RPC endpoints. The capture uses JSON-RPC `status`, `EXPERIMENTAL_genesis_config`, `EXPERIMENTAL_protocol_config` at historical block 276392452, and `block` for heights 276349252 and 276392452. For each call the report records endpoint, RPC method and parameters, SHA256 of serialized JSON-RPC request, SHA256 of the raw response body, and the **selected** returned consensus metadata. The report itself has a SHA256 integrity seal. The original complete HTTP response bodies are **not persisted in this report**, so their hashes alone are not independently verifiable by a future reader. This report does NOT substitute for the original archived RPC fixtures nor for full authenticated chain data.

Both providers returned the same values:

| Field | Both providers returned |
| --- | --- |
| Chain ID | `testnet` |
| Genesis hash (`status`) | `FWJ9kR6KFWoyMoNjpLXXGHeuiy7tEY6GmoFeCA5yuc6b` |
| Genesis metadata | 2020-07-31 03:39:42.911378Z, height 42376888, protocol 29, epoch length 43200 |
| Historical protocol at candidate height | 88 |
| Predecessor block | height 276349252, hash `FXA5BRLmqPP1uPfFx6Ndd5cyLZnHTvjpZ2oBEgoxxYpP` |
| Candidate block | height 276392452, hash `GCa27vYKmwgQLvRTbLYvWitZe3L54J48LbR2Zx62XvWv` |
| Epoch transition | `HbM6N16QDpBgam1mxFQNjDX7HdWhcTz5HxewWgYgjcHk` → `p27PbpijmKi38kLzQdnbc6b3koijbkGsbCQGWzjen46` |

**Trust limitation:** Two servers agreeing does not prove they are honest, independent, or using a checkpoint verified from trusted nearcore history. A malicious shared upstream, testnet reset, or competing chain can report internally coherent data. Do not conclude `authenticated=true` or give the reviewer signatures on this basis. The `EXPERIMENTAL_genesis_config` response does not itself include a signed genesis hash; the `status.genesis_hash` is also RPC-reported. The selected version 88 still requires an independently reviewed protocol-release policy. No external checkpoint-review ceremony or HSM reviewer keys were available to perform that final step.

### Offline reproduction

```bash
python3 x3-lang/bridge/checkpoint_provider_crosscheck.py verify \
  --root evidence/near-checkpoint-review/p27 \
  --report evidence/near-checkpoint-review/p27/two-provider-crosscheck.json
python3 -m unittest discover -s x3-lang/bridge -p 'test_checkpoint_provider_crosscheck.py' -v
```

Run the commands in an isolated Python 3.11+ environment with the repo's existing requirements. To make a **fresh** capture into a new file and compare results (never rewrite the committed archive), use `collect --root ... --report /tmp/x3-near-new-crosscheck.json`. Verification makes no network calls. Tests verify the genuine archived positives and reject mutations of claimed genesis, protocol, checkpoint hashes, request method/parameters, digest and authorization flags.

## Exact-head peer mirror checkout: deterministic cause and owner decision

The `X3Lang Exact-Head Local Execution Proof` workflow in `.github/workflows/x3lang-execution-proof.yml` attempts `actions/checkout@v4` of `Cyptopimpinainteazy/xxxstar` using `ref: ${{ github.head_ref }}`. PR #61's branch is `sec/near-unsigned-checkpoint-review-20261105`, which does **not** exist in the peer repository. The failing job says `A branch or tag with the name 'sec/near-unsigned-checkpoint-review-20261105' could not be found` at its `Checkout matching peer feature branch` step. Thus `scripts/proof/check_x3lang_mirror.py` never ran in that job. Its intended check compares seven always-shared files, plus every changed path within `x3-lang/` or `scripts/proof/` against the corresponding peer feature branch (using `origin/main` and peer `origin/master` divergence). Passing the separate X3Lang execution job is **not** the same as satisfying the mirror check.

No GitHub Actions code has been modified here, because replacing the absent peer ref with `xxxstar/main`, changing `if` conditions, excluding files from the comparison, or treating checkout failure as success would weaken or invalidate this hard gate.

Only two acceptable owner-controlled strategies, both requiring explicit approval and audited policy:

1. **Keep the strict same-branch mirror invariant.** Authorized repo owners create/maintain corresponding peer branches in `xxxstar`, mirroring the *exact shared and changed* source files. Run the existing two-repo checker, review both commits/diffs, and require exact-head CI success before any merge. Current `AGENTS.md` makes `xxxstar` **read-only** during the hackathon, so an agent must NOT create or modify those peer branches without a separate explicit owner decision.
2. **Formally separate X3Lang-specific work from cross-repository mirror obligations.** Define in writing which paths must be mirrored, update security requirements and the comparison test under separate reviewed PR, and prove that all formerly shared proof-code paths are still protected by exact SHA256 checks. This is a genuine policy change, NOT a shortcut to make the current run green.

The branch `feat/near-authenticated-light-client-final-20261106` does exist in both repos, but substituting it for PR #61's exact branch would compare different feature content and does **not** demonstrate that PR #61's files are mirrored. This is not a valid fix by itself.

## Final disposition

Checkpoint cross-provider consistency: **verified against two public RPC responses**, not authenticated.

Checkpoint trust ceremony: **not performed** (requires independent chain/validator provenance, approved threshold signer policy, signatures, and independent review).

Exact-head peer mirror: **still failed** until the owner approves and carries out one of the secure paths above.

Any financial settlement authorization, signed certificate, deployed X3 code identity or mainnet release: **BLOCKED**.
