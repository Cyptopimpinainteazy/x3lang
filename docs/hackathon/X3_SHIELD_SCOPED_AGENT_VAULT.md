# X3 Shield — non-bypassable **NEAR testnet pilot** (local Sandbox first)

**Judging claim (bounded):** A disposable agent's NEAR key has **only** FunctionCall permission to `request_payment` on a contract-funded vault. The agent cannot use that key to move vault reserves directly; the vault itself enforces a recipient allowlist, per-payment cap, aggregate budget across pending/completed transfers, expiry, owner revocation, and unique nonces. The testnet-named vault is separately initialized by a trusted deployment account.

**Not claimed:** mainnet security, independent custody audit, a trustless bridge, a secured Base Sepolia EOA signing path, or proof that the owner's *full-access* key cannot move funds. An owner can still update/recover its vault using an unrestricted key, which must stay out of the agent runtime. This pilot is separate from the real Tidepool trade already proven in PR #109.

**Verified real offline evidence:** [read-only Sandbox result](../../evidence/agent-vault-local/verified-sandbox-20261107.json), with exact Rust 1.99.1 local WASM SHA-256 `971d9cc43982b5ca9cdb30deab87e39ac09e752850b20773cace20cae2886128`. The direct transfer was rejected by the RPC with `ServerError(InvalidTransaction)`; it did **not** create an on-chain receipt, so the demo distinguishes protocol rejection from executed failed receipts.

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

## Storage exhaustion and owner recovery blockers — do not deploy

A compromised scoped agent can spend **one yoctoNEAR** per distinct nonce,
incurring payment-history and replay-tombstone storage far out of proportion
to the spending budget. The provisional vault now hard-stops after **64
lifetime accepted payment requests** (including pending, paid and failed payments)
before adding a 65th nonce/record. The new regression confirms the capacity
denial does not debit escrow or reserve funds. This is a bounded-storage
safety stop, **not** a measured or audited storage-deposit accounting policy:
the account's actual storage stake, code size, reserve, gas costs and
callback liquidity still require independent testing. A compromised agent
could deliberately exhaust all 64 slots and deny subsequent legitimate
payments; the prototype has no safe on-chain pruning/migration workflow.

**Critical release blocker:** the contract also lacks an owner-only
escrow recovery/withdrawal method. After owner funding, any `available`
escrow left after revocation/expiration, exhausted payment slots or normal
use **cannot currently be reclaimed through this contract's API**.
An administrator holding the vault account's FullAccess key might be
able to replace code, but an unreviewed upgrade is not a safe recovery
procedure. Require an independently audited recovery state machine with
reserve/pending-payout protection, a rollback-safe migration plan and a
real local-node failure matrix before **any public-testnet funding**.

This safeguard is **not** evidence of mainnet readiness or a finished P0
custody system, and it does not alter the pre-existing Tidepool Guard.

## 90-second judge narration (recording plan, NOT a completed video)

**0–15s:** "X3 Shield doesn't just ask an AI agent to obey rules. This vault makes the spending boundary enforceable, with only a FunctionCall key available to the agent." Show local on-chain access-key permission.

**15–40s:** "Here is a compromised agent attempting a direct NEAR transfer, an attacker recipient and an oversized amount." Demonstrate refusal class honestly: direct attempt may be stopped by SDK access-key selection; wrong-recipient and amount cap are real local NEAR Sandbox failed transactions.

**40–60s:** "Now the same agent makes one allowed 0.1 test NEAR payment. The vault completes its async callback; replay of the nonce is denied."

**60–75s:** "The owner revokes permission and the agent tries again. A fresh request is denied." Show state and receipts from the same Sandbox run.

**75–90s:** "Separately, X3Lang already completed a real Base Sepolia → NEAR testnet trade with finalized independent proof. That uses a trusted Tidepool relayer, not this new vault and not our permissionless native bridge." Link to `docs/hackathon/FIRST_SWAP_JUDGE_WALKTHROUGH.md`.

**Do not claim** this is a hosted, public testnet, custody-audited or EVM non-bypassable demonstration until those things are independently proven.
