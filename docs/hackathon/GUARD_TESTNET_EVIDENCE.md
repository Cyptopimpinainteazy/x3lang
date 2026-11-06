# X3 Guard — NEAR testnet evidence (2026-11-06)

**Status: test-only mock signer — NOT an official 1Click production signature.**

This document records only sanitized, hash-chained RPC observations of the Guard contract on NEAR testnet. It does **not** claim settlement, cross-chain causality, wallet protection, or mainnet deployment. A signed quote proves signed terms, not a completed swap. Guard does not custody funds; an unrestricted wallet key can still bypass it.

Evidence files:

- [`evidence/near-guard-testnet/manifest.json`](../../evidence/near-guard-testnet/manifest.json) — deployed account + code identity
- [`evidence/near-guard-testnet/transactions.json`](../../evidence/near-guard-testnet/transactions.json) — collection of 8 finalized transactions with `record_hash` per entry

Verify independently (read-only, no signing):

```bash
PYTHONPATH=x3-lang .venv/bin/python scripts/hackathon/testnet_guard_evidence.py --verify evidence/near-guard-testnet/manifest.json
PYTHONPATH=x3-lang .venv/bin/python scripts/hackathon/testnet_guard_evidence.py --verify evidence/near-guard-testnet/transactions.json
```

Both exit `0` on 2026-11-06. Any tampered hash, chain-id, account, block height, or status fails verification. Collector default RPC is `https://test.rpc.fastnear.com` (HTTPS required; localhost HTTP only for tests). `EXPERIMENTAL_chain` is tried first per the brief; public RPCs answer `METHOD_NOT_FOUND` for it, so the collector falls back to `status.chain_id` and still requires `testnet`. Genesis recorded by RPC: `FWJ9kR6KFWoyMoNjpLXXGHeuiy7tEY6GmoFeCA5yuc6b`.

## Account and code

- Account: `x3lang-guard-20261106.testnet` (created by controller, tx `J3Cbtg5owXqJG3naPQ2Xow2BQnJdyFPZMoGoU7Cr4K4t`; 10 NEAR, empty code, key outside repo `0600` — not copied, not printed here)
- `view-account-summary … network-config testnet now` before deploy: 10 NEAR, 182 B, “No contract code”
- Manager key pinned by deployed `new()`: `ed25519:reYaWhvwu8Jzo3WUM3zhn6VrhuMEF4eADL17qtRVifc` (confirmed via `get_manager_public_key` view; `state_version` = 2)
- WASM built twice from a clean target with `cargo build --release --locked --target wasm32-unknown-unknown`; both digests equal:
  - `ac44fa53acf514d26c7adf883b31b49a125faf5a4c1074928d3e1fe94b36f8ab`, 544944 bytes
  - On-chain `code_hash` `CbU8SpgfTgG1egbw8ygMTK3deozvDnoFrF8DJ8qifSe6` = base58(sha256(wasm)) per NEAR code-hash semantics
  - After deploy: 9.995876 NEAR, 545.2 KB storage, local SHA-256 shown by CLI matches above

## Transactions (all `FINAL`, signer `x3lang-guard-20261106.testnet`)

| Label | Hash | Block | Methods | Overall status |
|---|---|---|---|---|
| deploy | `FJ5voQaArx17QV4TzDLYDq93cuoAVLSRxjcxbswuyE8v` | 276546688 `AedwFq82FcDkRV6N7wotXTiUBhV8wHXrMFuCS9Lmfepo` | DeployContract, new | Success |
| allowed_register | `9odHjwPhqG74rNtNvbPeNsAtNYupYHzpZBDkM8VY9ehx` | 276547096 | register_intent | Success |
| invalid_register | `D3ca73CX4ACr9wFhqdhGyrmx4HrrBCgG69So1TncxaKT` | 276547116 | register_intent | Success |
| prohibited_register | `GtRZAo4rRr4QQGVc9s8bGPTq4j4B4EziZkQirqQgQjw` | 276547124 | register_intent | Success |
| invalid_accept | `9GthsTiAvMvPDjgGLuB2CopVAPXkW5Lmw7mYBGCtsXXe` | 276547148 | accept_quote | Failure (trap) |
| create_mandate | `J4WrxJTNhL5Z841thsvpmMCgsMWwrmUsrmrwHSMu9tXX` | 276547210 | create_mandate | Success |
| revoke_mandate | `7aVonMiRCDyetbkpRBK6GKnL2X1XrQTPsf3Pk1gKGzp8` | 276547226 | revoke_mandate | Success (event `mandate_revoked`, agent `x3-agent-20261106.testnet`) |
| prohibited_accept | `3Q7qTZqiRBB6EuhN78Ra5Q5NBqPXDhfuCg2FKW8WoU2q` | 276547296 `FRtCixxuptu1mbWaXs4CgjFyFHATPbUyvzPxKsauSVNT` | accept_quote | Failure (trap) |

Each entry stores its own block hash/height, receipt IDs/statuses, `method_names`, and `args_sha256` with signatures/payloads replaced by SHA-256 hashes in sanitized args (no private-key fields, no env data). Failed `accept_quote` receipts show a generic WebAssembly `unreachable` trap on-chain; the precise guard reason is in the view below and in contract code (`X3_NEAR_INVALID_SIGNATURE` vs `quote rejected by …`), not in the receipt string.

## Paths

- **Allowed:** `register_intent` for `evidence-allowed-02` succeeded. `check_quote` view for an allowed policy shape would pass signature + guards only for a matching live quote; no live quote was accepted here.
- **Invalid signature:** `evidence-invalid-01` + forged signature → `accept_quote` failed (`9Gths…`); on-chain `check_quote` returns `quote_signature ok:false, "signature does not verify under the 1Click manager key"`.
- **Revoked:** `create_mandate` then `revoke_mandate` succeeded; subsequent agent acceptance would be refused by `mandate_active`. This evidences the revocation path, not an acceptance.
- **Valid-but-prohibited (official dry quote):** real manager-signed **dry** quote fetched from public `https://1click.chaindefuser.com/v0/quote` (`dry:true`, amount 25000000, sol-USDC → near-USDC, recipient `wrap.near` vs policy recipient `x3lang-guard-20261106.testnet`, timestamp `2026-11-06T22:34:26.239Z`). Local `check_oneclick_quote_signature` = `(True, verified)` against the pinned key. On-chain `check_quote` (still fresh at 2026-11-06T22:35Z) returned `quote_signature ok:true (DmNz8fw…)`, but `quote_kind ok:false`, `quote_request_echo ok:false mismatched:["recipient"]`, `refund_fee ok:false`, `deposit_address ok:false`; `accept_quote` then failed (`3Q7q…`). This is a signed-dry + recipient-mismatch refusal — **not** a live-quote acceptance and **not** proof that a live valid quote paying the accepted recipient would be accepted. No test-only mock signer was presented as official.

## Blockers and non-claims

- **Live valid-quote acceptance blocker:** no live (`dry:false`) official quote was accepted. Acceptance would require a live quote whose deposit address, deadline, and `refundFee` pass every guard within 5 minutes; the dry path cannot reach it by design. Not attempted here.
- **No `mandate_recipient` on-chain acceptance refusal:** recipient refusal was at `quote_request_echo` on the owner `accept_quote` path, not via `agent_accept_quote`/`mandate_recipient`. That path was exercised locally only.
- **No settlement/causality:** no deposit was sent, no `record_outcome` was called, no solver fill or refund is claimed.
- **RPC trust:** evidence is recorded from public RPC responses with canonical hashing and independent re-query; it is not a light-client finality proof.

Reproduce collector tests: `PYTHONPATH=x3-lang .venv/bin/python -m pytest scripts/hackathon/test_testnet_guard_evidence.py -q` (5 passed).
