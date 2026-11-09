# Native vault policy configuration — offline increment

Plan: [X3 Shield execution plan](X3_SHIELD_EXECUTION_PLAN.md), Task 2.
Draft PR: #164, stacked on #159. Implementation baseline:
`bb81124ae7b4524a6a29165576c8ecbc8d23da9a`.
Branch owner: ChatGPT, `feat/shield-native-policy-binding-20261108` only.

## What this increment establishes

A supported `.x3` mandate produces the existing native vault constructor's
`owner`, `agent`, `recipient`, `cap`, `budget`, and `deadline_ms` arguments.
The existing `near_mandate.parse_mandate` parser handles the source, under an
explicit `near_native_vault_v1` profile. No second parser or policy evaluator is
introduced. Default Guard parsing and rolling-budget semantics remain unchanged.

This is **offline source-to-configuration binding**. It does not establish that a
running vault uses the configuration, that an owner authorized it, or that the
compiled policy constrained a transaction. PR #159's separate Sandbox evidence
must not be relabeled as evidence for this new compiler path.

## Supported source

See `x3-lang/examples/near_native_vault_supplier.x3`:

```text
mandate native_supplier {
    target near_native_vault_v1
    owner owner.testnet
    agent agent.testnet
    spend Near.NEAR
    per_payment 200000000000000000000000
    lifetime_budget 400000000000000000000000
    allow recipients allowed.testnet
    expires 1h
}
```

Amounts are integer **yoctonear**, not decimal NEAR. `lifetime_budget` means the
cumulative sum of spent and reserved payouts, not a rolling daily allowance.
There is exactly one recipient. All four principals (including the destination
vault) must be distinct valid `.testnet` accounts; this conservative compiler
restriction is stricter than the contract's constructor. Only `sandbox` and
`testnet` deployment contexts are supported. An account suffix is not proof of
which network a node actually serves.

Reject, rather than discard: `per_24h`, `approve`, `refund_to`, `allow assets`,
fee or slippage requirements, token/swap assets, multiple recipients, unknown
clauses, duplicate clauses, wrong/missing target, invalid units or limits, bad
principals, and out-of-range expiry. Guard/browser commands do not implicitly
select this backend; use its explicit CLI.

## Compile and verify without signing

After separately building the reviewed vault WASM, use its actual SHA-256. The
following commands only read files and print JSON; they perform no RPC or signing.
Replace the WASM path and the policy/principals with the intended reviewed inputs.

```bash
WASM=x3-lang/near/x3-agent-vault/target/wasm32-unknown-unknown/release/x3_agent_vault_testnet.wasm
CODE_SHA=$(sha256sum "$WASM" | cut -d' ' -f1)
POLICY=x3-lang/examples/near_native_vault_supplier.x3
APPROVED_AT=$(python3 -c 'import time; print(time.time_ns() // 1000000)')
python3 x3-lang/near_native_vault.py compile "$POLICY" \
  --network sandbox --vault-account vault.testnet \
  --wasm-sha256 "$CODE_SHA" --policy-version 1 --at-ms "$APPROVED_AT" \
  > /tmp/native-vault-manifest.json
python3 x3-lang/near_native_vault.py verify "$POLICY" \
  --network sandbox --vault-account vault.testnet \
  --wasm-sha256 "$CODE_SHA" --policy-version 1 \
  --manifest /tmp/native-vault-manifest.json --approved-at-ms "$APPROVED_AT"
```

`--at-ms` is an explicit compile/replay time for deterministic testing. Do not use
an attacker-supplied timestamp for deployment admission. `--approved-at-ms` is
mandatory for verification and must be retained separately from the manifest as
part of the reviewed context. The variable in the example is only a proposed
compilation time, not proof of owner consent. Verification rejects
future manifests, manifests older than five minutes, and expired policies.
Recompiling is a new proposed configuration: it is not automatic owner approval
and must not silently extend an owner's already-approved deadline.

## Trust and identity boundaries

Verification **recompiles** independently trusted source bytes with independently
provided network, vault, WASM identity, expected policy version and reviewed
compilation time. A manifest's own hash is not sufficient, even when an attacker
recomputes it after altering a constructor value. Duplicate JSON keys, floats,
nonfinite numbers, unexpected fields and mismatched types are rejected. The parser
reads a private temporary snapshot of the exact source bytes that are hashed.

The output records the hashes of five relevant compiler source files; this is
not a complete transitive-dependency attestation or reproducible binary proof.
The caller supplies the expected WASM hash; this module does not fetch or
verify deployed code. Network, code hash, source hash and policy version are
manifest metadata, **not new contract state fields**. The current vault does not
store or enforce those metadata fields. The 64-record contract limit is disclosed,
not replaced with a larger or renewable budget.

`owner_authorized=false` and `deployment_verified=false` remain explicit. This is
an unsigned configuration artifact, not an owner signature, wallet authorization,
chain-finality proof, or permission to deploy. Anyone can author their own policy;
the trust anchor must come from the owner outside the agent's control.

## Verification ledger

Tests were written first: 17 local unittest methods failed with the explicit
missing-backend assertion (exit 1). The original `near_mandate.py` materialized
snapshot was verified against Git blob `a52338d4a536fe82163ba1fda0fc7b9982616a44`
before the narrow parser-profile edit. Python syntax checks passed locally.
This session initially lacked the full repository dependencies and Rust toolchain;
syntax checks are not claimed as a complete compiler integration run.

The dedicated exact-head workflow runs the real shared-parser integration tests
in an isolated environment and retains the tracked source snapshot plus test log.
That artifact is compiler-input/test evidence, never chain evidence. Final test
results and exact source heads are recorded in PR comments without moving the head.
The first compiler-source artifact (`8a0d325`) was downloaded and digest-checked
before local integration replay. All 17 initial tests passed locally and in CI.
An added regression then exposed a coordinated `compiled_at_ms` / `deadline_ms`
rewrite that could otherwise extend relative expiry despite a consistent hash.
The test failed with "ValueError not raised". Verification was changed to require
an independently pinned `expected_compiled_at_ms`; the regression then passed.
With CLI-time and short-expiry coverage added, 20 focused tests passed. A separate
surface regression checks the checked-in native example and its explicit budget.

Existing mandate/boundary/Python-browser parity: 205 passed, one existing skip.
Browser suite: 88 passed. The initial local full-suite attempt stopped at a
missing root-level `scripts/judge_evidence.py` in the partial `x3-lang` archive;
it is recorded as a collection blocker, not a pass. The new CI job runs the full
Python and browser suites from the complete checkout using the existing
hash-locked judge dependency environment and preserves full tracked source inputs.

Changing the shared parser shifted existing mutation anchors by 19 lines. The
mutation runner's exact-expression and line checks correctly rejected those
stale pins; they were repinned without deleting mutations or loosening the
classifier. All 12 curated mutants were caught locally after repinning, with
zero survivors, invalid cases or timeouts; all five mutation-runner tests passed.

The first full CI regression run correctly rejected the new example because it
was not yet classified in the surface-boundary table. The table now records the
observed `X3_PARSE_NO_INTENT` result for the generic intent parser, while the
explicit native profile reads the same example. No parser refusal is suppressed.
After the full source archive was obtained, local regression results were 613
passed, one skipped and two failures in `test_judge_evidence.py` caused by the
snapshot's missing `.git` metadata. These are not claimed as green; the complete
CI checkout is the verification environment for those tests. The local virtual
environment reuses preinstalled platform dependencies without global installation;
CI installs the existing hash-locked judge runtime. Final-head results are recorded
separately in the PR.

The existing broader Python/browser/VM/security and peer-mirror checks remain
required; a green new workflow cannot override a failing peer gate.

## Remaining acceptance gates

1. Read back actual contract configuration and code identity on the disposable
   Sandbox, and compare them with the generated artifact before requesting funds.
2. Use generated constructor arguments rather than hand-edited values. Change
   one supported `.x3` rule and show the same actual payment succeed/refuse on
   the corresponding reviewed deployments, without changing contract logic.
3. Add real expiry/deposit/budget boundary cases and preserve credential separation.
4. Complete exact-head checks, authorized peer synchronization and independent
   review. Public deployment, funding and release require separate owner approval.

Rollback: revert this isolated compiler increment or close its draft PR. No
contract storage layout, deployed state or funds are changed by this increment.
Do not relax any existing refusal, mirror or release gate to merge it.
