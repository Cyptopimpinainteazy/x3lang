# X3Lang track control: first vertical slice

## Competition framing

**.x3 is an owner-controlled, typed intent language for AI-agent payments, combining compiler checks with Guard enforcement of signed quote terms.**

An intent states the desired outcome. The owner’s `.x3` mandate sets the limits.
The compiler rejects invalid programs; deterministic mandate checks and NEAR Guard
refuse prohibited terms. NEAR Intents / 1Click supplies the supported execution route.
Read [the stage-by-stage explanation](X3_LANGUAGE.md#x3-intents-with-compiler-checks-and-guard-enforcement)
for compile-time versus runtime checks and the evidence boundaries.


**Scope:** the owner-selected tracks are AI Security & Defense (`security`, primary), Agent Economy (`agents`) and Unstoppable DeFi (`defi`). This does not certify organizer acceptance. One `.x3` owner mandate and one `.x3` payment policy supply
the local security and agent demonstrations; there is no new language syntax,
second policy authority, or separate track-specific transaction engine.
Historical Tidepool testnet delivery is assessed separately in
[DeFi evidence and gaps](DEFI_TRACK_EVIDENCE_AND_GAPS.md).

## Reproduce the local policy preview

From the repo root, with Python dependencies in `x3-lang/requirements.txt` installed:

```sh
PYTHONPATH=x3-lang python3 x3-lang/track_control.py \
  --mandate x3-lang/examples/near_mandate_agent.x3 \
  --intent x3-lang/examples/near_intents_agent_payment.x3 \
  --tokens x3-lang/tests/fixtures/near_intents_tokens.json \
  --at 2026-11-03T03:00:00Z

PYTHONPATH=x3-lang python3 x3-lang/track_control.py \
  --mandate x3-lang/examples/near_mandate_agent.x3 \
  --intent x3-lang/examples/near_intents_agent_denied.x3 \
  --tokens x3-lang/tests/fixtures/near_intents_tokens.json \
  --at 2026-11-03T03:00:00Z

PYTHONPATH=x3-lang python3 -m pytest x3-lang/tests/test_track_control.py -q
```

The approved *local preflight* exits 0, while the forbidden payee exits 6. Invalid input
exits 2. `authorization=not_verified`, `wallet_enforced=false`, `quote_verified=false`, and
`settlement_proven=false` are unconditional in this tool. The token list is an archived local
fixture, not a live 1Click response. Neither command signs, reaches RPC, calls 1Click, deploys,
sends money, or consumes a nonce.

## Shared workflow

1. **AI Security & Defense:** the same Guard-parity mandate checks explicitly reject the
   forbidden recipient, amount, fee, slippage and expired mandate in the existing test suite.
2. **Agent Economy:** the agent's `.x3` payment references the owner's `.x3` spending mandate;
   amounts above the approval threshold are reported as `approval_required`, never authorized.
3. **DeFi perspective, proposed:** the local intent lowers to one NEAR Intents leg
   with source/destination assets, minimum receipt, slippage, refund address and
   timeout preserved. This preview does not reproduce the separate historical
   Base Sepolia → NEAR Tidepool transfer or establish official 1Click settlement.

This document describes the local policy slice only. Separately, the committed
testnet evidence contains finalized deployment, registration, refusal,
revocation and an allowed `accept_quote`; that acceptance authorizes a deposit
address only. The live `agent_register_intent` request to a forbidden recipient
is now recorded as a `mandate_recipient` refusal. A fresh live manager-signed
quote refused at final `agent_accept_quote` and funded official 1Click settlement
remain unproven. Separately, the [first-swap walkthrough](FIRST_SWAP_JUDGE_WALKTHROUGH.md)
records a historical funded Base Sepolia transfer and finalized NEAR testnet
delivery through the trusted Tidepool relay. That evidence does not establish
provider-independent cryptographic cross-chain causality or atomicity. Do not
promote local preflight results or official quote acceptance into those claims.

## Change isolation

The language grammar, compiler, VM and Guard state are unchanged. This adds a read-only
control-surface entrypoint and tests so one policy can be exercised across the
two evidence-backed hackathon categories without implementing separate products.
