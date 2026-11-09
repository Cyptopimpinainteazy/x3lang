# X3 Shield progress: strict raw-RPC permission classification

Date: 2026-11-08. Continuation of [the execution plan](X3_SHIELD_EXECUTION_PLAN.md), Task 1.
PR: [#159](https://github.com/Cyptopimpinainteazy/x3lang/pull/159).
Starting branch head: `b399c1ab277e1d04081890753f8cb7b5dbd15e70`.

## Status and scope

Implemented a dependency-free, read-only Python classifier for original NEAR
JSON-RPC permission-error responses. This is a behavioral verifier, not another
source-text assertion.

A later increment independently connected the same strict wire shape to the Rust
Sandbox collector, removed the temporary inconclusive quarantine, and passed the
real local lifecycle. That live increment currently performs its own typed
classification; the Python classifier remains an independent behavioral verifier
and is not imported by the Rust harness. Task 1's core lifecycle is therefore
verified locally, while expiry/deposit edge cases, exact-head CI and review remain
open. Issue #119 remains open.

This increment adds no transactions, deployment, signing, contract changes,
dependency changes, historical evidence changes, or publication. `xxxstar` remains
read-only. Synthetic inputs live only in the test module, not in `evidence/`.

## Files and interfaces

- `scripts/hackathon/near_permission_refusal.py`
  - `Expectation(request_id, case, method_name=None, tx_receiver=None, ak_receiver=None)`
  - `classify(raw_response: bytes, expected: Expectation) -> dict[str, Any]`
  - `RefusalEvidenceError`: missing, malformed, mismatched or ambiguous response.
  - CLI requires `--response` and `--expectation`; writes only to stdout/stderr.
- `scripts/hackathon/test_near_permission_refusal.py`: 27 unittest methods,
  including synthetic positive/negative inputs and CLI execution.
- `.github/workflows/x3-agent-vault-testnet.yml`: add path triggers and a separate
  behavioral-test step to the existing refusal-source-guards job. Preserve every
  original source guard, Rust check, lint, format and WASM build.

## Exact error matching

The classifier requires JSON-RPC 2.0, the independently expected string request
ID, integer server error code -32000, HANDLER_ERROR and INVALID_TRANSACTION. It
reads the subtype only from:

`error.data.TxExecutionError.InvalidTxError.InvalidAccessKeyError`

Supported case-to-reason mappings:

| Case | Required exact subtype and details |
| --- | --- |
| `direct_transfer` | `RequiresFullAccess` |
| `wrong_method` | `MethodNameMismatch` with the independently expected method name |
| `wrong_receiver` | `ReceiverMismatch` with both independently expected receiver IDs |
| `attached_deposit` | `DepositWithFunctionCall` |

Generic InvalidTransaction text, logs, stale nonces, expired transactions, bad
signatures, missing keys, insufficient balances/allowances, timeouts, transport
errors, pending/successful results and unrelated permission subtypes cannot pass.
Duplicate JSON keys, ambiguous enum variants, non-finite numbers (including
floating-point overflow), oversized inputs and excessive nesting also fail.
Unknown or conflicting response formats remain inconclusive; there is no
substring-matching or generic-error fallback.

Wire-format reference checked during implementation:
https://docs.near.org/api/rpc/transactions#invalid-transactions

## Evidence boundary

A response JSON file can be fabricated. Matching its structure does NOT prove a
node produced it, a transaction was signed or submitted, a key was restricted, a
balance stayed unchanged, or any chain state is authenticated. The expectation
must be prepared independently, not copied from the response to manufacture a
match. Request-ID matching is correlation, not cryptographic authentication.

Successful output is explicitly `EXPECTED_PERMISSION_ERROR_IN_RPC_RESPONSE`.
It includes a SHA-256 digest of the original response bytes and always sets:

- `live_execution_verified: false`
- `signed_transaction_verified: false`
- `cryptographic_proof: false`
- `included_receipt_id: null`

The CLI also hashes the original expectation bytes. Hashes identify files; they
are not signatures. A mismatch exits 2, writes INCONCLUSIVE to stderr and emits no
success JSON. An included contract failure must use a separate verifier; this
classifier deliberately rejects responses containing a `result` field.

## Actual local verification

Environment: Python 3.13.5 in an isolated source snapshot, not a full repository
checkout. No local Rust toolchain or reachable remote execution was available.
Desktop Commander again returned its monthly usage limit; it was not retried.

1. RED: wrote the 27 tests first; all failed with the explicit missing-classifier
   assertion. This establishes an absent implementation, not a reproduced chain
   failure.
2. GREEN: implemented the classifier; all 27 methods passed.
3. Additional RED: added `1e999` and `-1e999`; the numeric rejection test failed
   because JSON floating-point overflow became infinity.
4. Fixed finite-number handling; all 27 methods passed again.
5. Direct invocation and unittest discovery both passed; Python compilation passed.
6. Four independently applied temporary mutations were each detected (nonzero test
   exit): accept any subtype; ignore request ID; accept result and error together;
   allow duplicate keys with last-value-wins behavior. This is a four-mutation
   spot-check, not a comprehensive mutation score. The temporary copies were not
   committed and the real implementation remained unchanged.

Reproduce the new suite from the repository root:

```bash
python3 scripts/hackathon/test_near_permission_refusal.py
python3 -m unittest discover -s scripts/hackathon -p 'test_near_permission_refusal.py' -v
python3 -m py_compile scripts/hackathon/near_permission_refusal.py scripts/hackathon/test_near_permission_refusal.py
```

Verified local Git blob identities before publishing:
- Classifier: `d45da28d841e953f7d44788dcd042c9065ea1dd3`
- Tests: `58388c847a8fbd12e0286b08a539d22a6e934e67`

These results apply only to the new Python behavior. No full repository test run,
new Sandbox run, public-chain run or independent security review is claimed.
Exact-head CI status belongs in the PR discussion after the final commit exists.

## Baseline CI blocker retained

At the starting head, History/Reproducibility, Vault Pilot, Judge Quick Gate and
Security Audit workflows completed successfully. Exact-Head Local Execution
Proof failed in its cross-repository job at `Checkout matching peer feature
branch`; its separate x3lang-proof job passed. Run: `42505978188`.

No matching peer branch was created by this increment and no gate was suppressed.
Do not describe the branch as all-green or ready to merge based on the classifier.

## Next integration gate

The live Rust collector now captures the original JSON-RPC error structure before
SDK conversion and matches exact `RequiresFullAccess` and `MethodNameMismatch`
values during a passing Sandbox lifecycle. It exercises a permitted control, a
fresh post-revocation nonce, contract-level refusal reasons and balance/state
controls. The next hardening step is to persist and replay the original response
bytes through this Python classifier as an independent second check.

Expiry and attached-deposit cases still need real Sandbox coverage. The
language-to-vault configuration binding remains the following milestone, not a
completed feature.

Rollback is a reviewed revert of this increment. There are no deployed contracts
or funds to roll back, and the earlier evidence quarantine must remain intact.
