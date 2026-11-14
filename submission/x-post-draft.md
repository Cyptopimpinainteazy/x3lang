# X3 Force Field — owner-published X demo draft

**Previously X3 Shield.** Draft only. Attach the demo video before posting. Do not claim a post exists until its signed-out URL is verified.

Written to the organizer's demo-post advice: lead with the user problem, show one complete moment, explain NEAR's role concretely, then give a next step and ask a genuine question.

## Post 1 — video post

In May an AI agent was talked into sending ~$150K to an attacker. The model's output was the authorization.

X3 Force Field takes the spending rules out of the model. Watch an agent fall for that attack and get refused.

Built at NEAR HACKATHON @NEARProtocol #NEARHackathon

## Post 2 — reply: how NEAR fits

The agent signs with a key that can call only 3 methods on our NEAR contract, which verifies the NEAR Intents quote's signature and checks its terms against the owner's rules. On mainnet, the owner's Solana wallet then sent 25 USDC; 24.9952 USDC arrived on NEAR 84s later.

## Post 3 — reply: try it

Try it without a wallet or keys: one command replays the attack, and the scripted model falls for every step. 3/3 attack payments refused, the owner's normal payment allowed. Code, proofs and every mainnet transaction: https://github.com/Cyptopimpinainteazy/x3lang

## Post 4 — reply: limits and a question

Limits, plainly: one owner-funded ~$25 payment; the contract doesn't hold funds or verify the outcome the agent reports; no atomic cross-chain proof.

Builders: what's the one spending rule you'd want enforced before giving your agent a wallet?

## Video storyboard (about 2 minutes, captions only)

1. 0:00–0:10 — The problem: AI agents hold wallets; the May 2026 drain; the model's output was the authorization.
2. 0:10–0:45 — LOCAL, real run: `replay_grok_wallet_drain.py`. The scripted model falls for both attack steps; 3/3 attack programs are refused for their own reason; the owner's payment is allowed.
3. 0:45–1:00 — LOCAL, real run: `x3_wont_compile.py`. 9/9 unsafe edits fail to compile.
4. 1:00–1:45 — MAINNET: invoice_06 on the explorers. Owner approval → restricted agent key accepts the signed 1Click quote → the owner's wallet sends 25 USDC on Solana → `intents.near` pays 24.9952 USDC to `x3lang.near` → the agent records the outcome.
5. 1:45–1:55 — How NEAR fits: a FunctionCall key limited to three Guard methods; Guard verifies the quote signature and terms; NEAR Intents settles Solana→NEAR.
6. 1:55–2:10 — The limits on one card, the repository link and the question from Post 4.

Never display private keys or credentials. Verify the published video and X post while signed out.
