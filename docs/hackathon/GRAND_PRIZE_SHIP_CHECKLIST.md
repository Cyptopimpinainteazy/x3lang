# Grand-prize ship checklist (internal)

Date: 2026-11-08 UTC. Latest inspected master `038e82d73a522642ae755af48eb6acb838902329`; tests remain bound to the commits named in the reproduction report. Deadline: 2026-11-14 23:59 UTC; unrelated-feature freeze November 12. No official score or prize probability is implied.

| Priority | Deliverable / next executable action | Owner / dependency | Current evidence | Release gate |
|---|---|---|---|---|
| P0 | Keep X3Lang and xxxstar private; recheck `gh api --method GET repos/Cyptopimpinainteazy/x3lang` | Owner; release coordinator | Owner-directed private restoration verified on November 8; xxxstar private | No visibility change, publication, final submission or new signing without authorization |
| P0 | Reconcile the one remaining draft #159 in [matrix](PR_MERGE_MATRIX.md); retain the historical dependency graph | Existing branch owners; coordinator reviews | One draft remains held on failed peer parity; earlier 30-PR state preserved | Review, no unresolved findings, all current required/applicable checks, no stale head; never bypass |
| P0 | Revalidate integrated #143 allocator failure-path tests | #143 owner; inspect existing work before editing | Revised quote-style fix is integrated; historical finding is not current | Actual regression red→green, independent review and CI |
| P0 | Resolve #159 peer harness/raw_capture drift | Peer owner; xxxstar read-only for this agent | Earlier #112 missing-branch failure inspected; #159 log inspection pending | Matching approved peer branch/committed bytes; no removal of comparison |
| P0 | Guard #94→#98→#102→#103→#108→#132 and stress alternatives | Existing owners; deployment owner approval | Stacks integrated by others; current exact-commit release audit and deployment boundary remain | Full exact-head unit/WASM/Sandbox/adversarial/restart checks plus approved deployed-byte evidence |
| P0 | #117/#120 bootstrap + anchors; reconcile overlapping #116 | Existing owners | Both integrated by others; verify current semantics and tests | No implicit dry mode, explicit RPCs, bounded thresholds, nonempty anchors, review |
| P0 | Revalidate integrated #146 and rerun both judge modes | Existing owner | Historical full and quick passed separately; current coverage fix integrated | Full includes every quick test; no narrowing |
| P1 | Continue real settlement verification: diagnose RPC timeout; rerun read-only archive verifier | Coordinator; public testnet RPC availability | Historical signed evidence and 75 regression tests pass; fresh live rerun exit 2 timeout | Fresh source/Guard/destination canonical evidence; preserve trusted-relay limitation |
| P1 | Custody #125→#130 and #134, #136/#137; Base #138→#140→#142 | Existing owners; separate review paths | Integrated local proof implementations; no public vault deployments verified | Independent economics, callbacks, key scope, replay, quorum, recovery review; no public deployment without owner |
| P1 | Fresh Linux x86_64 machine reproduction | Coordinator; clean VM/container toolchain | Clean source clone with pinned Python tested, warm host caches | Real fresh environment, exact commit, command output and exit 0 |
| P1 | Verify exact deployed WASM and signature/trust-anchor evidence | Coordinator; version/deployment owner | Historical hashes documented; changed Guard stacks currently fail byte gate | Exact bytes and applicable finality independently reproduced; no compile-only claim |
| P2 | Finish fair public entrant evidence review | Coordinator | Veto/LeashPay local tests reproduced; other entrants UNKNOWN | Same proof standards, exact sources/SHAs, no invented rankings |
| P2 | Record/review 90–120-second video | Owner/recording operator | Storyboard exists; no recording verified here | Real explorer receipts, separate LOCAL/SANDBOX/TESTNET labels, no keys shown |
| P2 | Demonstrate user value and sustainable service cost | Product owner | Commercial proposal, no billable fee activation | Comparable evidence and adoption proof; no unsolicited fee activation |
| P3 | Public disclosure and final form | Owner only, later explicit approval | Not authorized; repository private | Owner review before deadline, consent, public repo/demo requirement handled only then |

## Stop/continue contract

Do not stop at a green local test when a relevant issue remains open. After bounded unsuccessful repairs, record a concrete blocker and continue independent work. A skipped job is not a passed test; a timed-out RPC is not a settled trade. No mainnet funds, unattended signing, new token transfers, deployment or final submission are authorized.

Local proof files and command details are linked in [JUDGE_REPRO_REPORT.md](JUDGE_REPRO_REPORT.md). This checklist records incomplete work rather than reclassifying the objective around the completed subset.
