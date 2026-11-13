# Agent law: what is enforced today, and what is design

X3 Agent Law is the rule that an AI agent's authority comes from an owner-written,
versioned mandate, never from the agent's own words. This page maps the lifecycle,
birth → compile → test → authorize → execute → terminate → review → evolve, to what
exists in this repository today. Every row carries one label:

| Label | Meaning |
|---|---|
| **MAINNET** | Enforced by the deployed Guard on `x3lang.near` (code hash `BPdpJAw8R4SDh6Wi7dPcXTs5T7ncJDVKCLgzuqZFb15T`). |
| **OFFLINE** | Enforced by the compiler or the off-chain policy check before anything is sent, and run in CI. An agent runtime that skips them still meets Guard. |
| **CHECK ONLY** | A pure function with tests. No runtime calls it yet. |
| **DESIGN** | Not built. |

## The lifecycle

| Stage | What it means | Status | Where |
|---|---|---|---|
| Birth: mandate | The owner writes the agent's limits: recipients, assets, per-payment and 24-hour caps, refund address, approvals and expiry. Guard also stores the mandate source's SHA-256 as a record; it does not check intents against it (pinning payment programs is PR #202, not deployed). | **MAINNET** | `create_mandate` in [`mandate.rs`](../../x3-lang/near/x3-guard/src/mandate.rs). invoice_06 ran under mandate v4, whose source is [`mandate-proposed.x3`](../../evidence/near-guard-mainnet-invoice06-2026-11-12/source-recovery/mandate-proposed.x3). |
| Birth: identity | The agent is its own NEAR account with a FunctionCall key limited to `agent_register_intent`, `agent_accept_quote` and `agent_record_outcome` on `x3lang.near`. | **MAINNET** | The keys in the [proof card](../../PROOF_CARD.md#accounts-keys-and-code). The same account still holds the owner's FullAccess recovery key (limit 3). |
| Birth: child agents | A child cannot gain capabilities its parent lacks, exceed the parent's budget or lifetime, or spawn past the parent's limit; an Ed25519 operator approval binds the exact manifests. | **CHECK ONLY** | [`agent_constitution.py`](../../x3-lang/agent_constitution.py), [`agent_admission_signed.py`](../../x3-lang/agent_admission_signed.py), [`X3_AGENT_CONSTITUTION_V1.md`](../security/X3_AGENT_CONSTITUTION_V1.md). There is no identity registry and no atomic budget reservation. |
| Compile | A payment the chain cannot enforce does not compile: no slippage bound, no timeout or refund, no nonce, a second swap leg, an unenforceable finality promise, a look-alike account. | **OFFLINE** | [`x3_wont_compile.py`](../../scripts/hackathon/x3_wont_compile.py) (9/9). On the real mainnet file: [`invoice06_chain.py`](../../scripts/hackathon/invoice06_chain.py) (8/8 edits refused; the unedited files compile to exactly the policy and mandate Guard stores). |
| Test | A compliant-but-compromised model is run against the mandate before anything is sent. | **OFFLINE** | [`compromised_agent_redteam.py`](../../scripts/hackathon/compromised_agent_redteam.py) (8/8), [`replay_grok_wallet_drain.py`](../../scripts/hackathon/replay_grok_wallet_drain.py) (3/3). Gating a new agent's privileges on passing such a run is **DESIGN**. |
| Authorize | Every intent and quote is checked against the stored mandate; a 1Click quote must carry a valid manager signature and match the policy term by term. | **MAINNET** | `check_mandate` in `mandate.rs`; `verify_signed_quote` and `evaluate` in [`lib.rs`](../../x3-lang/near/x3-guard/src/lib.rs). invoice_06 returned 21 checks, all ok. |
| Execute | The agent cannot move value except through an authorized route. | **Gap** | Guard authorizes a deposit address; it does not hold or move funds. The funding wallet's key and the FullAccess keys are routes around it (proof card limits 1–3). A Guard-controlled vault as the only spend route is **DESIGN**. |
| Terminate | The owner revokes the mandate; the agent's next register, approve or accept fails at once. | **MAINNET** (code), shown in a unit test and on testnet | `revoke_mandate` in `mandate.rs`; unit test `revoking_stops_the_agent_at_once_and_replacing_voids_old_intents` (required `near-guard` CI job); testnet `revoke_mandate` [`7aVonMiR…`](https://testnet.nearblocks.io/txns/7aVonMiRCDyetbkpRBK6GKnL2X1XrQTPsf3Pk1gKGzp8), block 276547226, in [`transactions.json`](../../evidence/near-guard-testnet/transactions.json). A finalized refusal of an agent call after revocation is not yet recorded on chain. Stopping off-chain processes and preserving their state (Scrapyard) is **DESIGN**. |
| Review (court and jury) | Independent reviewers examine the preserved proposal, checks and receipts; the agent may explain itself but cannot authorize or acquit itself. | Evidence: **MAINNET** receipts and committed bundles. Court and jury: **DESIGN** | The [invoice_06 bundle](../../evidence/near-guard-mainnet-invoice06-2026-11-12/README.md). Refusals revert, so Guard stores no record of them; an on-chain refusal record is **DESIGN**. The agent's outcome report is stored as reported and not authenticated (limit 4). |
| Evolve | Rules change only as a new owner-signed mandate version; intents registered under an older version can no longer be accepted. | **MAINNET** | `create_mandate` increments the version; the unit test above shows the old intent refused. The agent's FunctionCall key cannot call `create_mandate` or `revoke_mandate`. Chaining each version to the previous one by hash is **DESIGN**. |

## Properties the deployed Guard already has

These hold on `x3lang.near` today, with no change:

1. **The quote key cannot be rotated.** The 1Click manager key is set from `ONE_CLICK_MANAGER_PUBLIC_KEY` in `new()`, `migrate()` copies it unchanged, and no method writes it.
2. **A valid signature is not permission.** Guard verifies the quote's signature, then still checks every term: a genuinely signed quote with the wrong recipient or refund address is refused (`quote_request_echo`, `mandate_recipient`, `mandate_refund_to`).
3. **The cross-chain refund address is pinned.** The quote's `refundTo` must equal the policy's, and the policy's must equal the mandate's.
4. **An approver cannot widen the mandate.** `approve_intent` adds one named approver's approval to one intent; it changes no term.
5. **The agent has nothing to talk with.** No agent method takes a reason, purpose or decision argument. Authority comes from the stored mandate and the signed quote.
6. **A rule the chain cannot enforce does not compile** (the Compile row above).

## Built, not deployed

[PR #202](https://github.com/Cyptopimpinainteazy/x3lang/pull/202) lets an owner pin the exact `.x3` payment programs an agent may run (`pin_mandate_programs`), so an agent cannot rewrite a payment and stay inside the caps. It changes the contract and is not deployed; the mainnet Guard is frozen until results.

## Provenance

The original ten-commandment constitution is Orchestra's Score in the xxxstar repository (`crates/orchestra/src/score.rs`), with the operational rules in `docs/swarm-governance/AGENT_LAW.md` there. Neither is compiled into Guard. This page describes what this repository enforces.

## Next, in order

1. Check the recorded outcome against both chains before marking it `Settled` (closes limit 4).
2. Move the recovery key off the agent account to the owner (closes limit 3).
3. Make a Guard-controlled vault the agent's only spend route (closes the Execute gap).
4. Store a refusal record, with the guard and program hash, for every refused call (gives review on-chain evidence).
5. Pin payment programs (PR #202) and chain mandate versions by hash.
6. The runtime registry, termination and Scrapyard for child agents.
