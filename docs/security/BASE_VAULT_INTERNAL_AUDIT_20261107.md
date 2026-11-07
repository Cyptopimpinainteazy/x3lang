# X3 Shield Base Sepolia ERC-20 vault — internal security review

**Review type:** model-assisted / checklist-based internal source audit, supplemented by real Foundry tests and read-only public RPC observations. **NOT an independent human audit, formal verification, production custody certification, or deployed smart-contract assessment.**

**Target:** `x3-lang/base-agent-vault/src/X3ShieldBaseVault.sol` (Solidity 0.8.24, local Foundry 31337 and Base Sepolia 84532 only). Original draft PR #138 head `f749d0656c81a54e2324f753be12f8131836c371`. The fixes documented below remain on this PR branch until exact-head CI and reviewer approval.

**Checklists consulted:** EVM general; precision/math; ERC-20; EIP-712/signatures; access control; DoS/griefing; OP-stack chain-specific. Review covers constructor, EIP-712 digest and ECDSA recovery, immutable owner/token/agent/recipient, deposit/execute/revoke/recover, nonce/budget, token balance and exact-credit checks, reentrancy and error propagation.

## [M-01] Arbitrary-token substitution on Base Sepolia (FIXED IN THIS PR)

**Severity**: Medium

**Category**: ERC-20 / access-control

**Location**: `X3ShieldBaseVault.constructor()`, current source lines ~75–80.

**Description**: Original constructor required only nonzero `asset_` and `asset_.code.length > 0`, allowing arbitrary testnet ERC-20 addresses (including attacker-authored lookalikes) to become the vault's custodial asset. That violates the intended trust model that all publicly demonstrated vaults hold exactly the reviewed Base Sepolia test-USDC contract.

**Proof of Concept**: Real Foundry red test `testBaseSepoliaRejectsUnapprovedERC20TokenAddress` failed with `next call did not revert as expected` against old constructor; separate `testBaseSepoliaRejectsUnapprovedCodeAtCanonicalTokenAddress` red test showed a fake contract deployed to the canonical test token address was also accepted.

**Recommendation / Implemented fix**: On chain 84532 require exactly `0x38d81431B9f76d9699AF7517040D5221D9dEA252` and SHA-256 of its deployed runtime bytes equal `db77a21a647b5973bac5728bf9f6977b56933006bf2f0730a14d109d33072c83`. Reject mismatched address or bytes. Both regression tests now pass. Chain 31337 still accepts disposable fixture tokens for local adversarial tests. Optional real-token fork test against pinned finalized block 55,557,819 continues to pass on the genuine public token.

## [M-02] On-chain vault verifies only the owner's QUOTE HASH, not the maker's quote signature (OPEN)

**Severity**: Medium

**Category**: EIP-712 signatures / cross-chain trust assumptions

**Location**: `constructor(... quoteHash_ ...)`, `paymentDigest()`, `executePayment()`.

**Description**: The EIP-712 agent signature commits to the immutable owner-selected `quoteHash`, but the Solidity vault does not independently validate the Tidepool/NEAR Intents maker signature, quote freshness, minimum destination output, refund, finality or on-chain NEAR Guard authorization. A malicious/compromised owner or bad integration could pin a fabricated quote hash; the vault would still enforce its own *recipient and budget* but cannot attest the off-chain quote to judges or a cross-chain settlement protocol.

**Proof of Concept**: Constructor accepts any nonzero 32-byte `quoteHash_`; a correctly signed `AgentPayment` commits that hash while having no `signed_payload`, manager public key, or off-chain proof argument. No fake funds are assumed. Existing Foundry tests for the immutable hash and EIP-712 signature are **not** cross-chain quote-authenticity tests.

**Recommendation**: Bind a *fresh cryptographically verified maker quote* and independently finalized NEAR Guard approval to a distinct, audited authorization or attestation interface BEFORE funding/issuing a live testnet agent order. Do not claim the Solidity vault verifies maker signatures or performs atomic cross-chain settlement.

## [M-03] Proxy/token administrative authority is not fully proven by a code-hash pin (OPEN)

**Severity**: Medium

**Category**: ERC-20 / upgradeability / access control

**Location**: `constructor()` SHA-256 check and `_checkTokenCode()` codehash check.

**Description**: Runtime-bytecode pinning defends against direct address substitution or code replacement. It does **not** prove that a proxy's implementation, beacon, admin, custom delegation logic or token blacklist/freeze rules cannot change independently. Owner balances could become frozen or token semantics could change under an issuer/upgrade authority, even though the vault code stays constant.

**Proof of Concept**: At Base Sepolia block 55,557,819 and independently observed finalized heads, both standard EIP-1967 implementation and beacon storage slots were zero under two public RPCs. This is positive but **insufficient to rule out custom upgrade patterns**, issuer privilege or proxy mechanisms outside those slots. Any stronger claim lacks supporting source evidence.

**Recommendation**: Before even a small public pilot, inspect and independently audit the target token's verified source, implementation/governance roles and any nonstandard delegation or freeze controls. Disallow unexpected runtime-bytecode changes and prefer immutable non-censorable test assets where appropriate.

## [M-04] Owner key remains an unrestricted single custody/recovery authority (OPEN BY DESIGN)

**Severity**: Medium

**Category**: Access control / operational signing trust

**Location**: `owner = msg.sender` and `recover()` after owner `revoke()`.

**Description**: The owner can revoke and recover all remaining vault escrow. That is an intended recovery right, but loss of the owner signing key defeats the vault's compromised-AGENT isolation, even if the agent EIP-712 key itself remains narrowly scoped.

**Proof of Concept**: `testOwnerOnlyDepositsAndOnlyRecoversAfterRevocation` demonstrates that the owner can recover funds after a terminal revoke, while an agent or relayer cannot. The same right would be abused if the owner's full-access signing key is compromised.

**Recommendation**: Keep owner keys outside model/agent processes and separate them from EIP-712 agent signer and relayer. Use hardware-backed or multi-signature owner custody for any later production-grade release. Publish the threat model without claiming non-bypassability under compromised OWNER keys.

## [L-05] Timestamp boundary and external call returndata remain residual integration risks (OPEN)

**Severity**: Low

**Category**: Chain-specific / DoS and gas griefing

**Location**: `block.timestamp` deadline checks; `_callToken()` low-level ERC-20 call.

**Description**: Production deployment depends on OP-stack timestamp inclusion and ERC-20 behavior. ERC-20 callbacks and enormous returned data may consume gas; issuer freeze/blocklisting could revert transfers. EVM rollback protects nonce and spent ledger from a failed payout, but could still leave escrow inaccessible if the token itself stops transferring.

**Proof of Concept**: Foundry emitted timestamp/reentrancy lint notices. Real malicious-token reentry and taxed-token tests are green, but these do not exhaust all gas or token issuer outcomes.

**Recommendation**: Use generous short testnet expiry margins, measure actual gas on a pinned public-chain fork, hold only tiny preapproved amounts, preserve revocation/recovery procedures, and perform independent review of the issuer/token implementation.

## Verified evidence and remaining blockers

- **Real red→green**: wrong testnet token and wrong runtime bytes were accepted before hardening and rejected after.
- **Foundry**: 29/29 offline tests pass (including 3 fuzz tests × 256 cases). The optional fork test skips without RPC. **1/1** opt-in real-token public-state fork test at pinned Base Sepolia block 55,557,819 passes, with no on-chain transaction.
- **Public RPC read-only token attestation**: two providers agreed on chain 84532, canonical historic and latest common finalized hashes, the exact deployed code hash and empty *standard* EIP-1967 implementation/beacon slots at the observed blocks. Evidence captured at `evidence/base-vault-local/base-sepolia-token-two-rpc-preflight-20261107.json` (SHA-256 `1d18dd3df98a743eacb505710c750e2c3902c3233b6e2dd6aed9223a456a218a`).
- **Verifier hardening (red/green):** an additional two fail-closed unit regressions found that identical malformed checkpoint hashes and RPC responses from the wrong requested height could be accepted. Both failed first, were fixed, and **8/8** token-preflight unit tests now pass. Updated stricter parser live replay: `evidence/base-vault-local/base-sepolia-token-two-rpc-attestation-v2-20261107.json` (SHA-256 `ad9ba58aaea0fb4ee3de31ef76951c286e54cba31d64a6b3bab2e298b1b56fa5`). This reread additionally rejects malformed canonical hash values and RPC responses whose block number does not match the specifically requested checkpoint. The original snapshot remains unchanged.
- **Exact code-change refusal:** a local adversarial test mutates the ERC-20 runtime bytes after escrow deposit and requires the exact `TOKEN_CODE_CHANGED` failure before another deposit or agent payout. This test passes and demonstrates fail-closed behavior if the pinned ERC-20 code itself changes; it does NOT detect nonstandard proxy implementation upgrades that leave the proxy runtime unchanged.
- **Not yet done**: an independent third-party security audit, user-approved owner key/agent key and precise deployment parameters, a live deployed Base testnet vault, public-chain denial/allowed receipts, fresh authenticated maker quote, verified NEAR Guard linkage, proof of EVM/NEAR atomicity or permissionless native bridge.
- **Disk warning**: x3star1 was at ~99% root utilization during this review. Avoid full-workspace builds, cache clearing, or long-running CI without explicit resource planning.

**Release decision:** HARD HOLD. Do not merge or publicly deploy this high-value-custody feature solely because internal audits or CI tests are green.
