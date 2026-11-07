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

The read-only Base Sepolia observation requires **two independent, agreeing RPCs**: the official `https://sepolia.base.org` and `https://base-sepolia-rpc.publicnode.com`. The previously configured dRPC endpoint returned HTTP 403 from x3star1, so it was replaced without removing the second-provider hard gate. Both replacement RPCs independently confirmed chain 84532, the exact test-USDC contract code hash, 100 test USDC, and 0.001 test ETH on November 7, 2026 UTC. Provider disagreements or outages continue to block admission.

The historical stored live quote will fail freshness checks by design. Never reuse its deposit address. The testnet-only Tidepool operator controls the maker/relayer and is not an official NEAR 1Click testnet. Mainnet 1Click uses separate authority, tokens and infrastructure. Never send real assets to a testnet quote.

**Deployment blocker:** the proposed guard contract account is not on NEAR testnet yet; source audit, source-to-WASM reproduction, exact-head CI, and a deliberately authorized deployment with an independently controlled signing account are required. No usable `x3lang.testnet` signing credentials have been established on x3star1. Private GitHub `xxxstar` mirror access is also a separate CI gate. These are not to be silently bypassed.

## Separate testnet-only signer implementation (disabled until owner approval)

`base_guarded_broadcast.py` uses the same exact Guard+chain checks in **the same
process** as the Foundry encrypted-keystore signing operation, checks the
keystore address, caps gas/fees, takes an exclusive process lock, and stores a
quote-hash attempt record **before** attempting network broadcast. It has no
Guard-bypass argument and requires `--approve-exact-quote-sha256` matching the
**fresh, cryptographically verified** live quote exactly. Default behavior is
preview only. A transaction can only be broadcast with explicit `--execute`
after all conditions pass.

The wallet keystore and password file remain owner-readable only on x3star1.
No private key is passed to a subprocess argument or entered in chat. This
review candidate is **not** production approved, has not signed or broadcast a
trade, and cannot operate until a reviewed on-chain testnet Guard exists and
an owner has registered/authorized an exact policy. A prior quote fixture or a
fake Guard must fail closed. We deliberately do not create any new quote here.

Even integrated signing cannot prevent the wallet owner from using another
program outside X3; claims are limited to the **verified X3 execution path**.
The first trade still requires a separate explicit owner review of the exact
quote/hash, contract deployment, fee budget and expiry before `--execute`.

## Actual `.x3` source commitment

`x3-lang/examples/base_sepolia_to_near_testnet_1usdc.x3` is parsed and
typechecked through the real X3Lang Python frontend and lowered to a STRICT
Base Sepolia testnet policy by `base_sepolia_x3_intent.py`. The signed owner
mandate must pin the SHA-256 of those **exact source bytes**; the read-only and
execution admission tool refuse another SHA even if manually supplied.

This is a typed Python frontend/policy projection, **not** evidence that the
canonical Rust X3VM compiler emitted bytecode or that NEAR Guard actually
executes X3VM bytecode. Base Sepolia is deliberately absent from the production
1Click chain mapping: a Base Sepolia quote is testnet-only, and mainnet route
resolution fails closed rather than silently translating it as Ethereum.
