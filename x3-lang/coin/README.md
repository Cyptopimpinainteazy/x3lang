# Author a coin from .x3

An issuer or agent supplies a structured request. The X3Lang Python coin compiler writes a typed .x3 declaration and compiles that declaration into a standalone Solidity ERC-20 source and an unsigned classic SPL issuance plan. The Rust trading-bytecode compiler does not implement the coin declaration.

From the repository root, using Python 3:

```sh
export PYTHONPATH=x3-lang
python -m coin author x3-lang/coin/examples/dual_vm.json > demo.x3
python -m coin build demo.x3 > bundle.json
python -m coin sale x3-lang/coin/examples/ten_chain_sale.json
```

## Issuance

The example issues 600 whole tokens on a local EVM and 400 on a separate local SVM at six decimals. Use exact decimal strings for all base-unit quantities. The sum of the two positive allocations must equal the cap, and the cap must fit an unsigned 64-bit integer. The compiler binds the EVM chain ID and contract address, Solana genesis hash and mint, recipients, symbol, name, precision and supply terms into one declaration hash. A separate hash identifies the exact input source. Fixture keys and addresses are disposable test identities.

On EVM the emitted ERC-20 constructor checks the chain ID and its ordinary CREATE address before minting its fixed allocation. The allocation goes to the declared holder. The contract has no later issuer or administrative mint. On Solana the assembler creates a classic SPL mint, credits the receiver's associated token account, revokes mint authority and includes the declaration hash in one transaction. The mint has no freeze authority. The standard mint account carries supply and precision. The name and symbol are manifest metadata. Holders can burn SPL balances, lowering outstanding supply.

The two genesis transactions are independent. Treat issuance as incomplete if either fails. Identify official deployments by the network, address and approved declaration hash. Verify the actual finalized EVM code and constructor issuance, and the successful Solana transaction, original program, recipient, mint and revoked authorities before listing either deployment. A matching ticker or memo alone cannot establish that provenance.

The fixed allocations are separate local inventories. The generated ERC-20 and revoked SPL mint cannot mint extra tokens to refill another network. New network and bridge support require a different versioned issuance policy or transfers from funded inventory. There is no mint-and-burn bridge or all-network transaction coordinator in this compiler.

## Shared inventory sale model

The separate sale command previews equal participation against supplied balances. Its example starts with ten treasuries holding 100,000 tokens each. For a 100-token buy routed through X3, Ethereum would deliver 100 and each treasury would account for 10 sold. The nine remote treasuries would owe Ethereum 90 in total. The buyer would have 100, the treasury system would have 999,900, and total outstanding supply would still be one million. At this point the physical Ethereum treasury would have 99,900, regardless of its receivable. The available destination balance is a view of spendable inventory and must never be added to the genesis supply.

The input chain names are scenario labels. The quote checks quotas and destination float and rejects a split that would require fractional base units. It assumes a single agreed token price; cash collection, per-venue price discovery, fees, signing, reservation, settlement and finality are outside this command. Source quotas are token-denominated. Net opposite-direction fills before settlement and verify settlement evidence before reducing a receivable. Independent DEX fills do not enter this accounting automatically.

## Verification

### Reproduce a local inventory hold

The reservation book stores the steward's opening balances in SQLite. The example uses a synthetic asset identifier and 1,000,000 sample coins with six decimals, divided equally across ten networks. The opening balance on each network determines its selling quota and its available delivery balance. These are two views of the same inventory.

From the repository root, this example reserves 100 coins for one buyer. Each command starts a separate process using the same database file.

```sh
export PYTHONPATH=x3-lang
example=x3-lang/coin/examples/ten_chain_inventory.json
work=$(mktemp -d)
db="$work/inventory.sqlite"
asset_id=$(python -c 'import json, sys; print(json.load(open(sys.argv[1]))["asset_id"])' "$example")
python -m coin inventory-init --db "$db" "$example"
python - "$asset_id" > "$work/order.json" <<'PY'
import json
import sys
import time

print(json.dumps({
    "asset_id": sys.argv[1], "order_id": "sale-100", "buyer": "buyer-01",
    "destination": "ethereum", "amount": "100000000", "expires_at": int(time.time()) + 600,
}))
PY
python -m coin reserve --db "$db" "$work/order.json"
python -m coin reservation --db "$db" "$asset_id" sale-100
python -m coin inventory --db "$db" "$asset_id"
python -m coin release --db "$db" "$asset_id" sale-100
```

While the hold is active, all ten selling quotas have 99,990 coins available. Ethereum has 99,900 coins available for another local delivery. Each other delivery balance still has 100,000. The buyer has received zero coins. Both release and expiry free the reserved capacity once. Retrying the same order ID with its original terms returns the existing status. Reusing it with different terms fails. The fixture's asset identifier labels this example independently of the dual-VM issuance example.

This book coordinates processes sharing the same durable database. Independent copies can each promise the same inventory. The operator supplies the opening balances and buyer reference. An order that proceeds to a paid Ethereum delivery requires the buyer's EVM wallet address as that reference.

### Deliver a coin against a single Ethereum payment

`X3CoinSale.sol` sells a prefunded supply of one generated ERC-20. A steward authorizes a single-use order for a specific buyer, token amount, total payment in wei and expiry. The buyer pays the exact price in one transaction. A successful purchase transfers the ERC-20 to that wallet and the native payment to the named treasury. Either transfer failing reverts the purchase. The nine other treasury shares are accounting entries in the inventory book. This contract sends payment to one treasury.

For a local rehearsal, run the EVM integration suite shown below. It exercises the compiled Solidity in an in-memory EVM. The CLI can also prepare and inspect orders for local chain ID 31337. Public network use requires a separately deployed, funded Ethereum Sepolia setup and two independently operated HTTPS RPC services. This repository includes local execution tests. A public purchase requires a matching finalized transaction.

The observer currently recognizes a direct wallet transaction in which the reserved buyer calls the sale contract. A purchase routed through a smart-account intermediary or a batch can execute on chain without matching that evidence layout. Use a direct buyer wallet for this rehearsal.

The [offline Sepolia setup helper](integration/evm/README.md) turns public deployment details and a Sepolia-specific declaration into unsigned creation and prefunding calls. It computes expected deployed code hashes using the real constructor arguments. The sale contract has no withdrawal method for unused prefunding. For a one-purchase public rehearsal, prefund only the 100 coins needed for that order.

For an operator-managed Sepolia run:

1. Use a dedicated asset book bearing the actual coin declaration hash as its `asset_id` (64 lowercase hex digits). Configure the treasury allocation from reconciled inventory. The million-coin fixture above has a synthetic asset ID. Deploy the declared ERC-20 at its pinned address and deploy `X3CoinSale` from the steward wallet with the token address, payment treasury and expected declaration hash. Fund the sale address with the ERC-20 to cover delivery. The buyer needs enough Sepolia ETH for the quoted payment and gas.
2. Reserve the order with destination `ethereum`, the buyer's EVM address, amount in token base units and an absolute future Unix expiry in seconds. The order used below must still be reserved. Use a fresh order if the inventory-only example was released.
3. Supply a reviewed JSON policy with exactly the following fields. The route is pinned on the first preparation in an asset book. The price belongs to the individual order.

| Field | Value |
| --- | --- |
| `destination` | `"ethereum"`, matching the reservation |
| `chain_id` | Integer `11155111` for Sepolia |
| `genesis_hash` | `0x25a5cc106eea7138acab33231d7160d69cb777ee0c2c553fcddf5138993e6dd9` for Sepolia |
| `sale_contract`, `token_contract`, `treasury` | Sale address, declared ERC-20 address and native payment recipient, respectively |
| `sale_code_hash`, `token_code_hash` | `0x`-prefixed Keccak-256 of each reviewed deployed runtime's raw bytes, including constructor-set immutables |
| `from_block` | Integer deployment block for the sale contract from which to search for its events |
| `payment_wei` | Positive decimal string for the entire order, in wei |

```sh
python -m coin sale-prepare --db "$db" "$asset_id" "$order_id" policy.json
python -m coin sale-status --db "$db" "$asset_id" "$order_id"
```

4. Review the returned calldata, destination, chain, value, buyer and expiry. The steward wallet submits the zero-value `wallet_calls.authorize` call. After authorization succeeds, the named buyer wallet submits `wallet_calls.buy` with the exact displayed value. The commands above only prepare the calls.
5. Put two operator-selected HTTPS RPC URLs into `SEP_RPC_A` and `SEP_RPC_B` in your environment. Use URLs without embedded userinfo, query strings or fragments. Run confirmation whenever evidence is needed, including after a restart. Use archival endpoints capable of serving the requested code, logs, block information and transaction evidence. The public CLI permits remote reads against Sepolia only.

```sh
python -m coin sale-confirm --db "$db" "$asset_id" "$order_id" --rpc-env SEP_RPC_A --rpc-env SEP_RPC_B
python -m coin sale-status --db "$db" "$asset_id" "$order_id"
```

`awaiting_confirmation` keeps the whole destination amount and equal selling quotas reserved. The first preparation protects the hold even if the wallet calls expire, a provider goes offline, or competing observations leave the outcome ambiguous. Releasing or waiting out an ordinary reservation remains available before preparation. Prepared orders have no automatic release path. After the same finalized purchase is independently observed, `delivered` records evidence and creates one token-denominated obligation per remote treasury. Confirmation retries reuse that record. These obligations still need a separate repayment process. Provider agreement relies on the selected RPC operators and their finalized state views.

Existing reservation files gain the delivery tables and protections when reopened with the updated code. Prepared and delivered rows continue to count against inventory for older readers. Use the updated `sale-status` view to distinguish their lifecycle states. Keep each book on a single durable SQLite file through migration and restart.

### Run the fixtures

The EVM fixture compiles generated Solidity with solc 0.8.28 and executes the bytecode on EthereumJS. The Solana fixture executes the actual System, Token, Associated Token and Memo programs in LiteSVM, with the same .x3 declaration. Both run entirely locally.

```sh
PYTHONPATH=x3-lang python -m pytest -q x3-lang/tests/test_coin_authoring.py x3-lang/tests/test_coin_sale.py
PYTHONPATH=x3-lang python -m pytest -q x3-lang/tests/test_coin_reservations.py x3-lang/tests/test_coin_reservation_cli.py
PYTHONPATH=x3-lang python -m pytest -q x3-lang/tests/test_coin_delivery_cli.py
(cd x3-lang/coin/integration/evm && npm ci --ignore-scripts && npm test)
python -m pip install -r x3-lang/coin/integration/svm/requirements.txt
PYTHONPATH=x3-lang python x3-lang/coin/integration/svm/test_litesvm.py -v
```

To assemble an unsigned Solana wire transaction locally, use `coin.svm.assemble_svm(source, observed_genesis_hash=..., payer=..., recent_blockhash=..., mint_rent_lamports=...)` with trusted observations from the same network. The result includes a base64 unsigned transaction and required signer addresses. Cluster identity is a client-side check. The compiler and assembler neither sign nor broadcast.
