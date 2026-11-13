# X3 Force Field — owner-published X demo draft

**Previously X3 Shield.** Draft only. Attach the actual public demo video or screenshots before posting. Do not claim a post exists until its signed-out URL is verified.

## Post 1 — video post (254 characters)

Built at NEAR HACKATHON: X3 Force Field 🛡️ — a typed .x3 language + compiler for AI-agent spending rules. A NEAR Guard checks signed terms, not model promises. 25 USDC moved Solana→NEAR on mainnet under owner-approved limits. @NEARProtocol #NEARHackathon

## Post 2 — reply: why this differs

This isn't a wallet settings toggle. The owner writes auditable .x3 mandate code; a compiler rejects unsafe payment programs. Deterministic checks and NEAR x3-guard enforce signed quote terms. X3BC/X3VM exist as prior work—not as on-chain Guard execution.

## Post 3 — reply: inspect the proof

Mainnet invoice_06: 25 USDC deposited from the owner's Solflare wallet; 24.9952 reached x3lang.near. Restricted agent recorded Settled; original .x3 files match Guard-stored hashes. Source, payout and outcome receipts: https://github.com/Cyptopimpinainteazy/x3lang

## Post 4 — reply: limits and reproducibility

One owner-assisted ~$25 settlement; Guard does not custody Solana funds or authenticate the agent-reported outcome tx hashes. No atomic cross-chain proof; X3VM not executing in Guard. Reproduce the compiler refusal and adversarial tests from the README: https://github.com/Cyptopimpinainteazy/x3lang

## Video storyboard (2–3 minutes)

1. 0:00–0:20: Problem—model output must not be authorization.
2. 0:20–0:50: Show real v4 `mandate-proposed.x3`, the typed rules, and compiler pipeline.
3. 0:50–1:20: Demonstrate 9 unsafe edits fail compilation and the local Grok drain replay refuses 3/3 attacks. Label LOCAL.
4. 1:20–2:15: Show invoice_06 owner approval, restricted-agent quote acceptance, Solana deposit, NEAR payout and recorded Guard outcome with explorer links. Label MAINNET.
5. 2:15–2:45: Explain custody, manually signed funding, FullAccess recovery key, unauthenticated outcome hashes, no atomicity and no on-chain X3VM.
6. 2:45–3:00: Show the exact public repository commit and clean-clone judge commands.

Never display private keys or credentials. Verify the published video and X post while signed out.
