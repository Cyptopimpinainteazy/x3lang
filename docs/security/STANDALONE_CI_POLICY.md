# Standalone X3Lang CI and explicit peer parity

Owner-authorized policy replacement, November 10, 2026. `xxxstar` remains
read-only. This changes the earlier requirement that every X3Lang feature branch
also exist with matching language/proof changes in the private parent repository.

## Deliberately reduced mirror coverage

Normal pull-request CI no longer checks cross-repository byte parity. A green
standalone PR therefore does **not** mean its source is identical to `xxxstar`.
The source-admission artifact says `cross_repository_parity: not_checked`.
Historical provenance, the original import and archived parity evidence remain
unchanged; new standalone development may diverge.

The automatic `Verify standalone committed proof inputs` job instead admits the
exact PR head with the existing whole-checkout source-binding verifier. It checks
tracked bytes, Git modes and index state, including hidden/ignored changes and
symlink targets. It also requires the existing nine compiler/VM proof inputs to
remain committed regular files. A wrong HEAD, missing proof, changed source,
unsupported file or unbound build input fails before any proof-execution claim.
Source admission alone is neither code execution nor a security audit.

The separate `x3lang-proof` job still runs the compiler/VM execution proof and
its admission/mirror-boundary regression tests. The actual Rust proof suites,
Guard unit/security mutations, real local Sandbox, persisted-restart checks,
deployed-WASM proof and reviewed upgrade-WASM hash gate are retained. This policy
does not change mainnet approval, source review or settlement-proof requirements.

## Optional explicit parity audit

`.github/workflows/x3lang-peer-parity.yml` is manual (`workflow_dispatch`) only.
An authorized operator chooses the X3Lang workflow revision and a full reviewed
40-character lowercase `xxxstar` commit SHA. The audit validates that SHA before
using `X3LANG_PEER_READ_SSH_KEY`, then checks out the fixed private repository
read-only with persisted credentials disabled. Missing credentials, unavailable
commits or drift still fail closed. The original mirror checker is unchanged:
it checks the nine required proof inputs plus both repositories' changed
`x3-lang/` and `scripts/proof/` paths since their `origin/master` merge bases.

No automatic peer branch creation, synchronization, credential export, write to
`xxxstar`, privileged pull-request trigger or `continue-on-error` was added.
No manual parity audit was executed as part of this change; no matching peer
commit is asserted. Existing deployments and archived evidence are unaffected.

## Reproduce admission and regression checks

From a complete clean checkout (report directory outside the checkout):

```bash
python3 scripts/proof/check_x3lang_checkout.py --expected-head "$(git rev-parse HEAD)" --output-dir /tmp/x3lang-source-admission
python3 -m unittest discover -s tests/ci -p 'test_*checkout*.py' -v
python3 -m unittest discover -s tests/ci -p test_proof_source_binding.py -v
python3 -m unittest discover -s tests/proof -p 'test_*.py' -v
```

Admission fixtures use real Git operations and do not execute Cargo. CI separately
executes `scripts/proof/x3lang_execution_proof.py` against the exact PR head.
Before/after binding is not a filesystem snapshot or a hermetic toolchain claim;
trusted checkout ownership and independent code review are still required.

If branch protections require the old mirror status name, the repository owner
must replace that required context with `Verify standalone committed proof inputs`
while retaining the execution/security contexts. This workflow edit does not
silently change repository protection settings.
