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

The EVM fixture compiles generated Solidity with solc 0.8.28 and executes the bytecode on EthereumJS. The Solana fixture executes the actual System, Token, Associated Token and Memo programs in LiteSVM, with the same .x3 declaration. Both run entirely locally.

```sh
PYTHONPATH=x3-lang python -m pytest -q x3-lang/tests/test_coin_authoring.py x3-lang/tests/test_coin_sale.py
(cd x3-lang/coin/integration/evm && npm ci --ignore-scripts && npm test)
python -m pip install -r x3-lang/coin/integration/svm/requirements.txt
PYTHONPATH=x3-lang python x3-lang/coin/integration/svm/test_litesvm.py -v
```

To assemble an unsigned Solana wire transaction locally, use `coin.svm.assemble_svm(source, observed_genesis_hash=..., payer=..., recent_blockhash=..., mint_rent_lamports=...)` with trusted observations from the same network. The result includes a base64 unsigned transaction and required signer addresses. Cluster identity is a client-side check. The compiler and assembler neither sign nor broadcast.
