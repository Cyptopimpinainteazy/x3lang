# Judge reproduction report

Previously tested master: `5bc7e51d01c03ea21ae7dfef29df3494577c58e1`. New inspected master: `038e82d73a522642ae755af48eb6acb838902329`; release-report candidate verification is recorded separately in its PR, not inferred from prior runs. Date: 2026-11-08 UTC. Classification: **LOCAL**. Source-only clean clone; pinned Python dependencies, installed Node/Rust and warm host dependency caches. This is not a freshly provisioned operating system or public judge-machine certification.

## Exact-commit results

| Commit | Command | Exit | Result/evidence |
|---|---|---|---|
| `8fa8266b25fedb04011680976f79f38ba9b1aa07` | `/tmp/x3-points-judge-venv/bin/python scripts/judge_evidence.py --mode full --report-dir /tmp/x3-shield-judge-full-8fa8266` | 0 | 8/8 executed gates passed; [saved JSON](evidence/points-maximizer-20261108/judge-full-8fa8266.json) |
| `5bc7e51d01c03ea21ae7dfef29df3494577c58e1` | `/tmp/x3-points-judge-venv/bin/python scripts/judge_evidence.py --mode full --report-dir /tmp/x3-shield-judge-full-5bc7e51` | 0 | 8/8 executed gates passed; clean worktree; [saved JSON](evidence/points-maximizer-20261108/judge-full-5bc7e51.json) |
| `5bc7e51d01c03ea21ae7dfef29df3494577c58e1` | `PATH=/tmp/x3lang-judge-py312-bin:$PATH X3_JUDGE_VENV=/tmp/x3-points-judge-venv bash scripts/hackathon/judge_one_command.sh /tmp/x3-shield-judge-5bc7e51` | 0 | Scenario freshness, demo, all 48 browser tests and four quick gates passed |
| `5bc7e51d01c03ea21ae7dfef29df3494577c58e1` | `PYTHONPATH=x3-lang /tmp/x3-points-judge-venv/bin/python -m unittest scripts/hackathon/test_first_swap_archive.py scripts/hackathon/test_first_swap_proof_bundle.py scripts/hackathon/test_two_chain_settlement_proof.py scripts/hackathon/test_two_chain_baseline.py tests/proof/test_x3lang_proof_scripts.py -v` | 0 | 75 tests passed (archive, finality, tamper/source-binding regressions); LOCAL |
| `5bc7e51d01c03ea21ae7dfef29df3494577c58e1` | `PYTHONPATH=x3-lang /tmp/x3-points-judge-venv/bin/python scripts/hackathon/verify_first_swap_archive.py --live` | 2 | HARD_DENIED: read operation timed out. No new TESTNET proof was established and no certificate changed |
| `5df23da8f73edd55a37dd3967cd60b8b3d955ca6` (#128) | `node --test docs/hackathon/attack-lab-policy.test.mjs` | 0 | 12 passed, zero skipped; independent reviewer reproduced |

Full mode ran Python policy/chain tests, browser signature/Guard tests, allocator graph gate, Guard tests, VM tests, Rust formatting, VM Clippy with warnings denied and Guard `wasm32-unknown-unknown` check. No test was weakened. At the two historical tested commits, full omitted a quick-only script test (#146), so both modes were run separately. New master integrates #146 and includes that test and all browser files; its results must be recorded under its own SHA. The manual/advisory two-build CI job is skipped on PR events and is not a passing deployed-byte proof.

## Python and environment

The initial default invocation in the restricted shell exited 2 because no Python 3.12 was discoverable. A pre-existing verified Python 3.12.14 environment contains exactly the nine pinned distributions: cffi 2.1.1, cryptography 50.0.2, iniconfig 2.3.1, packaging 26.3, pluggy 1.7.0, pycparser 3.12, pycryptodome 4.3.0, Pygments 2.21.0 and pytest 9.2.0. Exposing the already installed interpreter on PATH preserved all lock checks; no global pip or gate bypass was used.

## Proof boundary and retained logs

Neither judge report was supplied a settlement receipt. Both explicitly report `claimed_onchain_proven: false` and no refreshed source/destination proof. The archived Tidepool trade is separately classified TESTNET historical evidence with a trusted relay. It is not mainnet evidence, permissionless atomicity or native X3VM on-chain execution.

Logs and exit files are retained locally at `/tmp/x3-points-audit-20261107/`: `judge-full-8fa8266.log`, `judge-full-5bc7e51.log`, `judge-one-command-5bc7e51.log`, `settlement-proof-regressions-5bc7e51.log`, `first-swap-live-5bc7e51.log`, `preview-red.log`, `preview-exact-5df23da.log`. The saved judge JSON binds SHA, clean-worktree state, exact gate exit codes and output hashes. Rust output is hashed by the judge harness rather than retained in raw form. No credentials or private receipt files are included.

Not tested/proven: fresh OS setup; new on-chain attacks; custody-vault public deployment; mainnet; external RPC recovery after the observed timeout; unmerged PR behavior; a report-commit full gate. Re-run on every new proposed release SHA rather than transferring this result to future master.

RPC diagnosis after the failed live run: both configured Base Sepolia providers returned HTTP 403 to minimal chain-ID requests; the configured NEAR endpoint responded with `chain_id: testnet`. No provider was silently replaced or quorum reduced. The archive remains historical evidence; fresh live proof is still incomplete.
