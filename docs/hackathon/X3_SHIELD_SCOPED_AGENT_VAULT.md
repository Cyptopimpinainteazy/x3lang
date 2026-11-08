# X3 Shield — non-bypassable **NEAR testnet pilot** (local Sandbox first)

**Judging claim (bounded):** A disposable agent's NEAR key has **only** FunctionCall permission to `request_payment` on a contract-funded vault. The agent cannot use that key to move vault reserves directly; the vault itself enforces a recipient allowlist, per-payment cap, aggregate budget across pending/completed transfers, expiry, owner revocation, and unique nonces. The testnet-named vault is separately initialized by a trusted deployment account.

**Not claimed:** mainnet security, independent custody audit, a trustless bridge, a secured Base Sepolia EOA signing path, or proof that the owner's *full-access* key cannot move funds. An owner can still update/recover its vault using an unrestricted key, which must stay out of the agent runtime. This pilot is separate from the real Tidepool trade already proven in PR #109.

**Original scoped-agent pilot evidence:** [read-only Sandbox result](../../evidence/agent-vault-local/verified-sandbox-20261107.json), Rust 1.99.1 WASM SHA-256 `971d9cc43982b5ca9cdb30deab87e39ac09e752850b20773cace20cae2886128`. The direct transfer was rejected by the RPC with `ServerError(InvalidTransaction)`; it did **not** create an on-chain receipt. The recovered-escrow prototype uses a **different** locally compiled WASM SHA-256 `fe5bad16b3242eba09f17042f87f55ec4174b5fd2c866c0b7f79bd42a4dadfc3` and its own ephemeral node run; the original artifact must not be relabeled as proof of this new code.

**Recovery-pilot local evidence:** [`owner-recovery-sandbox-20261107.json`](../../evidence/agent-vault-local/owner-recovery-sandbox-20261107.json), exact SHA-256 `6fee7b31d7fe1b36c627dc539f936a38643a97872fe45557b56bbe2063f45e01`, records runner assertions from the isolated NEAR Sandbox. The local node reported a **0.5 NEAR owner recovery** and **0.4996041409828937 NEAR net owner balance increase after gas**. This is *not* independent public-chain proof. Reproduce with `scripts/hackathon/vault_sandbox_smoke.sh` using the pinned Sandbox binary.

## Local real-chain attack matrix

The local test used NEAR Sandbox 2.14.0 with an offline pinned binary, an isolated local blockchain and disposable generated accounts. The test **did not import any user's wallet credentials, contact public RPC or move real funds**.

| Test | Expected | Verified local result |
|---|---|---|
| Query on-chain agent access key | `FunctionCall` to `vault.testnet::request_payment` only | Key scope found on local chain |
| Direct NEAR transfer using only agent key | Permission denial | Refused by SDK/RPC path; may be rejected before broadcast; **not represented as an on-chain failed transaction receipt** |
| Agent chooses `attacker.testnet` | Rejected by vault | Transaction failed in local Sandbox |
| Agent chooses amount above cap | Rejected by vault | Transaction failed in local Sandbox |
| Agent requests permitted 0.1 NEAR | Paid from vault (not agent account) | Successful local Sandbox transaction, receipt state `Paid` |
| Agent reuses same nonce | Rejected | Transaction failed in local Sandbox |
| Owner revokes, agent tries new nonce | Rejected | Transaction failed in local Sandbox |
| Owner recovery before revocation | Rejected | Transaction failed in recovery-pilot local Sandbox |
| Compromised scoped key calls `recover_available` | Node/contract rejects | Forbidden FunctionCall permission; refusal observed in recovery-pilot local Sandbox (SDK/RPC class not assumed to be a receipt) |
| Owner recovers 0.5 test NEAR after revocation | Actual async transfer succeeds and reduces available escrow | Recovery-pilot local Sandbox verified `available` 0.9 → 0.4 test NEAR; not a public network transfer |

**Local Sandbox CLI:**

```bash
bash scripts/hackathon/vault_sandbox_smoke.sh
```

Requires pinned cached `near-sandbox` 2.14.0 with SHA-256 `797b9d7a9a23907048d68a427e86a55c94f1de232dbfb8b8a1eec09ecef3bf13`. Ubuntu with glibc <2.39 uses a local cached `ubuntu:24.04` Docker image **with networking disabled**. The harness generates its own ephemeral agent private key, which is never printed, committed, or sent to ChatGPT.

### Local Rust contract tests

```bash
cd x3-lang/near/x3-agent-vault
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
cargo fmt --check
cargo build --locked --release --target wasm32-unknown-unknown
```

These tests include negative wrong-recipient/over-cap/replay/revoke/expired/forged-caller cases, plus failure-callback restoration without nonce reuse, and aggregate budget while transfers are in-flight. **Failed wallet transfer credits return to spendable escrow while their original nonce stays burned.**

### Safety/operational restrictions

1. Vault account must end in `.testnet`; naming alone **does not authenticate NEAR genesis**. Deployment tools must independently verify the actual testnet chain/genesis through pinned RPCs.
2. Deploy & initialize the vault **in a single transaction** to prevent hostile initialization of an uninitialized contract. Owner and agent keys remain distinct.
3. Before public-testnet deployment, review the precise vault WASM/source SHA, recipient, test-token funding limit, owner recovery process, account storage/gas reserve, access key allowance, contract upgrade authority and downgrade/rollback handling.
4. Never import or publish owner's full-access key. A leaked agent *scoped key* is the test adversary; if the agent also has access to a funded EVM EOA secret, this NEAR demonstration **does not constrain that EVM wallet**.
5. Contract reserved/available accounting fails closed on missing callbacks and does not imply successful payout until its asynchronous receipt arrives. Any on-chain deploy or funding requires separately approved exact transactions.
6. This vault currently pays **native NEAR only**. It does not yet integrate the Tidepool quote Guard, NEP-141 tokens or the native BSC↔NEAR bridge; expansion requires separate proofs and review.

## Storage exhaustion and owner recovery pilot — do not deploy

A compromised scoped agent can spend **one yoctoNEAR** per distinct nonce,
incurring payment-history and replay-tombstone storage far out of proportion
to the spending budget. The provisional vault hard-stops after **64 lifetime
accepted payments** (pending, paid or failed) before adding a 65th nonce.
The capacity denial does not debit escrow or reserve funds, but it allows
a compromised agent to exhaust all slots and deny future legitimate requests.
There is no audited pruning or migration design.

**Proposed testnet-only recovery state machine:** `recover_available(amount)`
is owner-only, and rejects calls until the mandate is revoked or expired.
It only withdraws the unreserved `available` escrow, keeps pending payouts
in `reserved`, and additionally protects NEAR account storage stake based
on current usage and byte cost plus a **provisional 0.5 NEAR gas/callback
buffer**. A single `pending_recovery` records an in-flight transfer;
the asynchronous callback clears it on success or restores the exact
debit on failure. Without a finalized callback the amount remains pending
and cannot be re-withdrawn. The scoped agent key has no owner-recovery
permission. This is an incremental safety boundary, not audited economics.

**Still release-blocked:** the new Borsh state field has **no compatible
migration path for an existing deployed vault**, and there is no recovery
if an asynchronous callback is permanently missing. The fixed buffer,
actual storage deposits, gas costs, byte growth, callback safety under
adversarial node conditions and 64-slot denial of service still require
independent testing/review. The sandbox-only prototype must **not**
be upgraded onto an existing funded contract or funded/deployed on any
public testnet until these gates and explicit owner authorization are met.

This work does not demonstrate mainnet readiness, cross-VM custody
enforcement, or trustless settlement, and it does not modify Tidepool Guard.

## Readable refusals and build mode

Every refusal carries a stable code: `X3_VAULT_INIT: …` (initialization),
`X3_VAULT_DENIED: …` (caller, mandate, nonce, cap, budget or escrow checks) or
`X3_VAULT_LEDGER: …` (callback state). The crate builds with `--cfg near`
(`x3-lang/near/x3-agent-vault/.cargo/config.toml`, as `cargo near build`) and refuses
to compile for wasm32 without it. Without that flag near-sdk discards every refusal message
and the explorer shows only `WebAssembly trap: unreachable`; the same build defect is in the
deployed X3 Guard (see `GUARD_TESTNET_EVIDENCE.md`).

This changes the compiled WASM. The SHA-256 values and Sandbox evidence above belong to the
earlier builds: re-run `scripts/hackathon/vault_sandbox_smoke.sh` and record new evidence
before citing results for this source.

**Dust storage:** the 64-record history cap bounds what a compromised agent can lock in
storage stake to roughly 64 records of state (about 0.08 NEAR at the current byte cost), and
`recover_available` reserves the account's actual storage stake. Charging each record's
storage to the agent's budget, proposed in #134, was not adopted.

## 90-second judge narration (recording plan, NOT a completed video)

**0–15s:** "X3 Shield doesn't just ask an AI agent to obey rules. This vault makes the spending boundary enforceable, with only a FunctionCall key available to the agent." Show local on-chain access-key permission.

**15–40s:** "Here is a compromised agent attempting a direct NEAR transfer, an attacker recipient and an oversized amount." Demonstrate refusal class honestly: direct attempt may be stopped by SDK access-key selection; wrong-recipient and amount cap are real local NEAR Sandbox failed transactions.

**40–60s:** "Now the same agent makes one allowed 0.1 test NEAR payment. The vault completes its async callback; replay of the nonce is denied."

**60–75s:** "The owner revokes permission and the agent tries again. A fresh request is denied." Show state and receipts from the same Sandbox run.

**75–90s:** "Separately, X3Lang already completed a real Base Sepolia → NEAR testnet trade with finalized independent proof. That uses a trusted Tidepool relayer, not this new vault and not our permissionless native bridge." Link to `docs/hackathon/FIRST_SWAP_JUDGE_WALKTHROUGH.md`.

**Do not claim** this is a hosted, public testnet, custody-audited or EVM non-bypassable demonstration until those things are independently proven.
