# X3 Contract Control Protocol (X3CCP)

**Experimental draft, uncompiled at initial submission, not audited, not deployed. No real funds.**

The initial EVM executor uses owner-registered exact action IDs and on-chain target/function rules. Agents cannot register their own actions. The target must explicitly delegate the relevant permission to the executor. See [action v1](spec/action-v1.md) for trust boundaries.

## Local verification

From the repository root:
```sh
cd x3-lang/contract-control/evm
forge build
forge test -vvv
```

Use Foundry with solc 0.8.24. Record exact commit, Foundry version and full outputs; do not promote on compilation alone. The test fixtures are not deployable production contracts. Test coverage is proposed, not a claim of passing execution. The initial environment had no forge/solc and package DNS failed. RED/GREEN has not been observed.

## Release holds

1. Run full tests, add mutation tests and canonical digest vectors (including independent encoder).
2. Bind actual compiler-produced policy commitments and demonstrate policy/compiler parity. An owner-entered hash alone is not X3Lang semantic enforcement.
3. Validate target permission isolation and code identity; proxies, delegatecall, generic ERC20 semantics, daily budgets, economic postconditions, owner recovery, and cross-chain enforcement are out of scope.
4. Independent audit and explicit testnet deployment approval. No mainnet, live funds, automatic deployment, or changes to existing Guard.
