# Hackathon CI and developer-experience audit

Checked on 2026-11-04 against
`.github/workflows/x3lang-history-ci.yml`.

## Findings

| Area | Finding | Action |
|---|---|---|
| Runner type | All lightweight tests use `ubuntu-latest`; no self-hosted dependency | retained |
| Workspace paths | Python runs from the repository root with `PYTHONPATH=x3-lang`; Rust runs in its nested workspace | retained |
| Duplicate execution | The workflow runs on both `push` to `feat/**` and `pull_request`, so a same-repository PR can show two copies of each job for one SHA | documented; retained because both events are useful and neither weakens a required check |
| Exact-head evidence | Checkout uses the triggering commit and every result is attached to that SHA | retained |
| Python/browser integration | One hosted job runs all Python tests and all browser tests | retained |
| Scenario drift | `build_scenarios.py --check` previously ran only through pytest | pytest subprocess check retained |
| Judge entrypoint | No workflow exercised the new terminal entrypoint | added `python scripts/hackathon/demo.py` |
| Guard workspace formatting | Root VM formatting did not cover the separate `near/x3-guard` workspace | added `cargo fmt -- --check` |
| Guard strict lint | No Guard clippy gate existed | added `cargo clippy --all-targets --locked -- -D warnings` |
| Guard Wasm compatibility | Contract tests compile native code but did not prove the required Wasm target type-checks | added `wasm32-unknown-unknown` and `cargo check --target wasm32-unknown-unknown --locked` |
| Security scans | GitGuardian is required by PR policy; allocator check remains mandatory | retained |
| Advisory reviewer | Cubic is advisory/neutral and is not treated as a passing correctness gate | retained |

## Why no second workflow

The existing workflow already owns exact-head correctness. Adding a parallel workflow would duplicate
the Python, Node and Rust jobs for the same push. The supporting path instead adds one cheap demo
step to the existing Python/browser job and the missing checks to the existing Guard job.

## Local equivalent

```bash
python3 scripts/hackathon/verify.sh
```

This uses the existing
`x3-lang/near/demo/judge/collect_evidence.py --with-contract` recorder. It records a command only
after the command exits, with its exit code and reported counts. A skipped or unavailable check is
`not_run`, never a pass.

## Remaining CI limitations

- GitHub Actions action references use version tags rather than commit SHAs.
- The duplicate push/PR executions remain visible in the checks list.
- CI cannot prove a wallet signature, deployed Guard authorization, live quote, deposit or
  settlement.
- Advisory review services are not security audits.
