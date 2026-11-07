# X3 Shield Base ERC-20 Vault: security assertions

**Scope:** A narrowly controlled, testnet-only single-token contract vault. An agent with only the approved EIP-712 signing key may not arbitrarily spend escrow.

| Invariant | Contract control | Local verification |
| --- | --- | --- |
| Agent cannot directly move escrow | No ERC-20 allowance given to agent; only owner deposit/recover | Direct token transferFrom attack rejected |
| Every payout requires agent authorization | EIP-712 canonical digest, secp256k1 ecrecover, EIP-2 low-S | Forged signer, corrupted message, bad v, high-S, truncated signature |
| Same authorization cannot cross domain | Chain ID + contract address domain separator | Replay on another vault/chain rejected |
| Quote/program, recipient and amount are bound | Constructor policy and typed payment | Signed redirect, changed amount, changed quote/policy rejected |
| Total payment amount is limited | Per-payment cap and total spent ledger | Payment cap, cumulative cap, fuzzed over-cap values |
| Every nonce is one-use | Storage records nonce before external token call | Double relayer, replay, fuzzed alternate nonce |
| Failed transfer cannot consume nonce or funds | EVM state rollback, exact pre/post token accounting | Taxed-token deposit and payment rollback |
| Malicious token reentry cannot induce another valid payment | Entered-state reentrancy lock and nonce/budget mutation before token interaction | Reentry fixture attempts a second correctly signed nonce during first ERC-20 transfer; refused without undoing first payment |
| Owner can terminate session | Only owner revoke; recovery after revoke/expiry | Unauthorized revoke/recovery, expiry, owner recover |
| Underlying token code changes fail closed | Constructor pins exact Base Sepolia test-USDC address/bytes and runtime codehash is rechecked before transfers | Mutated contract bytecode test asserts exact `TOKEN_CODE_CHANGED` denial |
| Mainnet deployment is prohibited | Chain ID 84532/31337 constructor guard | Tests reject Ethereum/BSC mainnet |
| Arbitrary contract execution is absent | No arbitrary call or delegatecall method | Minimal public execution surface |

**Real-token compatibility extension:** A separately opt-in Foundry fork at Base Sepolia block 55,557,819 verified the **real** token contract and immutable bytecode pin, then executed an exact local-fork deposit and agent-authorized transfer including replay/redirect refusals. Because fork transactions are local, this is NOT a live Base custody testnet deployment or public transaction proof.

**Excluded assurances:** No third-party audit or formal verification, no deployed Base Sepolia vault, no custody key lifecycle management, no verified maker signature on Base inside the vault, no ERC-4337 compliance, no multicall/upgrade support, no public chain adversarial receipts and no permissionless cryptographic BSC-NEAR atomicity. Existing Tidepool testnet trade proves a different authorized execution path.

The local token fixture exists **only in Foundry tests**. The reviewed vault source contains no alternate/unsafe production backend and no token minting or storage bypass.

**Internal, not independent, audit:** see `docs/security/BASE_VAULT_INTERNAL_AUDIT_20261107.md` for the resolved wrong-token vulnerability and still-open quote authenticity, token upgrade, custody key, and token DoS risks. Public Base Sepolia deployments require both pinned address and exact runtime SHA-256.

**Owner key is the trusted administrative root.** Do not expose it to an AI agent. Never move public funds or change repository visibility without explicit owner consent. Confirm session key isolation, owner revoke behavior, storage/gas overhead, token proxy upgrade privileges, and exact deployed runtime bytecode before any live public-testnet transaction.
