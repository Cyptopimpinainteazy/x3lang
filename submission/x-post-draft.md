# X post draft — owner publishes manually

Do not post automatically. Attach the demo video (or screenshots) to post 1 and
replace `REPOSITORY_URL`. Structure follows HACKATHON.md: the user's problem
first, one complete moment, what NEAR does, a next step.

## Post 1 (attach video; fits 280 characters)

A prompt-injected agent can request a payment to an attacker.

X3 Shield: owners write .x3 rules. NEAR Guard checks requests before authorizing a deposit address.

Watch an attacker.near intent get refused on testnet.

Built at NEAR HACKATHON @NEARProtocol #NEARHackathon

## Post 2 (reply)

The same language specified a cross-chain trade. One .x3 program, hash bound on
a NEAR Guard contract first, then 1.000000 test USDC left Base Sepolia and
0.997400 arrived on NEAR testnet. Both transactions are public. Trusted testnet
relay, not atomic settlement.

## Post 3 (reply)

What NEAR does here: the Guard contract verifies the 1Click quote's Ed25519
signature on-chain, then checks the signed recipient, asset, minimum output and
refund address against the owner's policy. An official signed dry quote was refused;
the archived checks include recipient mismatch and separate dry-quote failures.

## Post 4 (reply)

Clone it and run the ten "do X, see Y" checks, no wallet needed:
REPOSITORY_URL

What rule would you want on your own agent's wallet that this can't express yet?

## Video: the one complete moment (60 to 90 seconds)

1. Show the 12-line owner mandate `.x3` (5 s).
2. Run `python3 scripts/hackathon/compromised_agent_redteam.py`; hold on
   `Model complied with 8/8 attacks; deterministic checks refused 8/8` (20 s).
3. Open the testnet refusal of `attacker.near` on the explorer (10 s).
4. Show the trade `.x3`, run `sha256sum`, then open the Base Sepolia and NEAR
   explorer pages: 1.000000 out, 0.997400 in (30 s).
5. End on the README "Try it" section (5 s).

Label every frame LOCAL or TESTNET. Show no keys.
