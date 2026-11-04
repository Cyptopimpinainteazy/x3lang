# X3Lang execution proof / two-repository mirror

This gate executes **real, existing Rust tests** against the exact checked-out
revision. It does not claim on-chain settlement, externally verified trading,
EVM/SVM execution, a formal proof, or production readiness.

Run:

```bash
python3 scripts/proof/x3lang_execution_proof.py --output-dir /tmp/x3lang-proof
python3 scripts/proof/check_x3lang_mirror.py --peer-repo /path/to/other/repo
```

A successful run requires four Rust test binaries (compiler→bytecode→VM source
round trip, bytecode version admission, nonce replay and signed trading receipts)
to exit zero. The resulting manifest binds the Git revision, test source hashes,
toolchain versions, actual commands, and log hashes. Failure results remain
marked FAILED. No mock chain responses are introduced by this runner.

The separate `xxxstar` workflow `x3lang-control-flow-parity.yml` tests three
**local execution engines** under `x3-x3-integration`. It is a separate gate,
not equivalent to a funded transaction on three chains.

## Mirroring policy

Both repositories should carry identical copies of the two proof scripts,
their four test sources, and this documentation. PRs for the same feature should
be opened separately against `x3lang/main` and `xxxstar/master` using the
same source bytes. Run the comparison against both clean feature checkouts
before merging. Never blindly synchronize the entire repositories: they have
independent roots, dependency graphs, and outstanding branches.

The comparison script is **read-only**. It detects divergence, but it cannot
push to GitHub and cannot guarantee that future unrelated work is mirrored.
For continuing synchronization, configure trusted automation with separately
authorized write credentials, explicit path allowlists, protected-branch PRs,
and conflict rejection. Do not put a write-capable token into untrusted PR jobs.

## Remaining production proof obligations

- Compiler/VM deterministic execution and gas/state equivalence across
  canonical runtime and independent targets.
- Bound, signed, replay-checked receipts from the actual host execution.
- Authentic EVM and Solana transaction inclusion/finality evidence,
  with independently verified NEAR settlement/refund linkage.
- Exact deployed artifact hash and independent finalized block proof.
- Dual-repository per-commit parity checks and reviewed PR merges.

None of these obligations is discharged merely because this local test gate
passes.
