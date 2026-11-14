# x3-mpc-guard: mandate-bound NEAR Chain Signatures for EVM (draft)

**Status: draft for issue #35. Not deployed. No MPC signature has been requested and no
transaction has been sent.** All results below come from native unit tests against near-sdk's
mocked blockchain and a local stand-in for the MPC network. They are not on-chain evidence.

This is a new contract, separate from the deployed `x3-guard` (`x3lang.near`). It does not
import, call or change the Guard crate. The Guard's mandate types and checks were copied and
adapted (`src/authority.rs`); see "Wiring to Guard" below for the later integration.

## What it enforces

The contract account is the only account that calls the MPC signer's `sign`. NEAR MPC derives
the signing key from the caller (predecessor) and a path, so the funded foreign address belongs
to this contract's account. An agent that calls the signer itself gets an unrelated, unfunded key
(`an_agent_calling_the_signer_itself_derives_an_unrelated_address`).

| Step | Method | Who |
|---|---|---|
| Derive and confirm the foreign address | `register_foreign_account(key_index)`, callback `on_foreign_account_checked` | owner |
| Grant, replace, revoke | `create_mandate(agent, terms)`, `revoke_mandate(agent)`, `withdraw_credit(agent)` | owner |
| Propose a payment (structured fields only) | `agent_propose_signing(owner, request)` | agent |
| Approve the exact intent hash | `approve_signing(owner, intent_nonce, intent_hash)` | listed approvers |
| Request the signature | `agent_request_signature(owner, intent_nonce)`, callback `on_signature` | agent |
| Clear a request whose callback never came | `mark_stale_request(owner, intent_nonce)` | owner or agent |

Before any promise to the signer exists, `agent_request_signature` re-checks: mandate active
(not revoked, not expired) and of the same version as the intent; program SHA-256; chain ID;
foreign account (`key_index`), verified by the signer; asset; recipient on the allowlist (exact
lowercase spelling); per-payment cap; rolling 24-hour cap and count; `max_fee_per_gas`,
`max_priority_fee_per_gas`, gas limit and total fee ceilings; intent expiry within the mandate and
the maximum TTL; N distinct listed approvals above the threshold; one request in flight per
account; the EVM nonce equals the next unsigned one; the stored transaction re-encodes to the
stored bytes and hash and the intent hash still matches; the payload is a valid secp256k1 scalar;
signer, domain, predecessor and path equal the contract's own; enough prepaid gas.

The agent never supplies bytes, a hash, a path, a key version or a domain. `SigningRequest`
refuses unknown JSON fields. The contract builds the transaction and the path itself.

## The signing intent

`X3SigningIntent` (version 1) holds: program SHA-256, owner, agent, mandate version, one-use
intent nonce, CAIP-2 namespace and chain ID, MPC signer account, domain, derivation predecessor
and path, key index, sending address, recipient, asset, amount, the built `to`/`value`/`data`,
EVM nonce, fee fields, gas limit, fee ceiling, the canonical unsigned EIP-1559 transaction
(`0x02 || rlp(...)`), its Keccak-256 signing hash, expiry and refund strategy. Its Borsh bytes
are the canonical preimage of the SHA-256 `intent_hash` approvers sign off on.

Assets: the native coin (value transfer, empty calldata) or one ERC-20 token
(`transfer(recipient, amount)` to the token, zero value). Access lists are always empty.

## Pinned external interfaces

| What | Source |
|---|---|
| `sign(request: {path, payload_v2: {"Ecdsa": hex32}, domain_id})`, 1 yoctoNEAR deposit, ≥15 Tgas (code default; README says 10), yield/resume callback, `SignatureResponse` `{"scheme":"Secp256k1","big_r":{"affine_point"},"s":{"scalar"},"recovery_id"}`; ECDSA payload must be below the group order; EdDSA is domain 1 and unsupported here | `near/mpc` commit `686fbb2de1a6b426303c23617ab84773f15bfb19` (workspace 3.19.0): `crates/contract/src/api/sign.rs`, `api/common.rs`, `config.rs`, `crates/near-mpc-crypto-types/src/sign.rs`, `primitives.rs`, `crates/near-mpc-contract-interface/src/deposits.rs`, `crates/contract/README.md` |
| `derived_public_key(path, predecessor, domain_id)` returns `secp256k1:<base58 64 bytes>` | same commit, `crates/contract/src/api/keys.rs`, `near-mpc-crypto-types/src/crypto.rs` |
| Tweak = SHA3-256(`"near-mpc-recovery v0.1.0 epsilon derivation:" + predecessor + "," + path`); key = root + G·tweak | same commit, `near-mpc-crypto-types/src/kdf.rs`, `contract/src/crypto_shared/kdf.rs` |
| Signer accounts `v1.signer` (mainnet), `v1.signer-prod.testnet` (testnet); root keys | docs.near.org chain-signatures implementation page; `chainsig.js` 1.1.16 `ROOT_PUBLIC_KEYS` |
| EIP-1559 encoding and signing hash | EIP-1559; Besu vectors in ethereumjs-monorepo `11695f6cec3d99e143169e51ee91a0b6d146908f` `packages/tx/test/testData/eip1559.ts` |

Verified against vectors: all 16 `derive_tweak` snapshot values and the `derive_key_secp256k1`
snapshot of `near/mpc`; all 10 Besu EIP-1559 signed transactions (byte-exact re-encoding, and each
published signature recovers to the vector's key over our signing hash); the ethereumjs
access-list signing hash. **Not reproduced:** the NEAR docs' example addresses for
`example.near`/`ethereum-1` do not come out of either chainsig.js root key, although chainsig.js
derives the same way; the example probably predates the current keys. The binding check is
on chain instead: an account is usable only after the signer's own `derived_public_key` returns
exactly the key this contract derived.

## Deployment preconditions (not enforced by code)

- The contract account must have **no access keys** after deploy and `new`. A key holder could
  call `sign` as this account and bypass every check.
- No owner, admin, upgrade or migration method exists. Changing the code means a new account,
  new derived addresses and new funding.
- The owner prepays credit in `create_mandate` (storage and the 1 yoctoNEAR per sign request,
  because an agent's function-call key cannot attach a deposit). The MPC deposit's purpose
  (proof of a full-access key) is replaced here by the mandate check.

## Known limitations

- If a signature request fails and the mandate is then revoked or replaced, that intent can no
  longer be retried, and its EVM nonce stays consumed: later transactions from that account
  cannot be mined until the owner fills that nonce. Recovery for this case is not built.
- Fees are capped per transaction, not counted against `daily_cap`.
- Only EIP-1559 EVM transfers. No Bitcoin, Solana or EdDSA domain.
- Not security reviewed, not deployed, not run against a real signer. WASM size and gas use
  of on-chain secp256k1 derivation are measured only by CI's build, not on chain.

## Wiring to Guard (after the hackathon freeze)

1. **Read-only link (no Guard change).** At request time, call the deployed Guard's existing
   `get_mandate(owner, agent)` view and continue in a callback: refuse (before creating the sign
   promise) unless a Guard mandate exists, is unrevoked and unexpired, and pins the same
   `program_sha256`. Revoking the agent in Guard then also stops MPC signing.
2. **Guard version with EVM terms.** A new Guard release stores the EVM terms (chain ID, asset,
   recipients, fee caps) in its mandate and exposes a check view; this contract then reads its
   terms from Guard instead of keeping its own, and its own `create_mandate` is removed.
3. **Single authority.** Merge the signing path into the Guard contract only with a new account
   and audit, because the derived address depends on the calling account.

## Commands

```bash
cd x3-lang/near/x3-mpc-guard
cargo fmt --all -- --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo build --locked --release --target wasm32-unknown-unknown
cd ../../.. && python3 scripts/security/mpc_guard_rust_mutations.py --timeout 300
```
