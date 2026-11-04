# Contributing to X3Lang

Contributions are welcome for language correctness, reproducible test coverage, documentation, security hardening and NEAR integrations. This is experimental financial infrastructure; claims must be narrower than the supporting evidence.

## Understand the repository

- `x3-lang/`: Python DSL, Rust compiler and VM, NEAR adapter, Guard contract and tests.
- `crates/x3bc-core/`: bytecode core used by compiler/VM integration.
- `PRIOR_WORK.md`: historical baseline and NEAR hackathon prior-work disclosure.
- [`xxxstar`](https://github.com/Cyptopimpinainteazy/xxxstar): parent X3 chain. The two repositories have different default branches and workspaces. Do not blindly copy commits between them.

Start from an appropriate current branch, create a focused feature branch, and review overlapping open PRs before changing shared X3/NEAR code. Use PRs rather than pushing directly to protected branches.

## Run the checks

```bash
python3 -m pip install -r x3-lang/requirements.txt pytest
PYTHONPATH=x3-lang python3 -m pytest x3-lang/tests -q
(cd x3-lang && cargo test -p x3-lang-vm --locked --quiet)
(cd x3-lang && cargo fmt --all -- --check)
(cd x3-lang && cargo clippy -p x3-lang-vm --all-targets --locked -- -D warnings)
(cd x3-lang/near/x3-guard && cargo test --locked --quiet)
node --test x3-lang/near/demo/test/*.test.mjs
```

The Rust workspace under `x3-lang/` and the NEAR contract workspace are distinct. Do not assume a root-level `cargo test` covers either. Run focused tests for affected modules and the full relevant CI gates.

## Pull request checklist

- Describe the problem, trust boundary, scope and backward-compatibility impact.
- Cite the exact file paths changed, base/head SHAs, test commands and outcomes.
- Add meaningful happy-path **and adversarial/failure-path** tests.
- Record unresolved risks and any still-unverified live-chain conditions.
- Use `.x3` examples that match the existing grammar. Classify new examples in the surface-drift test when required.
- Preserve compiler semantics, bytecode versions, gas accounting, transactional rollback and receipt integrity.
- Do not add secrets, endpoints with credentials, transaction signing keys or funded example wallets.
- Do not describe local fixtures, simulated chain behavior or dry quotes as a real settled transfer.

Documentation-only changes must still be checked for dead links, inaccurate product claims, and conflicts with the prior-work disclosure. Match the established code style; avoid drive-by reformatting.

## Security and deployment

Report vulnerabilities privately using [SECURITY.md](SECURITY.md). Deployment, funds movement, changing wallet authority, publication of release tags and merges into release branches require explicit maintainer authorization and independent evidence.
