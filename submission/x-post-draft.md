# X post draft — owner publishes manually

Do not post automatically. Attach the public demo video or screenshots and
replace `REPOSITORY_URL` and `VIDEO_OR_SCREENSHOTS`.

I built X3 Shield at NEAR Hackathon. @NEARProtocol #NEARHackathon

AI agents can propose payments, but model output should not decide who gets
paid. X3Lang turns owner limits into an auditable mandate. The demo shows a
prompt-injected request to attacker.near refused at `mandate_recipient`, with
Python/browser parity.

On NEAR testnet, `x3lang-guard-20261106.testnet` verifies a pinned 1Click
manager signature before policy: an official manager-signed dry quote with a
recipient mismatch is refused after `quote_signature ok:true`; an official-live
allowed quote is accepted as a positive control, stopped before deposit.

No settlement, cross-chain causality, or live forbidden-quote refusal is
claimed.

Code + evidence: REPOSITORY_URL
Demo: VIDEO_OR_SCREENSHOTS
