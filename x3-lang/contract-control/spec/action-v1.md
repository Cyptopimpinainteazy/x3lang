# X3CCP action v1 — experimental, not deployed

The executor mediates calls from a named agent to a contract that voluntarily recognizes the executor as its caller. No delegatecall, arbitrary privileged third-party control, or cross-chain atomicity is provided. The owner is trusted and immutable; use a properly secured owner account. This version is an exact-action authorization transport, not a .x3 compiler or a general policy interpreter.

## Canonical action

Solidity tuple (all static):
- uint256 chainId
- address executor
- address target
- bytes4 selector
- bytes32 calldataHash (keccak256 of complete calldata including selector)
- uint256 value (native currency, supplied by executing agent)
- address agent
- bytes32 policyHash (opaque owner-selected commitment, nonzero)
- uint256 policyVersion
- uint256 nonce (globally single use per executor)
- uint256 expiry (Unix seconds; exclusive)

Action ID = keccak256(abi.encode(keccak256("X3ContractActionV1"), action)).
The tuple uses standard ABI encoding, not packed encoding. This is not an EIP-712 signature digest. The owner registers exact action IDs on-chain; no signature recovery or oracle is required. The policy hash is not proof of compiler semantics. Compiler equivalence is a later gate.

Owner installs a policy commitment and target/selector rules pinned to current runtime code hash. Every policy/rule/pause change increments the version, invalidating outstanding approvals. Owner may revoke an individual action. Default deny; only exact agent may execute, with matching calldata/value, current version, nonexpired approval, and unused nonce. Mark nonce and delete approval before CALL, reject reentrancy; revert on call failure so state changes roll back. Successful CALL means EVM-level success only, not semantic ERC20 success or economic profit. No generic token return-value handling. No deposit or withdrawal feature; forced native balance is not recoverable in v1.

Runtime code hash does not detect proxy implementation changes, mutable dependencies, or changes in target behavior. Only use reviewed non-proxy fixtures in the initial gate. An owner or independent privileged key can bypass this boundary if the target grants them direct access. Deployment, real funds, and promotion are forbidden pending exact-head test evidence, integration and independent review.
