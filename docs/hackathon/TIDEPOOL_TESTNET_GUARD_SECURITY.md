# Isolated Tidepool testnet Guard (research candidate)

**Status:** not reviewed, **not deployed**, not authorized for live funds. Original `x3-guard` official 1Click contract and key are unchanged. Any new testnet deploy requires owner confirmation, a compiled WASM hash, an approved testnet contract account and a separately signed transaction.

## Scope and trust limits

This contract is a **policy approval ledger**, not an EVM wallet, bridge, MPC signer or escrow. An independently funded Base Sepolia wallet can bypass its decision. To claim X3-authorized execution, the *actual sender* must consume an approved quote hash in a non-bypassable signing workflow. Tidepool's relayer and deposit addresses are community-operated and trusted on the testnet; their API statuses alone are not independent settlement evidence. Neither this contract nor a testnet settlement establishes safe mainnet atomicity.

It pins the Tidepool maker's independently observed NEAR testnet FullAccess public key, which is **not** the official 1Click manager key. It verifies `ed25519_verify(SHA256(signed_payload), signature, pinned_key)` before parsing the quote. `signed_payload` must be the byte-exact compact JSON serialization of the maker's quote response **with the top-level `signature` removed**, preserving original key order, and the signature is supplied separately. Provider serialization changes must fail closed until reviewed; do not guess another representation or fall back to HTTP trust. One genuine provider-signed **dry** historical fixture verifies cryptographically but is refused by `authorize` because it is not live.

The owner registers an exact `.x3` program SHA-256, unique nonce, 1 test USDC amount, minimum payout, Base Sepolia refund address, 1% maximum slippage and expiry. The contract rejects unregistered owners, revoked/reused nonces, signed quotes for any other chain/asset/recipient, expired/future messages, insufficient output, invalid EVM deposit address and dry quotes. Storage is charged to the caller. The contract refuses initialization unless its *account name* ends in `.testnet`; that name check alone is **not a cryptographic network ID guarantee**, so deployment tooling must additionally verify the NEAR testnet genesis and RPC. A testnet domain or maker key must **never** be reused in production.

## Required before any swap

1. Confirm the owner controls `x3lang.testnet` NEAR signing authority and a **specific** Base Sepolia sender/refund EVM address. No key material in chat or GitHub.
2. Independently verify Base Sepolia test USDC token, wallet token balance and native test ETH for gas. Tidepool's faucet can provide test USDC to a confirmed EVM address; a separate Base Sepolia ETH faucet is required. **Do not fund an unconfirmed address.**
3. Independent security review and exact-head native tests, clippy, WASM build, GitHub CI, source-to-binary proof and proper storage accounting; on-chain testnet deployment only with explicit approval.
4. Register owner policy and obtain a NEW live quote (never use the historical dry fixture); verify signature on-chain and owner policy before wallet signing.
5. Sender signs only exact token, amount, chain ID 84532, signed deposit address, nonce, gas ceiling and expiry, and refuses any quote hash absent from approved Guard state. Owner approves exact transaction.
6. Verify independently finalized Base Sepolia transfer and NEAR testnet delivery (token balance change and receipt); record two-chain RPC evidence and refund outcomes. Never claim settlement solely from a Tidepool `SUCCESS` status.

**Mainnet blockers remain** #42, #56, #71, #84 and independent security and custody reviews. Even a successful testnet swap is not permission for mainnet activation.
