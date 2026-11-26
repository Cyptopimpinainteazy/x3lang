# Generated ERC-20 execution test

From this directory run `npm ci --ignore-scripts` and `npm test`. Python 3 is used to compile the shared `.x3` example. Solidity 0.8.28 compiles its emitted self-contained source for the Shanghai EVM. The issuance test executes creation and runtime bytecode in memory on two chain IDs. The paid-delivery tests use disposable local transaction signatures. All tests run offline.

The emitter bundles the ERC-20 implementation from `@openzeppelin/contracts@5.4.0` (npm tarball SHA-256 `4e4d51f18935b32f0c877bf666ac37f5e5a98e75f03e7a77eb7569f84701f680`). The flattened source includes `Context.sol`, `IERC20.sol`, `IERC20Metadata.sol`, the `IERC20Errors` interface, and `ERC20.sol`, with their MIT license. Import directives and per-file pragmas are replaced by a single pinned compiler version. The appended `X3IssuedCoin` constructor binds the chain ID, contract address and genesis issuance.

The declaration's contract address is the address predicted for ordinary `CREATE` from the deployer's address and nonce. The fixture uses chain ID 31337 with deployer `0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf` at nonce zero.

## Paid delivery and recovery

`sale.test.mjs` compiles the test declaration with 1,000,000 authorized coins, 100,000 assigned to the local EVM and 900,000 assigned to the unsigned SVM plan. EthereumJS executes the EVM issuance and funds the sale contract with its 100,000 coins. The ten 100,000-coin sales quotas are local accounting inputs. The test only instantiates the EVM allocation.

The steward authorizes an immutable order on `X3CoinSale`. The named buyer submits the order key and exactly 0.01 units of local test currency. The contract transfers 100 coins from its own inventory, forwards payment to the configured treasury and consumes the order in one transaction. The sale contract assumes a reviewed deployment of the ordinary generated ERC-20 and a trusted authorizing steward. It has no general inventory withdrawal or price discovery.

The test signs transactions with deterministic disposable identities inside EthereumJS. It builds blocks, records receipt logs, checks the buyer and treasury balances, and starts a fresh Python process to discover the transfer after a simulated application interruption. The observer reads real execution data through the test's RPC transcript. The fixture designates a local block as finalized. Public validator consensus, live RPC availability and public testnet issuance require independent evidence.

### Prepare unsigned Sepolia transactions

From this directory run `node sepolia-setup.mjs path/to/coin.x3 path/to/public-setup.json` after installing the pinned dependencies. The .x3 must target chain ID 11155111 and declare the ordinary CREATE address of the steward at its chosen pending nonce. The setup file contains these public fields.

| Field | Value |
| --- | --- |
| `steward`, `treasury`, `buyer` | Public EVM addresses for the deployment wallet, payment recipient and buyer |
| `steward_nonce` | Wallet's intended next nonce, as a decimal string |
| `prefund`, `amount` | Initial sale inventory and purchase size, in integer base-unit strings |
| `order_id`, `expires_at` | A unique local reference and future Unix timestamp in seconds |
| `payment_wei` | Exact native payment in wei, as a decimal string |

The output includes three unsigned wallet requests, a reservation request and a partial delivery policy. The first two wallet requests use consecutive ordinary CREATE nonces for the token and sale deployments. The prefunding call transfers existing tokens from the declared EVM genesis recipient to the sale contract. When the same wallet holds both roles the emitted third nonce follows the deployments. If the wallet nonce changes, regenerate the declaration and all artifacts for the new address.

The helper executes both constructors in an offline Sepolia-configured VM to calculate the runtime code hashes with the actual immutable values. A public run still needs the real wallet owners and deployment receipts. Add the verified sale deployment block number as `from_block` before preparing an order, and match on-chain code, contract configuration and funded inventory before using the route. A hash from this offline package describes expected code.
