# X3 Shield Base Sepolia — contract-controlled agent vault

**Security stage:** Experimental testnet-only pilot. Not mainnet-ready or audited.

**Purpose:** Close the unrestricted EVM EOA-key bypass for funds that are explicitly moved into this contract. The owner holds a separate custody key and deposits ERC-20 units into an immutable contract vault. A compromised agent holds ONLY a narrowly scoped EIP-712 signing key and cannot spend assets outside the signed owner policy. The arbitrary transaction relayer has no authorization powers.

This is **not EIP-4337**, a wallet plugin, or a proof that the previously executed Tidepool trade used this vault. It is a distinct Solidity custody enforcement mechanism.

## Enforced boundaries

- **Deployment:** Base Sepolia chain ID 84532 or isolated Foundry chain 31337 only. The constructor refuses mainnet chain IDs 1 and 56.
- **Owner:** Immutable deployment account, kept outside the agent runtime. Only owner can deposit, revoke, or recover. Remaining escrow is recoverable only after revocation or expiry.
- **Immutable policy:** One ERC-20 token, its deployed code hash, signer, recipient, maximum per-payment value, cumulative budget, owner expiration, the chosen quote commitment and the X3Lang program commitment.
- **Signed spending:** AgentPayment(nonce,recipient,amount,deadline,quoteHash,programHash) using the canonical EIP-712 domain that binds chain ID and the vault address. Relayers cannot change signed terms. High-S malleability and malformed ECDSA signatures are refused.
- **Atomic payment:** On execution, the vault checks the signature and all policy fields, consumes a fresh nonce, checks the cumulative budget, and performs one ERC-20 transfer. A failure reverts nonce and spent accounting in the same EVM transaction. Reentrancy is guarded even when a malicious local test token attempts to execute another independently authorized payment during the first transfer.
- **Exact ERC-20 accounting:** Owner deposits, agent payouts and owner recoveries require exact pre/post token balance differences, rejecting taxed/deflationary behavior.

## Reproduction on x3star1 or any local Foundry setup

From x3-lang/base-agent-vault run:
1. forge fmt --check
2. forge test --offline --fuzz-runs 256 -vv
3. forge build --offline --sizes

The Foundry tests use an explicitly disposable local PilotToken fixture. This mock token is NOT part of the production vault contract and is not represented as a live Base Sepolia USDC transaction.

## Optional real Base Sepolia ERC-20 fork compatibility (read-only network)

The optional Foundry integration test obtains genuine public state from a **pinned finalized Base Sepolia block 55,557,819**, authenticates the deployed test-USDC token bytecode SHA-256 **db77a21a647b5973bac5728bf9f6977b56933006bf2f0730a14d109d33072c83** at address **0x38d81431B9f76d9699AF7517040D5221D9dEA252**, and uses Foundry's **LOCAL fork-only account impersonation** for the public testnet owner 0x5aF0c1BF95B3A158f6Fc48e3DED35903b2fa0721. The fork tests the actual ERC-20 approve, deposit, exact EIP-712 agent-signed transfer, receipt ledger, nonce replay refusal, and attacker-recipient refusal, against genuine archived token code/storage.

Opt in explicitly using this read-only Base Sepolia RPC and the separate test path:

    X3_BASE_SEPOLIA_FORK_RPC=https://sepolia.base.org forge test --match-path test/X3ShieldBaseFork.t.sol -vv

If X3_BASE_SEPOLIA_FORK_RPC is absent, the fork test is **SKIPPED** (not counted as proof). The standard offline suite needs no network. This is a **local EVM fork simulation on actual token bytecode**, not a signed public transaction, verified public-vault code, user wallet authorization or live Base Sepolia custody release.

See evidence/base-vault-local/finalized-base-sepolia-fork-20261107.json for the captured opt-in result and its local log hash.

## Read-only, two-provider token attestation (does not authorize deployment)

Before any public Base Sepolia pilot, rerun:

    python3 scripts/hackathon/base_vault_token_preflight.py --output /tmp/x3-token-audit-new-unique.json

Run from repository root. This checks two independent public RPCs, chain 84532, the canonical finalized block at height 55,557,819, unchanged test-USDC runtime code at that block and each provider's latest finalized head, and consistent canonical block hash at their shared finalized height. Standard EIP-1967 implementation/beacon slots must be zero at both checked states. Any RPC/archive failure hard-denies the observation. This provides *token bytecode integrity evidence*, not quote authenticity, owner key management, a smart-contract audit, or permission to transact.

Updated stricter parser live replay: `evidence/base-vault-local/base-sepolia-token-two-rpc-attestation-v2-20261107.json` (SHA-256 `ad9ba58aaea0fb4ee3de31ef76951c286e54cba31d64a6b3bab2e298b1b56fa5`). This reread additionally rejects malformed canonical hash values and RPC responses whose block number does not match the specifically requested checkpoint. The original snapshot remains unchanged.

Historical read-only evidence: `evidence/base-vault-local/base-sepolia-token-two-rpc-preflight-20261107.json` (SHA-256 `1d18dd3df98a743eacb505710c750e2c3902c3233b6e2dd6aed9223a456a218a`). Internal audit limitations: `docs/security/BASE_VAULT_INTERNAL_AUDIT_20261107.md`.

## Important security limitations

The quoteHash is an owner-pinned commitment, not **cryptographic verification of the Tidepool maker signature inside the vault**. Do not reuse the expired historical Tidepool quote from the November 7 real swap. The next pilot requires a fresh maker-signed quote, real deployment verification, policy registration and a separate owner-approved funding transaction.

A proxy token may change implementation behavior without changing the proxy's deployed bytecode hash. Token code hash checks do not solve upgrade-authority risk. An unrestricted owner key can still revoke and recover funds; compromise of that key defeats the isolation. An agent that also holds a funded Base EOA key can bypass custody for those **other assets still held in that EOA**. Move only approved small test token balances into the reviewed vault; do not pretend other EOA funds are protected.

Revocation is effective when the revocation transaction executes; it cannot undo prior successful payments or guarantee priority over a payment submitted earlier in the block. The EVM vault is not yet connected to the NEAR Guard, Tidepool settlement pipeline or the native BNB↔NEAR bridge.

## Public Base Sepolia deployment gate — NOT APPROVED

Before deployment, an independent security reviewer must inspect exact code and the selected test token's implementation, verify both RPC providers and Base Sepolia chain ID, and reproduce the artifact. The owner must explicitly approve the deployment wallet, nonce/session signer, payout recipient, quote/program hashes, max payment, total funding budget, expiry and gas limit.

No mainnet assets, no public-chain transactions, no wallet private keys, and no contract deployment are authorized by this PR. The separate successful Base Sepolia to NEAR testnet swap remains recorded under docs/hackathon/FIRST_SWAP_JUDGE_WALKTHROUGH.md with its original proof classification.
