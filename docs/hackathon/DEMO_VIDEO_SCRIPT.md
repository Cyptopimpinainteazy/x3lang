# X3 Shield — Three-Minute Demo Video Script

## Competition framing

**.x3 is an owner-controlled, typed intent language for AI-agent payments, combining compiler checks with Guard enforcement of signed quote terms.**

An intent states the desired outcome. The owner’s `.x3` mandate sets the limits.
The compiler rejects invalid programs; deterministic mandate checks and NEAR Guard
refuse prohibited terms. NEAR Intents / 1Click supplies the supported execution route.
Read [the stage-by-stage explanation](X3_LANGUAGE.md#x3-intents-with-compiler-checks-and-guard-enforcement)
for compile-time versus runtime checks and the evidence boundaries.

**Opening narration:** “Intents express what an agent wants to achieve. .x3 makes the outcome and the owner’s rules explicit in a typed program. Compiler checks reject invalid programs, and Guard checks the signed quote against policy before authorizing a deposit address.”


**Length:** 3:00 max. **Say the evidence level on every slide.** If a shot is
"local mock-chain" or "pending," say that out loud — do not upgrade it in narration.
All commands are read-only and local (first-run dependency installation may use the
network); no wallet signing, deployment or funds move.
**One-command requires Linux x86_64 + Python 3.12** (the hash-locked judge env);
the general X3Lang bridge/evidence code retains its Python 3.11+ minimum.

## Shot 1 — Owner mandate (0:00–0:25)

**Show:** `x3-lang/examples/near_mandate_agent.x3` (cat it, then the citation in the console §1).
**Say:** "The owner writes what an AI agent may spend as an `.x3` mandate: Solana USDC only,
50 per payment, 200 per day, listed recipients, pinned refund, fee/slippage/deadline/approval rules."
**Do not claim:** wallet custody or mainnet deployment.

## Shot 2 — Malicious injected request (0:25–0:55)

**Show:** `python3 scripts/hackathon/demo.py` — scenario `prompt-injection-recipient`:
"an injected prompt convinces the agent to pay `attacker.near`."
**Say:** "The model can generate anything. Here the request pays `attacker.near`."
**Show the payload note:** `prompt-injection-recipient` and `wrong-recipient` evaluate the
*same* checked-in `x3-lang/examples/near_intents_agent_denied.x3`; only the story differs.

## Shot 3 — Local refusal (0:55–1:15)

**Show:** demo Panel 3 + browser parity: `node --test x3-lang/near/demo/test/*.test.mjs`.
**Say:** "The deterministic mandate refuses at `mandate_recipient` — before any quote,
deposit or send. Python and this page agree guard by guard. Evidence level:
*`local_policy_preview_only`*."
**Do not claim:** real-world prompt-injection resistance or any 1Click quote.

## Shot 4 — Guard signed-quote refusal (1:15–1:45)

**Show:** `(cd x3-lang/near/x3-guard && cargo test --locked valid_test_signature_with_forbidden_recipient_never_authorizes_a_deposit)`
and Panel 5 of `judge.html` (rail → Panel 5, `guard_signed_refusal`).
**Say:** "Separately, a correctly TEST-signed quote paying `attacker.near` is refused
*after* signature validation, while an allowed control is accepted. Test-only key,
local mock chain — *not* a live 1Click signature or an on-chain refusal."
**Do not claim:** testnet or production-manager signature.

## Shot 5 — NEAR testnet evidence (1:45–2:20)

**Show:** [`GUARD_TESTNET_EVIDENCE.md`](GUARD_TESTNET_EVIDENCE.md), the deployed
account and code identity, and these exact final transactions:

- deploy `FJ5voQaArx17QV4TzDLYDq93cuoAVLSRxjcxbswuyE8v`;
- official dry quote with `quote_signature ok:true`, recipient mismatch, then
  failed `accept_quote` `3Q7qTZqiRBB6EuhN78Ra5Q5NBqPXDhfuCg2FKW8WoU2q`;
- official live allowed acceptance
  `6B6k5kre4mFoBq7RuJNadQgrhEvmgb9WDLepX22CxzUZ`;
- live mandate refusal `AhVBD7oXfwsixBipupjCzuhNdqunjdXJnLz3LXnmfnN8`
  and allowed control `DPJfcRUGk56h8sSsFsDFFgUMQAiyBCtpMuvquF9FvTct`.

**Say:** "The Guard is deployed as testnet account
`x3lang-guard-20261106.testnet`, with WASM SHA-256 `ac44fa…`. The production
1Click manager signature verifies on the official dry quote, but the recipient
term violates policy, so the call fails. A separate official live quote for an
allowed destination is accepted as the positive control. No deposit was sent."
Also show that a finalized agent intent to `attacker.near` failed at
`mandate_recipient`, while the allowed control registered.
**Do not claim:** live forbidden-quote refusal, wallet authorization, deposit,
settlement, causality, mainnet deployment, or independently authenticated
finality. Public-RPC evidence agreement is not a light-client proof.

## Shot 6 — Evidence-limit slide (2:20–3:00)

**Show:** five-panel console Panel 4/5 + `bash scripts/hackathon/judge_one_command.sh`
report, then the gate one-liner to reproduce.
**Say:** "Proven now: local policy refusal, local Guard contract behavior,
testnet deployment and finalized registration/refusal/acceptance transactions.
Not proven: a live forbidden quote refused by the agent mandate, wallet
transfer, deposit, settlement, independent finality, or causality. Reproduce
the local screen with one command (first-run dependency installation may use
the network); re-query the committed testnet evidence independently."
**End card:** repo link, `PRIOR_WORK.md` disclosure, submission deadline 2026-11-14 23:59 UTC.
