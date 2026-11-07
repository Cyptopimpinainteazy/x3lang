# Base Sepolia → NEAR Testnet guarded admission (read-only candidate)

This is **not** an execution proof. No owner signing, transfer, deposit, or NEAR Guard deployment is performed by the admission script.

`base_sepolia_guarded_admission.py` is deliberately unable to sign or broadcast. It verifies a *fresh*, genuine Tidepool maker-signed live quote for **exactly 1 test USDC** from the dedicated Base Sepolia test wallet `0x5aF0c1BF95B3A158f6Fc48e3DED35903b2fa0721` to `x3lang.testnet`, including bound refund address, signed deposit EVM address, minimum output, and deadline.

It then verifies NEAR testnet chain/genesis, exact deployed bytecode hash of a **separate, reviewed testnet Guard**, and `get_policy` on the finalized chain for matching owner, unique nonce, `.x3` program hash, current non-revocation, exact signed quote SHA-256, amount, refund address, maximum slippage, minimum payout and expiry. Both Base Sepolia public RPC providers must agree about token and gas balances, chain 84532 and the designated test USDC contract code hash.

Even when all conditions pass, it **prints only an unsigned ERC-20 calldata plan**. A separate wallet signing program must re-check these conditions at execution and bind the transaction nonce and fees, otherwise the approved result is only advisory and the wallet can bypass the Guard. Do not claim that `get_policy` prevents an external signer from disregarding X3. On-chain approval is not proof of settlement.

Example usage (expects NEW owner-approved, nonexpired quote and an actually deployed/reviewed Guard):

```sh
python3 scripts/hackathon/base_sepolia_guarded_admission.py \
 --quote-file "$HOME/.config/x3lang/base-sepolia-test-wallet/live_tidepool_quote_unsigned_funds.json" \
 --guard-account <deployed-reviewed-testnet-guard-account> \
 --guard-wasm-sha256 <independently-confirmed-deployed-guard-code-sha256> \
 --nonce <registered-owner-unique-nonce> \
 --program-sha256 <sha256-of-approved-x3-source>
```

The historical stored live quote will fail freshness checks by design. Never reuse its deposit address. The testnet-only Tidepool operator controls the maker/relayer and is not an official NEAR 1Click testnet. Mainnet 1Click uses separate authority, tokens and infrastructure. Never send real assets to a testnet quote.

**Deployment blocker:** the proposed guard contract account is not on NEAR testnet yet; source audit, source-to-WASM reproduction, exact-head CI, and a deliberately authorized deployment with an independently controlled signing account are required. No usable `x3lang.testnet` signing credentials have been established on x3star1. Private GitHub `xxxstar` mirror access is also a separate CI gate. These are not to be silently bypassed.
