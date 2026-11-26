# X3CCP offline EVM compiler profile (experimental)

This standalone, strict source profile follows the bounded-profile approach of `near_native_vault.py`. It does **not** extend the canonical X3Lang parser, VM or bytecode yet. It generates one exact-action proposal for the existing executor. No signing, broadcast or RPC is performed. A compiler output is not authorization.

Supported source:
```
contract_policy treasury {
 chain_id 84532
 executor 0x1111111111111111111111111111111111111111
 target 0x2222222222222222222222222222222222222222
 agent 0x3333333333333333333333333333333333333333
 calldata 0x12345678
 max_value 10
 expiry 2000
}
```
These are fictional fixture addresses and an expired fixture timestamp. One target, exact calldata bytes, one agent, native value cap in wei, absolute expiry in Unix seconds. Base Sepolia only. Unknown terms, duplicates and unsupported encodings refuse. Only selector plus 0–16 32-byte words; this is not ABI semantic validation. Source SHA-256 commits exact bytes including whitespace; executor action ID uses Ethereum Keccak-256, standard ABI encoding. Numeric JSON fields are decimal strings.

```sh
cd x3-lang/contract-control/compiler
python -m unittest -v test_compile_action
```
Requires Python 3 and OpenSSL providing `dgst -keccak-256` (locally checked with OpenSSL 3.5.5). No SHA3 fallback. Treat the executable and PATH as trusted; missing algorithm fails closed.

Owner tooling must independently pin source, executor code identity, current chain state, nonce, policy epoch and time; recompile and compare every byte before registering an action. The compiler's supplied context is not authenticated. An owner can still register actions outside this profile; the executor does not evaluate the source semantics. Value cap is not a lifetime spending budget. No token semantic guarantees. The Python tests and static digest vector are not Solidity execution, testnet evidence, or permission to deploy. No live funds.

Next: Solidity vector parity and full executor tests; authenticated owner-side source verification; integration with canonical X3Lang parser/compiler and on-chain enforceable policy semantics.
