# Three-minute X3Lang walkthrough

Record from the repository root. Keep the terminal font readable and avoid showing environment
variables, key files or private receipts.

## 00:00–00:20 — The problem

**Show:** `python3 scripts/hackathon/demo.py`, stopped at Panel 1.

**Say:** “An AI agent can write a transaction, but an English request is not financial authority.
X3Lang turns the owner's limits into a policy that every proposed payment must satisfy. This one
mandate is reused for agent safety, agent commerce and cross-chain settlement.”

## 00:20–00:50 — The owner policy

**Show:** Panel 1 and highlight `per_payment`, `per_24h`, `allow recipients`, `approve above`,
`refund_to`, fees, slippage and expiry.

**Say:** “The owner controls the assets, recipients, amount, rolling spend, approval threshold,
refund destination and execution bounds. A model does not choose these terms.”

## 00:50–01:25 — Allowed request

**Show:** Panel 2's authorized program and Panel 3's `Passes the local policy check`.

**Say:** “This program pays 25 USDC to a listed recipient. X3Lang compiles it and evaluates the
same ten mandate guards in order. Every guard passes. This is a local preflight decision, not
wallet authorization.”

## 01:25–01:55 — Approval and refusal

**Show:** the 40 USDC scenario and the `attacker.near` scenario.

**Say:** “Forty USDC remains below the spending cap but crosses the owner's approval threshold, so
the result is approval required, never approved. A substituted recipient is refused with the exact
guard `mandate_recipient`.”

## 01:55–02:25 — Cross-chain path

**Show:** Panel 4 and then open `http://localhost:8000/judge.html`.

**Say:** “The same program lowers to one NEAR Intents leg from Solana USDC to NEAR USDC with a
fixed minimum receipt and refund address. The browser console evaluates the same scenarios through
the same guard order. Python and browser decisions are tested for parity.”

## 02:25–03:00 — Evidence and limits

**Show:** Panel 5 and the output of:

```bash
python3 scripts/hackathon/verify.sh
```

**Say:** “The verified claim stops at deterministic local policy plus the contract test suite.
There is no wallet signature, deployment, live quote, deposit, settlement or causality proof in
this demo. The readiness report names each missing level instead of turning a local pass into a
financial claim.”

## Recording checklist

- Show the commit SHA once: `git rev-parse HEAD`.
- Show the evidence command and final pass/fail counts.
- Do not edit terminal output.
- Do not show `.env`, key files, JWTs, private receipts or account secrets.
- Do not imply that `https://x3-guard.pages.dev` performed a transaction.
- Upload the video publicly and use it in the required X submission post.
