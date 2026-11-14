# X3 Force Field — owner-published X demo draft

**Previously X3 Shield.** Draft only. Attach the actual public demo video or screenshots before posting. No public post or public video URL has been verified. The owner publishes the post; these drafts do not authorize posting.

## Post 1 — video post

Built at NEAR HACKATHON: X3 Force Field puts AI-agent spending rules in .x3. Owner approves the exact quote; an agent executes a 5-USDC Solana vault payment; 4.999746 USDC arrives on NEAR. Policy and receipts, not model promises. @NEARProtocol #NEARHackathon

## Post 2 — reply: separate Guard proof

NEAR HACKATHON proof: invoice_06 used a restricted NEAR key for 21 Guard quote checks. The owner funded 25 USDC; 24.9952 reached NEAR. Today's 5-USDC vault transfer was agent-signed and made no new Guard call. @NEARProtocol #NEARHackathon

## Post 3 — reply: inspect and reproduce

NEAR HACKATHON: inspect the signed quote, agent-signed vault transfer, finalized NEAR payout and consumed approval. Reproduce offline checks and tamper tests: https://github.com/Cyptopimpinainteazy/x3lang @NEARProtocol #NEARHackathon

## Post 4 — reply: limits

NEAR HACKATHON limits: owner approves each exact quote; recovery/upgrade authority stays trusted. One RPC per chain for the vault pilot; provider linkage is not atomic cross-chain proof. No external audit or on-chain X3VM. @NEARProtocol #NEARHackathon

## Video storyboard (about 2 minutes 30 seconds)

1. 0:00–0:15: State the problem: an agent should spend within owner-set rules.
2. 0:15–0:40: Show a real .x3 policy and an unsafe payment refused with its reason. Label LOCAL; show a permitted control.
3. 0:40–1:00: Show invoice_06 as the separate NEAR Guard proof: owner approval, 21 signed-quote checks and restricted key. State that the owner signed that 25-USDC source deposit.
4. 1:00–1:50: Show the November 14 vault policy, exact owner quote approval, agent-signed 5-USDC source event/SPL CPI, and finalized 4.999746-USDC NEAR payout. Label MAINNET. Show the consumed nonce and empty vault; no new NEAR Guard call is claimed for this pilot.
5. 1:50–2:15: Explain owner approval and recovery/upgrade trust, one RPC per chain, provider linkage rather than atomic cross-chain proof, no external audit and no on-chain X3VM. Keep the detailed receipt-verifier deadline limitation linked in the proof bundle.
6. 2:15–2:30: Show the pinned public repository commit, proof bundle and offline verification command with its solders dependency. End on the public proof links.

Never display private keys or credentials. Verify the published video and X post while signed out. A local recording and these draft texts are not evidence of a published post.
