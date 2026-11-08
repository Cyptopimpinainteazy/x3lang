# X3 Guard — NEAR testnet evidence (2026-11-06)

**Status: mixed evidence (see below) — test-key invalid-signature path + official manager-signed dry and live quotes. No settlement.**

Mixed evidence, precisely:
- **Invalid-signature path:** forged signature (test-key style corruption of a fixture payload) refused at `quote_signature`. This path does **not** involve the official manager key beyond the pinned verification key.
- **Refusal path:** official production-manager-signed **dry** quote (`dry:true`, manager `ed25519:reYa…`) from `https://1click.chaindefuser.com/v0/quote`, signature-valid but policy-refused (`quote_request_echo` recipient mismatch + `quote_kind`/`refund_fee`/`deposit_address`). Dry refusal alone does not prove a live acceptance.
- **Allowed/live path (new):** official production-manager-signed **live** quote (`dry:false`, same manager, timestamp `2026-11-06T22:51:40.892Z`) matching the registered policy, `accept_quote` succeeded (`quote_accepted`, tx `6B6k…`) — positive control. **Stopped before any deposit; no value moved; no `record_outcome`, no settlement/causality claim.**

This document records only sanitized, hash-chained RPC observations of the Guard contract on NEAR testnet. It does **not** claim settlement, cross-chain causality, wallet protection, or mainnet deployment. A signed quote proves signed terms, not a completed swap. Guard does not custody funds; an unrestricted wallet key can still bypass it.

Evidence files:

- [`evidence/near-guard-testnet/manifest.json`](../../evidence/near-guard-testnet/manifest.json) — deployed account + code identity
- [Reproduce deployed WASM exact bytes](GUARD_WASM_BYTE_REPRODUCIBILITY.md) — toolchain-pinned, offline build against committed manifest
- [`evidence/near-guard-testnet/transactions.json`](../../evidence/near-guard-testnet/transactions.json) — collection of 13 finalized transactions with `record_hash` per entry
- [`evidence/near-guard-testnet/check-quote-views.json`](../../evidence/near-guard-testnet/check-quote-views.json) — sanitized `check_quote` views (dry-refused vs invalid vs live-allowed)
- [`evidence/near-guard-testnet/quote-dry-official.json`](../../evidence/near-guard-testnet/quote-dry-official.json) — sanitized official dry quote + signature verification
- [`evidence/near-guard-testnet/quote-live-official.json`](../../evidence/near-guard-testnet/quote-live-official.json) — sanitized official live quote + signature verification

Verify independently (read-only, no signing):

```bash
PYTHONPATH=x3-lang .venv/bin/python scripts/hackathon/testnet_guard_evidence.py --verify evidence/near-guard-testnet/manifest.json
PYTHONPATH=x3-lang .venv/bin/python scripts/hackathon/testnet_guard_evidence.py --verify evidence/near-guard-testnet/transactions.json
```

Both exit `0` on the 2026-11-07 UTC recapture. Any tampered hash, chain id, genesis, account,
block height/hash, code view, or status fails verification. The manifest records
the exact block returned by the final account view, not a moving RPC status
head. Verification requires that historical block and fails instead of falling
back to current final state. Collector default RPC is
`https://test.rpc.fastnear.com` (HTTPS required; localhost HTTP only for tests).
The chain id is `testnet` and the pinned genesis is
`FWJ9kR6KFWoyMoNjpLXXGHeuiy7tEY6GmoFeCA5yuc6b`.
After building the Guard Wasm, append
`--wasm x3-lang/near/x3-guard/target/wasm32-unknown-unknown/release/x3_guard.wasm`
to either command so the verifier also recomputes the local byte length and
SHA-256.

## Account and code

- Account: `x3lang-guard-20261106.testnet` (created by controller, tx `J3Cbtg5owXqJG3naPQ2Xow2BQnJdyFPZMoGoU7Cr4K4t`; 10 NEAR, empty code, key outside repo `0600` — not copied, not printed here)
- `view-account-summary … network-config testnet now` before deploy: 10 NEAR, 182 B, “No contract code”
- Manager key pinned by deployed `new()`: `ed25519:reYaWhvwu8Jzo3WUM3zhn6VrhuMEF4eADL17qtRVifc` (confirmed via `get_manager_public_key` view; `state_version` = 2)
- WASM built twice from a clean target with `cargo build --release --locked --target wasm32-unknown-unknown`; both digests equal:
  - `ac44fa53acf514d26c7adf883b31b49a125faf5a4c1074928d3e1fe94b36f8ab`, 544944 bytes
  - On-chain `code_hash` `CbU8SpgfTgG1egbw8ygMTK3deozvDnoFrF8DJ8qifSe6` = base58(sha256(wasm)) per NEAR code-hash semantics
  - After deploy: 9.995876 NEAR, 545.2 KB storage, local SHA-256 shown by CLI matches above
- Pinned account-view block: height `276558043`, hash
  `8DJx3nHMAbznUfTqFNswobCdcLqudkHS6BaTMjxjezst`
- Manifest record hash:
  `7e470b4a2567ed1ad18bccc8d5743148972cafbabd4b17691a794191f7c7115a`
- Transaction-collection record hash:
  `080bfd59b9644eb029694e85dafe8013a58348be07ed92b8e8700ba539fb159b`

## Transactions (all `FINAL`, signer `x3lang-guard-20261106.testnet`)

| Label | Hash | Block | Methods | Overall status |
|---|---|---|---|---|
| account_create | `J3Cbtg5owXqJG3naPQ2Xow2BQnJdyFPZMoGoU7Cr4K4t` | 276545403 `89Yri5bkVMRLxKna4S5pJ2mjvDiiJdDELwwCwPRmnzV8` | CreateAccount, Transfer, AddKey (signer `testnet`) | Success |
| deploy | `FJ5voQaArx17QV4TzDLYDq93cuoAVLSRxjcxbswuyE8v` | 276546688 `AedwFq82FcDkRV6N7wotXTiUBhV8wHXrMFuCS9Lmfepo` | DeployContract, new | Success |
| allowed_register | `9odHjwPhqG74rNtNvbPeNsAtNYupYHzpZBDkM8VY9ehx` | 276547096 | register_intent | Success |
| invalid_register | `D3ca73CX4ACr9wFhqdhGyrmx4HrrBCgG69So1TncxaKT` | 276547116 | register_intent | Success |
| prohibited_register | `GtRZAo4rRr4QQGVc9s8bGPTq4j4B4EziZkQirqQgQjw` | 276547124 | register_intent | Success |
| invalid_accept | `9GthsTiAvMvPDjgGLuB2CopVAPXkW5Lmw7mYBGCtsXXe` | 276547148 | accept_quote | Failure (trap) |
| create_mandate | `J4WrxJTNhL5Z841thsvpmMCgsMWwrmUsrmrwHSMu9tXX` | 276547210 | create_mandate | Success |
| revoke_mandate | `7aVonMiRCDyetbkpRBK6GKnL2X1XrQTPsf3Pk1gKGzp8` | 276547226 | revoke_mandate | Success (event `mandate_revoked`, agent `x3-agent-20261106.testnet`) |
| prohibited_accept | `3Q7qTZqiRBB6EuhN78Ra5Q5NBqPXDhfuCg2FKW8WoU2q` | 276547296 `FRtCixxuptu1mbWaXs4CgjFyFHATPbUyvzPxKsauSVNT` | accept_quote | Failure (trap) |
| live_register_short | `5apKRjRnHkPL4z1dUELwyEJgFQ2S9tXBUQiSg7ymv9BD` | 276549021 | register_intent (policy +2h; superseded by deadline blocker) | Success |
| live_register | `7G4RSYQiZB88qunZ2xrmWAGneEMSxBmLjadcrbFtXS1q` | 276549052 | register_intent (policy +7d for live deadline) | Success |
| live_accept_storage_fail | `D9jzFrV88GVUwJ8GUQXwH5TswwFmj9sa3EcXfFysSsVK` | 276549115 | accept_quote (0.01 NEAR storage too small → trap after `quote_accepted` event, rolled back) | Failure |
| live_accept | `6B6k5kre4mFoBq7RuJNadQgrhEvmgb9WDLepX22CxzUZ` | 276549145 `3u6f8jhCi7yaF4XT3TZKPFEo3ZxWFQYigFN5yAiLB2JG` | accept_quote (0.1 NEAR storage) | **Success — `quote_accepted` `FugpJ…` deposit `Ck96…`** |

Each entry stores its own block hash/height, receipt IDs/statuses, `method_names`, and `args_sha256` with signatures/payloads replaced by SHA-256 hashes in sanitized args (no private-key fields, no env data). Failed `accept_quote` receipts show a generic WebAssembly `unreachable` trap on-chain; the precise guard reason is in the view below and in contract code (`X3_NEAR_INVALID_SIGNATURE` vs `quote rejected by …`), not in the receipt string.

### Why the deployed binary's refusals carry no reason text

The deployed code (`ac44fa53…`, source `800a077e`) was compiled with plain
`cargo build --target wasm32-unknown-unknown`, **without `--cfg near`**. near-sdk 5.x only
calls the `env.panic_utf8` host function in that on-chain mode; otherwise `env::panic_str`
and `require!` drop their message and execute a WebAssembly `unreachable`. The deployed WASM
therefore cannot report *any* refusal reason, including the storage-deposit check in
`D9jz…`. This is a build-mode defect, not a missing check: the guard decisions are unchanged.

Source after this fix sets `--cfg near` in `x3-lang/near/x3-guard/.cargo/config.toml`,
refuses to compile for wasm32 without it, and is gated in CI by
`scripts/hackathon/check_guard_wasm_host_panics.py` (`env.panic_utf8` must be imported) and
by the local NEAR Sandbox run, which requires refused calls to fail with
`Smart contract panicked: X3_QUOTE_REJECTED …` / `X3_MANDATE_DENIED …` and never a bare
`unreachable`. The deployed-byte proof also asserts the legacy binary lacks that import.

**None of this changes the deployed contract.** The receipts above stay exactly as recorded.
Readable on-chain refusals exist only after an owner-authorized redeploy, which produces a
**new** code hash that must be recorded as a new deployment and never presented as
`ac44fa53…`/`CbU8Sp…`.

## Paths

- **Allowed/live (positive control):** `evidence-allowed-live-02` (`defuse.near`, +7d policy) + official **live** quote (`dry:false`, `Ck96…`, deadline `2026-11-09T23:01:40.753Z`, hash `FugpJ…`, ts `2026-11-06T22:51:40.892Z`) → `check_quote` all `ok:true`, `accept_quote` `6B6k…` succeeded with `quote_accepted`. Stopped before deposit; no SOL/USDC sent to `Ck96…`. First attempt `D9jz…` failed only on storage deposit (0.01 NEAR insufficient after event, rolled back); retry with 0.1 NEAR succeeded from the same fresh quote. Short-policy attempt (`5apK…`, +2h) failed `deadline` because live 1Click deadlines are ~3 days out.
- **Invalid signature:** `evidence-invalid-01` + forged signature → `accept_quote` failed (`9Gths…`); on-chain `check_quote` returns `quote_signature ok:false, "signature does not verify under the 1Click manager key"`.
- **Revoked:** `create_mandate` then `revoke_mandate` succeeded; subsequent agent acceptance would be refused by `mandate_active`. This evidences the revocation path, not an acceptance.
- **Valid-but-prohibited (official dry quote):** real manager-signed **dry** quote fetched from public `https://1click.chaindefuser.com/v0/quote` (`dry:true`, amount 25000000, sol-USDC → near-USDC, recipient `wrap.near` vs policy recipient `x3lang-guard-20261106.testnet`, timestamp `2026-11-06T22:34:26.239Z`, hash `DmNz8…`). Local `check_oneclick_quote_signature` = `(True, verified)` against the pinned key. On-chain `check_quote` (still fresh at 2026-11-06T22:35Z) returned `quote_signature ok:true`, but `quote_kind ok:false`, `quote_request_echo ok:false mismatched:["recipient"]`, `refund_fee ok:false`, `deposit_address ok:false`; `accept_quote` then failed (`3Q7q…`). This is a signed-dry + recipient-mismatch refusal — **not** a live-quote acceptance. Kept for contrast: signature-valid refusal vs invalid-signature failure (`9Gths…`). Live `wrap.near` quotes fail 1Click 400 (contract account); live control uses `defuse.near` instead.

## Blockers and non-claims

- **No `mandate_recipient` on-chain acceptance refusal:** recipient refusal was at `quote_request_echo` on the owner `accept_quote` path, not via `agent_accept_quote`/`mandate_recipient`. That path was exercised locally only.
- **No settlement/causality:** no deposit was sent to the live `Ck96…` address, no `record_outcome` was called, no solver fill or refund is claimed. `accept_quote` only authorizes a deposit address.
- **RPC trust:** evidence is recorded from public RPC responses with canonical hashing and independent re-query; it is not a light-client finality proof.

Reproduce collector tests: `PYTHONPATH=x3-lang .venv/bin/python -m pytest scripts/hackathon/test_testnet_guard_evidence.py -q` (16 tests, including genesis and exact-block negative cases).
