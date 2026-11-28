# X3 Contract Control Protocol (X3CCP)

**Experimental, offline-tested EVM prototype. Independent audit and explicit network deployment approval are pending.**

The initial EVM executor uses owner-registered exact action IDs and on-chain target/function rules. Agents cannot register their own actions. The target must explicitly delegate the relevant permission to the executor. See [action v1](spec/action-v1.md) for trust boundaries.

## Local verification

From the repository root:
```sh
cd x3-lang/contract-control/evm
forge build
forge test -vvv
cd ../..
python3 -m unittest discover -s contract-control/compiler -v
cargo fetch --locked
bash scripts/verify_pir06.sh
```

Use Foundry 1.8.3 and solc 0.8.24. The Solidity suite covers the action vector, nonce, revocation, rollback and target constraints. The in-tree compiler integration runs the production Rust parser and exporter in simulation and checks the action ID against Solidity. No credentials or RPC are required.

## Release holds

1. Expand independent mutation coverage and target-specific postcondition tests before network use.
2. The tested Rust compiler commitment is verified in the local EVM fixture. An owner may register other hashes, so reviewed owner-side policy and target validation remain essential.
3. Validate target permission isolation and code identity; proxies, delegatecall, generic ERC20 semantics, daily budgets, economic postconditions, owner recovery, and cross-chain enforcement are out of scope.
4. Independent audit and explicit testnet deployment approval. No mainnet, live funds, automatic deployment, or changes to existing Guard.
