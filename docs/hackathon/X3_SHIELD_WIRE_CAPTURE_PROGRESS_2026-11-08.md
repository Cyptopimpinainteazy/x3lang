# X3 Shield: original-wire capture continuation — 2026-11-08

Plan: [X3_SHIELD_EXECUTION_PLAN.md](X3_SHIELD_EXECUTION_PLAN.md).
Tracking: draft [PR #159](https://github.com/Cyptopimpinainteazy/x3lang/pull/159), related to #119.
Continuation baseline: `a70970754e1058d88db9f365467010d419c70d37`.
Scope: native-NEAR local Sandbox observation quality, not new public custody deployment.

## Why another increment was needed

At this baseline the PR already contained a successful local Sandbox lifecycle,
exact access-key subtype checks, stable contract failure reasons and an actual
Sandbox CI job. That work was preserved rather than repeated.

The live Rust sender still reused one RPC request ID and deserialized untrusted
response bytes directly to `serde_json::Value`. The independent Python verifier
had stricter duplicate-key, envelope, ID and permission-detail checks, but the
live sender did not use it. This continuation closes that wiring gap.

Ruling: keep the existing contract, policy checks and lifecycle unchanged; make
original-wire verification mandatory in the sender. This avoids a competing
classifier and prevents a synthetic-test-only verifier from being mistaken for
the implementation exercised on the node.

## Implemented connection

Files:
- `scripts/hackathon/near_permission_refusal.py`: adds `classify_capture(bytes)`
  and `--capture-stdin`, preserving the original strict classification checks.
- `scripts/hackathon/test_near_rpc_capture.py`: 19 behavioral regression tests.
- `x3-lang/near/agent-vault-sandbox-tests/src/raw_capture.rs`: bounded loopback
  sender, request identity, original-byte capture and mandatory classifier call.
- `x3-lang/near/agent-vault-sandbox-tests/src/main.rs`: both actual signed
  restricted-key attempts call the new sender with independently selected cases.
- `.github/workflows/x3-agent-vault-testnet.yml`: runs the new Python tests and
  Rust integration tests; retains all existing jobs and requires two live captures.
- `scripts/hackathon/vault_sandbox_smoke.sh`: preflights the classifier runtime.

The Rust sender hashes the complete Borsh-serialized signed-transaction bytes,
adds a checked process-local sequence and uses `x3-vault-<sha256>-<sequence>` as
the JSON-RPC request ID. It serializes the outgoing request once and submits those
exact bytes. The HTTP response is captured before JSON deserialization.

The Python verifier requires the exact request/response/expectation schema,
canonical base64, size bounds, a matching response ID, the expected `send_tx`
request with `FINAL`, and the specific access-key error and method/receiver details.
Duplicate JSON keys, pending/success results and unrelated errors fail closed.
No existing test expectation or permission rule was loosened.

The client accepts a literal loopback IP endpoint only, disables HTTP proxies and
redirects, requires HTTP 200, bounds the streamed response to 1 MiB and uses a
60-second HTTP timeout. Missing Python or a failed classifier is a hard failure.
`X3_VAULT_SANDBOX_RAW_CAPTURE` records original request/response bytes in base64,
request/response hashes and the classifier result in the local CI artifact. These
records contain disposable local transaction bytes and public signatures, not
secret keys. The artifact is not a public-chain receipt.

## Verification observed during this continuation

### Python regressions

In an isolated source snapshot using Python 3.13.5:

- Wrote 19 tests first; all 19 failed because `classify_capture` was absent.
- Implemented capture validation; all 19 passed both directly and under unittest discovery.
- Four temporary mutations were each caught: disabled transaction-byte binding,
  disabled response-ID checking, duplicate-key acceptance and wrong request-method acceptance.
  This is a bounded mutation spot-check, not a comprehensive mutation score.
- Python compilation and `bash -n scripts/hackathon/vault_sandbox_smoke.sh` passed.
- Uploaded classifier, harness and workflow bytes were compared to their local Git blob hashes.

The source snapshot is not a full repository clone. No local Cargo, Rust formatter
or local Sandbox execution is claimed from this session; those run in GitHub CI.

### Real compiled-WASM Sandbox observation at `526a875`

Pinned source: `526a875f580374d3a37112d071c7cdb0ee4b1be0`.
[Vault workflow run 42528185774](https://github.com/Cyptopimpinainteazy/x3lang/actions/runs/42528185774).
Sandbox job: `128876336398` — SUCCESS, including both mandatory raw capture records.

This run exercised the actual sender, strict Python classifier, permission refusals,
contract-policy refusals, allowed payment, replay, revocation and owner recovery.
The complete local lifecycle reached its success marker.

The harness unit-test log showed 8 passing tests. The Python/source-guard job and
existing vault test/lint/format/WASM job passed. However, strict harness linting
failed because the pinned Rust 1.99.1 compiler deprecated `AtomicU64::fetch_update`.
The code was changed to the compiler-recommended `try_update` in `c8afe0c` without
suppressing the warning or changing the gate. New-head lint/format and all applicable
checks must be re-read; the passing Sandbox run alone is not an all-green result.

Compiled contract WASM SHA-256 reported by this CI run:
`1d50f7e7e235308f19fd8cfb0731fc5605e8a5b3b33a6fa79d2ec0a2269e460c`.
This is this run's artifact identity, not a claim of byte equality with builds made
using the older toolchains listed in earlier local evidence.

### Independent artifact replay

Downloaded actual [artifact 13636118263](https://github.com/Cyptopimpinainteazy/x3lang/actions/runs/42528185774/artifacts/13636118263),
verified ZIP SHA-256 `c9a60097f9efe06b8e23d4029095541937b1f6832f1f874910ae6eaeb6449f4c`,
and read its single `x3-vault-sandbox.log`. Log SHA-256:
`7b942791004c8a4329ce355f1d50eddf278513eba33e194c07781291d12f8aa4`.

Reclassified both original request/response pairs locally. The reconstructed reports
matched the recorded reports exactly, including their byte hashes and expected
`RequiresFullAccess` / `MethodNameMismatch:recover_available` reasons.
This replay did not sign, broadcast, query a public RPC or run another node.

## Runtime requirements and limitations

The combined harness requires Python 3.12+ in the runtime executing the binary, in
addition to the existing pinned Rust/Sandbox build prerequisites. On native Ubuntu
24.04 the wrapper preflights Python before building. On older glibc systems the
network-disabled Docker runtime must already contain Python 3.12+ and the required
libraries. Set `X3_SANDBOX_RUNTIME_IMAGE` to an explicitly pre-provisioned compatible
image. A minimal `ubuntu:24.04` image without Python fails the preflight; nothing is
installed or pulled as a fallback. The Docker compatibility path was syntax-checked
but was not executed in this session. The repository and pinned Sandbox binary
remain read-only mounts; execution retains `--network none` and the caller's UID/GID.

The recorded SHA-256 covers the complete signed-transaction blob; it is not the
NEAR transaction identifier derived from the unsigned transaction. Process-local
sequence uniqueness is not a global replay-protection claim.

`request_payload_bound=true` establishes byte correlation, not authenticated
transaction semantics. The classifier deliberately retains:

```text
live_execution_verified=false
signed_transaction_verified=false
cryptographic_proof=false
included_receipt_id=null
```

Those fields describe the classifier's limits. The separate GitHub job establishes
that the local harness was executed. A supplied capture can still be fabricated;
this parser does not independently authenticate a node or validate signatures.

## Remaining work and release gate

- Re-read checks at the final proposed head; do not carry forward older green results.
- Independent review and explicit owner merge approval remain outstanding.
- Preserve the applicable peer-mirror gate; this continuation does not write `xxxstar`.
- Add actual Sandbox expiry and attached-deposit scenarios.
- Bind a supported `.x3` policy to compiled, reviewed and observed vault configuration.
- Keep trusted owner keys, missing-callback recovery, storage limits, migration,
  public deployment and production readiness outside the demonstrated claim.

No contract code/state layout, dependency lockfile, immutable historical evidence,
public deployment, mainnet action, real-wallet signing, funding, visibility change,
merge or `xxxstar` write occurred in this continuation. Earlier changes by other
workers are described by their own commits and evidence, not attributed to this run.

Rollback is a reviewed revert of this continuation's testing/collection changes.
Do not restore generic errors as successful-refusal proof to regain a green badge.
Final exact-head results belong in the PR verification comment so recording results
does not recursively move the commit being verified.
