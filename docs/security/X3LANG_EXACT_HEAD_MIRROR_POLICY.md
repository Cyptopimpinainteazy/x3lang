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

The proposed fallback is unavailable until `.github/mirror-policy.json` changes
to `OWNER_APPROVED` with a real review reference in a separately reviewed PR.
Once approved, a missing peer branch may pass only when all of these hold:

1. Every changed path is under `x3-lang/bridge/`.
2. No changed path is one of the seven always-shared proof files.
3. No changed path is anywhere under `scripts/proof/`.
4. Every shared file in the x3lang head matches its reviewed SHA-256.
5. Every shared file at the pinned peer commit matches the same SHA-256.
6. The policy names the exact 40-character peer commit and the repository.

Any failure stops the job. There is no skip, warning-only mode,
`continue-on-error`, mutable peer-main fallback, or inferred approval.

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

No agent may self-approve the policy. Until an owner completes one option, every
missing-peer-branch check remains red by design.

## Local verification

```bash
PYTHONPATH=.github/scripts python3 .github/scripts/test_exact_head_mirror_policy.py -v
python3 -m py_compile .github/scripts/exact_head_mirror_policy.py
```

The tests cover bridge-only admission, VM and shared-file refusal, arbitrary new
proof-script refusal, unapproved policy refusal, peer-anchor drift, empty diffs,
and manifest completeness.
