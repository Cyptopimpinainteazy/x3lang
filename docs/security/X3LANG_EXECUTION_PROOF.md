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


## Canonical runtime equivalence: second-stage gate

The mirrored `scripts/proof/x3lang_runtime_equivalence.py` is executed by
**xxxstar only**, because the standalone X3Lang checkout does not contain the
canonical runtime crate `crates/x3-integration`. In the full chain repository
it runs six real Rust suites:

- `source_language`: source compilation and local three-engine outcomes.
- `chain_engine`: actual canonical on-chain VM entry behavior.
- `opcode_differential`: opcode agreement and fail-closed refusal.
- `gas_accounting`: bounded execution at the interpreter boundary.
- `x3lang_trading_rollback`: no partial commit after rejected trades.
- `x3lang_trading_receipt_replay`: signature trust and replay verification.

Run on a machine containing the full chain repository and toolchain:

```bash
python3 scripts/proof/x3lang_runtime_equivalence.py \
  --output-dir /tmp/x3lang-runtime-proof
```

Both proof runners **reject zero-test Cargo successes**. Evidence contains
executed case counts, command exits, test log hashes, source input hashes, and
exact checked-out Git revision. The runtime suite is a new independent CI job
on the `xxxstar` self-hosted X3 runner; until it completes successfully on
the exact PR head, runtime equivalence has **not** been proven.

This is not an independent blockchain deployment proof. In particular,
`execute_on_chain` in a Rust test does not prove a deployed chain executed
the bytecode, and local engine consensus does not prove real EVM/SVM/NEAR
transaction finality, atomic refunds, authentic attestor key custody, or
signed external inclusion evidence.

### Release admission beyond local proof

For a cross-chain settlement release, require independently fetched,
verifiable, finalized source **and** destination chain evidence, transaction
and chain IDs, proofs of amount and asset identity, timeout/refund state,
trusted verifier keys, replay-protected binding to the same X3 execution
receipt, and deployed runtime artifact hash comparison. If any are missing,
the release gate must remain **closed**, irrespective of these local tests.
