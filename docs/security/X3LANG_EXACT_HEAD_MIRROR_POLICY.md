# Exact-head mirror policy proposal

Status: **DRAFT / UNAPPROVED**

This document proposes a narrow policy for stacked X3Lang feature branches that
cannot be checked out in `Cyptopimpinainteazy/xxxstar`. It does not approve the
policy, a NEAR checkpoint, settlement authority, deployment, or release.

## Observed blocker

The existing workflow checks out `xxxstar` at `${{ github.head_ref }}` before it
runs `scripts/proof/check_x3lang_mirror.py`. The current PR-head branch status
on 2026-11-05 is:

| x3lang PR | Expected peer branch | Peer state |
|---|---|---|
| #58 | `sec/near-receipt-outcome-inclusion-20261105` | missing |
| #59 | `feat/near-authenticated-light-client-final-20261106` | branch exists, but its changed-path content does not match the x3lang PR content |
| #60 | `sec/near-trusted-epoch-verification-20261105` | missing |
| #61 | `sec/near-unsigned-checkpoint-review-20261105` | missing |
| #62 | `sec/near-checkpoint-crossprovider-review-20261105` | missing |

All five x3lang PR heads currently contain the same seven required shared proof
files as peer content anchor
`444762e02211bb02096a94402496615e3dda2902`. That agreement is useful evidence,
but it does not replace matching peer branches for changed proof paths.

## Proposed admission rule

The strict same-branch gate remains the default. When the matching peer branch
exists, the existing `check_x3lang_mirror.py` comparison remains authoritative.

### Policy-review bootstrap

A PR that changes only the policy manifest, policy runner, its tests, the CI
workflow, CODEOWNERS, or this document runs in `POLICY_REVIEW_ONLY` mode when
the peer branch is absent. Every such diff must include the policy manifest.
That mode verifies the complete shared-file manifest against both the exact
x3lang head and the pinned peer commit, but it cannot admit production changes.
Any `x3-lang/**`, `scripts/proof/**`, or other path in the same diff keeps the
strict gate closed.

This lets the initial policy proposal receive exact-head CI before the policy
exists on `main`. It does not approve the policy, bypass the peer comparison,
or authorize a feature PR. Feature fallback remains unavailable until the
policy itself is merged to `main`.

The proposed fallback is unavailable until `.github/mirror-policy.json` changes
to `OWNER_APPROVED` with a matching owner-controlled repository variable,
`X3LANG_MIRROR_POLICY_APPROVAL_REFERENCE`. An approval PR can change only the
policy manifest's status and reference; every other policy field must remain
byte-equivalent. Once approved, a missing peer branch may pass only when all of
these hold:

1. Every changed path is under `x3-lang/bridge/`.
2. No changed path is one of the seven always-shared proof files.
3. No changed path is anywhere under `scripts/proof/`.
4. The PR does not modify the workflow, policy runner, or policy manifest.
5. The policy at the PR head is byte-identical to `origin/main`; approval is
   read only from the base branch and cannot approve itself inside the PR being
   checked.
6. Every shared file in the x3lang head matches its reviewed SHA-256.
7. Every shared file at the pinned peer commit matches the same SHA-256.
8. The policy names the exact 40-character peer commit and the repository.
9. The peer commit's recorded first parent is reachable from the peer
   repository's `master` branch.
10. The external approval reference exactly matches the value controlled
    outside the PR by the repository owner.

Any failure stops the job. There is no skip, warning-only mode,
`continue-on-error`, mutable peer-main fallback, or inferred approval.
`main` must also have a branch protection rule requiring the security workflow
and code-owner approval before a policy change can land. `CODEOWNERS` by itself
is not enforcement on an unprotected branch.

## Reviewed shared-file manifest

| Path | SHA-256 |
|---|---|
| `scripts/proof/check_x3lang_mirror.py` | `3509cffdc14af9571ea520d5e2ce6d0abdcf391c2233426f44b09d912efe71ef` |
| `scripts/proof/x3lang_execution_proof.py` | `3265c4d0b06eda8f3eaa43732bea7d3235ad84ecc738beaf00741ad2745d4af5` |
| `scripts/proof/x3lang_runtime_equivalence.py` | `398b5832d2d9f733be94b07b26b9c0af0914d7e0f12ee0ea1118a8c9de5ec58f` |
| `x3-lang/compiler/tests/test_bytecode_version_gate.rs` | `67cf288cc4865f0de051bfe9533547236bbdb3662e521463df2c31bf9b33e680` |
| `x3-lang/compiler/tests/test_control_flow_e2e.rs` | `fdfd791d14de047522c5252c62b24d12e3b1fa262bbbb57f99ee5bbb3bedcb94` |
| `x3-lang/vm/tests/test_nonce_replay.rs` | `6aa9d2ef46bcc5a75a3aa1b430731b58d251813603c67b4134bb5199bc5c2ecc` |
| `x3-lang/vm/tests/trading_receipts.rs` | `b68b2b58226e39cb8f22fca02793f951ea8c4fd999841a2f369e0dd9b0a31be6` |

## Owner action required

Choose one:

1. Keep the existing same-branch invariant and authorize repository owners to
   create exact matching branches in `xxxstar`. This does not weaken the gate,
   but `xxxstar` is read-only for agents.
2. Review this policy, complete `owner_approval_reference`, switch its status to
   `OWNER_APPROVED`, and merge it through a protected-branch PR. Also require
   code-owner review for `.github/workflows/x3lang-execution-proof.yml`,
   `.github/scripts/exact_head_mirror_policy.py`, and
   `.github/mirror-policy.json`.

Technical enforcement requires branch protection and the owner-controlled
approval variable; this repository alone cannot prevent an administrator from
merging policy changes. Until an owner completes one option, feature PRs with
missing peer branches and no approved fallback remain red. A policy-only PR may
be green in `POLICY_REVIEW_ONLY` mode, but that result explicitly admits no
feature changes.

## Local verification

```bash
PYTHONPATH=.github/scripts python3 .github/scripts/test_exact_head_mirror_policy.py -v
python3 -m py_compile .github/scripts/exact_head_mirror_policy.py
```

The tests cover bridge-only admission, external approval matching, VM and
shared-file refusal, arbitrary new proof-script refusal, self-approval in a
feature PR, unapproved policy refusal, policy-only bootstrap review,
manifest-required bootstrap, mixed policy/feature refusal, peer-anchor
provenance and drift, empty diffs, and manifest completeness.
