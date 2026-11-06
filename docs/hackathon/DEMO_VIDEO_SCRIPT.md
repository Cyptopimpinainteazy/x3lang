# X3 Shield — Three-Minute Demo Video Script

**Length:** 3:00 max. **Say the evidence level on every slide.** If a shot is
"local mock-chain" or "pending," say that out loud — do not upgrade it in narration.
All commands are read-only and local (first-run dependency installation may use the
network); no wallet signing, deployment or funds move.

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

## Shot 5 — Testnet transaction (1:45–2:20) — PENDING, do not fabricate

**Show:** the one command's report (`/tmp/x3-shield-judge/evidence.md`) and the PENDING
matrix in `docs/hackathon/JUDGE_GUIDE.md`; if testnet evidence exists by demo time,
show *only* a real, linked transaction with its exact evidence level.
**Say (current state):** "No Guard testnet deployment and no on-chain valid-signature refusal exist
yet (#73 pending). Here is the readiness preflight and shopping list — no transaction link
is shown because none exists."
**Do not claim:** deployed, deployed-testnet, or settled. Never invent a hash or link.

## Shot 6 — Evidence-limit slide (2:20–3:00)

**Show:** five-panel console Panel 4/5 + `bash scripts/hackathon/judge_one_command.sh`
report, then the gate one-liner to reproduce.
**Say:** "Proven now: local policy refusal plus local Guard contract behavior. Explicitly
not proven: on-chain authorization, wallet transfer, observed transaction, finalized
settlement, proven causality. Reproduce locally with one command (first-run dependency installation may use the network); raise this ceiling
only when the testnet refusal exists and is independently re-queryable."
**End card:** repo link, `PRIOR_WORK.md` disclosure, submission deadline 2026-11-14 23:59 UTC.
