# X post draft — owner publishes manually

Do not post automatically. Attach the demo video to post 1 and replace
`REPOSITORY_URL` in post 4 with https://github.com/Cyptopimpinainteazy/x3lang.
Structure follows HACKATHON.md: the user's problem first, one complete moment,
what NEAR does, a next step. Every claim below is backed by a file in the repo.

## Post 1 (attach video; under 280 characters)

A prompt-injected agent pays whoever the prompt says.

X3 Shield: the owner writes spending rules in .x3, and a NEAR contract checks each agent payment against them.

On mainnet: 25 USDC left Solana, 24.9952 arrived on NEAR after 21 checks passed.

@NEARProtocol #NEARHackathon

## Post 2 (reply)

The agent's NEAR key can call only three Guard methods. It accepted a signed
1Click quote only after Guard checked recipient, asset, caps, slippage, fee,
refund address and owner approval. Then it recorded the outcome on-chain:
Settled, floor met. All transactions are public.

## Post 3 (reply)

A scripted local model replayed the May 2026 Grok wallet drain: 3/3 attack
programs refused, the owner's control passed policy. Separately on testnet,
Guard refused an agent payment to attacker.near.

## Post 4 (reply)

Limits: one ~$25 trade, funded by hand; Guard checks payments but doesn't hold
funds. Clone it and run the "do X, see Y" checks, no wallet needed:
REPOSITORY_URL

What rule would you want on your own agent's wallet?

## Video (2–3 minutes)

1. Problem: the Grok wallet drain, where the model's output was the
   authorization (20 s).
2. The owner's v4 mandate, `source-recovery/mandate-proposed.x3`, and its
   limits (30 s).
3. Run `python3 scripts/hackathon/replay_grok_wallet_drain.py`; hold on
   `3/3 attack programs refused`. Label the frame LOCAL (30 s).
4. invoice_06 on the explorers, in order: owner approval `GFdQLrzT…`, agent
   quote acceptance `Eg75uehw…`, Solana deposit `4zhGYyWN…`, NEAR payout
   `8jfbACLS…`, outcome `9sTn9pJ1…`. Label MAINNET (55 s).
5. Limits: one ~$25 trade, manual funding, no custody, owner recovery key on
   the agent account, outcome hashes not verified by Guard, no atomicity,
   X3VM not on-chain (30 s).
6. Public repo, exact commit, judge commands (15 s).

Label every frame LOCAL, TESTNET or MAINNET. Show no keys.
