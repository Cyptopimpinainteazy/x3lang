# X3Lang security testing

`mutation_runner.py` mutates real Python mandate-admission logic in isolated copies, executes the existing pytest suite, and emits JSON/Markdown per-mutant evidence.

Run from repo root with a tested virtual environment:

```bash
PYTHONDONTWRITEBYTECODE=1 .venv/bin/python scripts/security/mutation_runner.py \
  --output-dir /tmp/x3-shield-mutation-results
```

The initial curated set contains 11 permissive authorization-bypass mutants and 1 exact-expiry boundary mutant. The baseline contains 45 existing mandate tests plus 14 boundary tests. No source file is modified in the real checkout; tests run against disposable copies. Any survivor, invalid mutation, timeout, red baseline or unknown mutation ID fails the gate.

See `docs/security/MUTATION_TESTING.md` for the current methodology and exact limitations. This is a Python mandate gate only: Rust Guard, compiler/VM and cryptographic proof mutations require separate suites and cannot be inferred from this score.
