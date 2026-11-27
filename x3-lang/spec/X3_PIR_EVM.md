# X3-PIR-06 — compiler-to-EVM exact-action integration (experimental)

This bounded profile extends the existing capability parser and compiler. It emits an offline proposal for the existing X3PolicyExecutor in PR #279, not an authorization, general contract firewall, or deployed capability. The owner must independently review the exact source, compiler, scope, chain, executor, target code, actual calldata, agent, policy epoch, nonce, expiry, and native approval before registering an action.

## Source and lowering

See compiler/tests/fixtures/pir_evm_action.x3 (fictional, expired test fixture). The resource is `evm_call <u64 chain_id> "<40 lowercase hex executor>" "<40 lowercase hex target>" "<8 lowercase hex selector>" "<64 lowercase hex calldata Keccak-256>"`. Subject is a nonzero lowercase 0x-prefixed 20-byte EVM address. The EVM-specific compiler supports only chain 84532 (Base Sepolia), root grant (parent none), zero delegation depth, NoAsset, empty requirements, zero native value, and exact 4..516 byte calldata. NoAsset does not establish absence of ERC20 or other asset side effects; only a reviewed non-asset fixture is tested. The Rust compiler independently verifies Keccak-256 and selector against supplied calldata; no implicit fallback or SHA3 substitution. Ordinary executable source still rejects capability declarations. Existing tool exporter explicitly rejects EVM resource. The existing workspace sha3 dependency is now linked to the compiler; lockfile adds one dependency edge, no package version changes.

`compile_evm_action_source(source, trusted_scope, now, policy_version, nonce, calldata)` yields 352 bytes of static Solidity Action ABI (11 words), Keccak-256 action ID, and policy hash. Policy hash = SHA-256(ASCII X3/PIR/EVM-POLICY/1 + NUL || exact source SHA-256 || canonical grant SHA-256). Action ID = Keccak-256(Keccak-256(ASCII X3ContractActionV1) || action ABI). The scope is independently selected, not authenticated chain state. Version/nonce/time are trusted caller inputs; actual on-chain state must be checked separately. No wallet or RPC. Policy hash is a commitment only; Solidity does not interpret the source and owner can register different actions, so this is owner-mediated exact-action enforcement rather than autonomous policy enforcement.

## Reproduce integration

From x3-lang, with the reviewed PR #279 checkout at exact commit a1d5a5f35cb6687e2114617cdd413cc42386f1c6 and installed Foundry:
```sh
cargo test -p x3-lang-compiler --test evm_capability --locked --offline
FORGE=forge bash scripts/verify_pir06.sh /path/to/pinned/checkout
```
The script builds the Rust exporter, compiles the fixture into action bytes and ID, retrieves the executor source/config from the pinned Git object, and runs Foundry in a disposable directory. It uses vm.chainId, vm.warp and vm.etch to simulate the chain and fixed fixture addresses with constructor immutables; these are not deployed addresses. The Solidity test independently computes action ID, requires it equals Rust, owner installs policy/rule/approval, and the agent executes the approved target call. Direct bypass, replay, changed arguments, wrong sender, modified policy hash, nonzero value, expiry and revocation are tested. No broadcasts or credentials.

## Release hold
Stacked on #285; executor dependency remains separate #279. Existing Foundry lint warnings, owner authorization boundary, proxy/code-dependency risks, token semantic success, arbitrary asset effects, signed action authorization, policy rotation and independent audit remain unresolved. No general EVM source semantics, delegated EVM permissions, production deployment, native value transfers, or cross-chain finality claim. Dedicated CI, stronger adversarial tests, actual testnet deployment approval and real integration audit required. Next: cross-chain policy lowering and verified settlement checks, keeping native finality requirements mandatory.
