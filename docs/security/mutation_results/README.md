# Locally generated mutation evidence

Mutation reports are generated **per run** and deliberately ignored by Git. Do not treat an old local report as current test evidence or on-chain proof.

From the X3Lang repository root, with the required project virtual environment:

```bash
PYTHONDONTWRITEBYTECODE=1 .venv/bin/python scripts/security/mutation_runner.py \
  --output-dir /tmp/x3-shield-mutation-results
```

The report contains `mutation_results.json` and `MUTATION_RESULTS.md`, each tied to the git HEAD, actual subprocess output, mutant source hash and test counts. The gate runs **12 curated Python security mutants**; all 12 were killed in the validated local initial run at source HEAD `eff72ac636283950714eafd06e18701238058a47` (59 baseline tests). The exact-head CI result for a proposed PR remains a separate requirement.

Any surviving mutant, baseline failure, invalid mutation or timeout blocks the gate. These results do not establish Rust Guard, compiler, VM, light-client, cross-chain settlement or public testnet security.

See [MUTATION_TESTING.md](../MUTATION_TESTING.md).
