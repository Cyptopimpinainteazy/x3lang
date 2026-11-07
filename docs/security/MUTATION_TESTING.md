# X3 Shield security mutation testing

This gate changes actual **source code in disposable local copies** and reruns the original mandate tests. It is not a NEAR Sandbox deployment, a blockchain receipt proof, or a security audit.

## Baseline and scope

The current first-stage campaign targets `x3-lang/near_mandate.py` — the Python policy-admission implementation, **not** Rust `x3-guard`, the compiler, VM, light-client verifier, or the owner-controlled wallet. The test corpus includes:

- `x3-lang/tests/test_near_mandate.py` — 45 cases on commit `eff72ac`.
- `x3-lang/tests/test_near_mandate_boundaries.py` — 14 added boundary cases.

The unmutated baseline must pass. Each mutant then runs the same 59 tests using the same Python interpreter in its own temporary source copy. The real repository/worktree is never edited by the runner. Source drift (missing/ambiguous replacement) invalidates the mutant and blocks the gate.

## Curated mutants

| ID | Single altered security behavior | Risk |
| --- | --- | --- |
| `recipient_bypass` | Accept a recipient outside the owner's allowlist | P0 |
| `origin_asset_bypass` | Accept an unauthorized source asset | P0 |
| `destination_asset_bypass` | Accept an unauthorized destination asset | P0 |
| `refund_bypass` | Accept a non-owner-approved refund destination | P0 |
| `payment_cap_bypass` | Ignore the per-payment spending cap | P0 |
| `daily_cap_bypass` | Ignore both the rolling daily cap and payment count | P0 |
| `slippage_cap_bypass` | Ignore the maximum permissible slippage | P0 |
| `fee_cap_bypass` | Ignore the maximum fee | P0 |
| `deadline_bypass` | Accept an intent with a deadline beyond mandate expiry | P0 |
| `mandate_active_bypass` | Accept a revoked or expired mandate | P0 |
| `approval_bypass` | Require no approvals above the owner threshold | P0 |
| `expiry_one_ms_late` | Permit execution at the exact expiry instant | P1 |

Each mutation must be applied to **exactly one matching source expression at its pinned line**. Inverting a comparison or making an already-permitted transaction fail is not treated as evidence of an authorization bypass. The mutations above test permissive defects plus one expiry boundary defect.

## Execution

Use a repository-local or separately provisioned Python virtual environment with pytest and the existing X3Lang dependencies. For the same dependencies as the judge gate on Linux x86_64/Python 3.12:

```bash
python3.12 -m venv .venv
.venv/bin/python -m pip install --require-hashes -r scripts/hackathon/requirements-judge.txt
PYTHONDONTWRITEBYTECODE=1 .venv/bin/python scripts/security/mutation_runner.py \
  --output-dir /tmp/x3-shield-mutation-results
```

The JSON and Markdown reports are written to the specified directory. The script also supports `--mutations ID1,ID2` and `--timeout 60` for a bounded focused run. Report the **exact repository HEAD**; do not assume stored snapshots are current.

## How results are classified

- **KILLED:** A real test assertion fails due to the changed behavior; mere process failure does not qualify.
- **SURVIVED:** The mutant is applied successfully but all selected tests pass, indicating a test gap or an equivalent mutation.
- **INVALID:** Source drift, import/collection error, malformed mutant, test-harness failure, or other non-assertion failure. This cannot count as killed.
- **TIMEOUT:** Bounded test run exceeds the configured duration; not counted as killed.
- **NOT_RUN:** Not executed. No credit.

The mutation score is `killed / (killed + survived)`, with all invalid, timed-out and not-run cases listed separately. **Any surviving mutant, invalid mutation, timeout, red baseline or unknown mutation ID fails the CI gate.** P0 survivors block release independently of the aggregate score.

## Verified local evidence versus release claims

At `eff72ac636283950714eafd06e18701238058a47`, the verified standalone manual permissive-mutation experiment on x3star1 used the 45-test baseline and recorded **11 killed / 0 survived / 0 invalid / 0 timed-out**; supporting details were posted to GitHub issue #88. After the gate was corrected to use actual authorization-bypass mutations, a local 12-mutant rerun using 59 baseline tests recorded **12 killed / 0 survived / 0 invalid / 0 timed-out** with exit 0. This is a mutation score for **only these curated Python defects**, not repo-wide, Rust Guard, wallet, NEAR bridge, authenticated finality or compiler coverage.

The results JSON is regenerated for each run; runner-local reports must not be confused with public testnet evidence. The next gates are native Rust Guard mutations, authenticated proof-verifier and VM/compiler mutations, and independent exact-head CI/review. Do not merge a security-sensitive change solely because this narrow test gate is green.
