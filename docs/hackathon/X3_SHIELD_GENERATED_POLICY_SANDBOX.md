# Generated policy to native-NEAR enforcement

Scope: a disposable local NEAR Sandbox, compiled WASM and the native-vault subset
of X3Lang. This is not a public deployment, an owner signature, an audit, an
isolated production signer, or a cross-chain settlement demonstration.

## The experiment

The harness reads `x3-lang/examples/near_native_vault_supplier.x3`, compiles its
explicit `near_native_vault_v1` profile, and uses the verified compiler output
as the actual vault initializer. It does not independently fill in cap, budget,
recipient or expiry in Rust.

Before admitting spendable test escrow, the harness reads the deployed account's
code hash and the vault's six configuration fields at one common finalized
local block. Both must match the generated constructor and the SHA-256 of the
WASM bytes supplied to the deployer. A different code hash, account, field,
block, unexpected RPC error or unsupported source clause aborts the run.

The added `configuration()` contract method is read-only. It adds no persisted
field, changes no payment predicate, and grants no signing or administrative
permission. Contract and source metadata remain distinguishable: source digest,
policy version and manifest identity are not new fields stored by the vault.

Two distinct local vault accounts run identical compiled WASM. The second source
snapshot changes exactly one authored clause, reducing `per_payment` from
200000000000000000000000 to 50000000000000000000000 yoctonear. Compilation uses
the same pinned chain timestamp. The harness asserts that the resulting six-field
constructors differ **only** in cap. Separate deployment account names and scoped
credentials are explicitly recorded; they are not passed off as one deployment.

| Scenario | Required observation |
| --- | --- |
| Original policy, request `once` for 0.1 test NEAR | `Paid`, exact recipient balance increase |
| Lower-cap source, same nonce/recipient/amount on the fresh second vault | Exact cap refusal, no recipient balance, escrow or nonce-record change |
| Second vault, 0.01 test NEAR valid control | `Paid`, exact recipient balance increase |
| Restricted key attaches one yoctonear to the permitted method | Exact `DepositWithFunctionCall`, no payment record |
| Second vault after actual Sandbox block/time advancement | Observed timestamp at/after generated deadline; fresh nonce refused for expiry, not revocation; balances and ledger unchanged |
| Original lifecycle | Direct-transfer refusal, wrong recipient, cap, allowed payment, replay, active recovery refusal, revocation, fresh revoked request, forbidden recovery, owner recovery |

Past-expiry execution is tested on an actual node using `fast_forward`; it is
not an exact-one-millisecond boundary measurement. The contract unit suite owns
the exact boundary and callback/pending-accounting cases. Concurrent pending
payouts and failure callbacks must not be described as new live campaigns merely
because their unit tests pass.

## Reproduce

Use the exact proposed Git commit and the repository's existing pinned Rust
1.99.1 and NEAR Sandbox 2.14.0 setup. The wrapper verifies the Sandbox binary
SHA-256 before starting it. Python 3.12+ and the compiler dependencies are now
required by the Rust harness as well as the standalone compiler.

```bash
python3 -m venv .venv-shield
.venv-shield/bin/python -m pip install --require-hashes \
  -r scripts/hackathon/requirements-judge.txt
export PATH="$PWD/.venv-shield/bin:$PATH"
python3 -m unittest discover -s scripts/hackathon \
  -p test_native_vault_binding.py -v
NEAR_NETWORK_ID=sandbox bash scripts/hackathon/vault_sandbox_smoke.sh
```

Do not substitute a public RPC endpoint or a real wallet. On an older-glibc
machine, the offline Docker path requires a pre-provisioned compatible image
with Python and compiler dependencies; this continuation does not establish
that compatibility path has been exercised.

The exact-head vault workflow retains the unit, lint, formatting, raw-response
and real compiled-WASM lifecycle gates. It requires three raw permission captures,
two generated-configuration records, the cap-change result, the expiry result,
and completion. A missing dependency or generic error is never an attack success.

## Evidence and replay

`X3_VAULT_SANDBOX_GENERATED_POLICY` records the source snapshot, independently
supplied compile context, generated manifest, observed configuration, observed
code hash, and common local block identity. Each record can be rechecked through
`scripts/hackathon/native_vault_binding.py verify` by supplying its `binding`,
`observation`, and `now_ms` fields on standard input. Use compiler sources from
the same reviewed commit; source identities are deliberately checked.

The raw permission captures retain the earlier strict original-byte verifier.
The policy readbacks use the trusted local node's JSON-RPC results. Neither
format independently authenticates a node, validator finality or owner consent.
Rehashing or completely replacing a self-contained log cannot establish that it
was produced by a genuine run; preserve the GitHub run/artifact provenance and
pin the source/context independently when replaying it.

These two authorities are deliberately distinct:

* the local harness verifies source/configuration consistency and node execution;
* the owner separately authorizes any public deployment, keys, funds or release.

The harness still retains disposable full-access setup credentials. This does not
prove production key isolation. The vault retains its 64-payment lifetime bound,
provisional recovery reserve and documented callback/migration limitations.

## Verification history and release gates

This continuation starts from PR #164 at
`e6ec0ba54e8e039ca3e1c41872e6c4c6b96082cb`. Thirteen new Python tests first failed
because the binding module was absent, then passed after implementation. They
check every configuration field, code/account mismatch, malformed/stale inputs,
manifest tampering, source-only cap changes and the subprocess boundary. Synthetic
comparator tests are not counted as node execution.

At `f8e2cecf95bb5c188dd4f270479d2f265f61496e`, actual Sandbox job
`128958800125` in workflow `42555058146` reached the generated-policy readback
and failed with `MethodResolveError(MethodNotFound)` before escrow funding.
The read-only accessor was intentionally absent at that test-first checkpoint.
The following implementation adds it and a serialized-state preservation test.
Harness compilation, integration tests and strict Clippy passed at that checkpoint;
the formatter's diagnostic patch was applied without changing execution semantics.

Exact source SHAs, CI run IDs, actual outcomes and artifact hashes are recorded
in the PR verification comment. Earlier manual-configuration Sandbox archives
are immutable and are not relabeled as this generated-policy path.

No release gate is waived: final-head CI, complete authorized peer synchronization,
independent review and explicit owner merge approval remain required. Public
funding, deployment, publication and submission require separate authorization.
Rollback is a reviewed revert of this branch's implementation; no public funds
or deployed state were changed by this local testing increment.

## Time-advance execution bound

The first accessor-enabled run at `1f4425d` did not finish the Sandbox step
promptly. Its in-progress log was unavailable, so no successful lifecycle is
claimed from it. Primary SDK source shows its HTTP send occurs before the
30-second polling timer; nearcore also crosses each epoch boundary through
ordinary block production. The revised local node explicitly uses a 100,000-block
epoch, making the 50,000-block request an intra-epoch experiment. This is recorded
in the log, not claimed equivalent to public-network consensus settings.

Time advancement now uses the loopback-only, 30-second-timeout RPC client rather
than the SDK's blocking send. The response is only an acknowledgement: the harness
still reads actual finalized chain timestamps, requires the generated deadline
to have passed, and then submits a fresh signed request. Transport failure,
insufficient advancement, or wrong policy reason fails the gate. No clock or
contract-state field is patched to force a denial.
