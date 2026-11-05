# X3Lang three-track control: first vertical slice

**Scope:** AI Security & Defense, Agent Economy, Unstoppable DeFi. One `.x3` owner mandate and
one `.x3` payment policy supply all three demonstrations; there is no new language syntax,
second policy authority, or separate track-specific transaction engine.

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
3. **Unstoppable DeFi:** the intent lowers to one NEAR Intents leg with source/destination
   assets, minimum receipt, slippage, refund address and timeout preserved.

This is the *first* vertical slice. The hackathon demonstration still requires an independently
verified on-chain authorization boundary, transaction/refusal evidence and real cross-chain
settlement. Do not promote local preflight evidence into such claims. Existing `near_intents.py`
`quote --live`, x3-guard and the PR #14 judge-evidence work remain separate release gates.

## Change isolation

The language grammar, compiler, VM and Guard state are unchanged. This adds a read-only
control-surface entrypoint and tests so one policy can be exercised across three applicable
hackathon categories without implementing three products.
