# Generated ERC-20 execution test

From this directory run `npm ci --ignore-scripts` and `npm test`. Python 3 is used to compile the shared `.x3` example. Solidity 0.8.28 compiles its emitted self-contained source for the Shanghai EVM. EthereumJS executes the creation and runtime bytecode in memory on two chain IDs. No RPC endpoints or signing keys are used.

The emitter bundles the ERC-20 implementation from `@openzeppelin/contracts@5.4.0` (npm tarball SHA-256 `4e4d51f18935b32f0c877bf666ac37f5e5a98e75f03e7a77eb7569f84701f680`). The flattened source includes `Context.sol`, `IERC20.sol`, `IERC20Metadata.sol`, the `IERC20Errors` interface, and `ERC20.sol`, with their MIT license. Import directives and per-file pragmas are replaced by a single pinned compiler version. The appended `X3IssuedCoin` constructor binds the chain ID, contract address and genesis issuance.

The declaration's contract address is the address predicted for ordinary `CREATE` from the deployer's address and nonce. The fixture uses chain ID 31337 with deployer `0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf` at nonce zero.
