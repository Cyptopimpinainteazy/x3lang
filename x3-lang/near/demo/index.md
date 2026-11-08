# X3 Guard

X3 Guard is the browser demo for X3 Shield. An owner can use X3Lang to describe who an agent may pay, how much it may spend, and the terms it must accept. The demo checks a signed NEAR Intents 1Click dry quote against those rules. It uses no wallet and moves no funds.

## Verify the evidence

- [Local policy evaluation](https://x3-guard.pages.dev/judge) shows how an owner mandate rejects an unlisted recipient. Its verdict comes from the browser.
- [NEAR testnet Guard decisions](https://github.com/Cyptopimpinainteazy/x3lang/blob/master/docs/hackathon/GUARD_TESTNET_EVIDENCE.md) record a signed 1Click dry quote refused for a mismatched recipient and other dry-quote checks. A separate accepted live quote stopped before deposit.
- [Finalized testnet settlement](https://github.com/Cyptopimpinainteazy/x3lang/blob/master/docs/hackathon/FIRST_FINALIZED_SWAP_EVIDENCE.md) records a separate 1.000000 test USDC transfer on Base Sepolia and 0.997400 test USDC received on NEAR testnet. The route relied on the trusted Tidepool relayer. The receipts show finalized transfers on each chain and leave cross-chain causality dependent on the relay.

The deployed Guard checks Guard-mediated actions. An unrestricted key with access to the funds can still transfer them directly.

## Resources

- [Interactive dry quote demo](https://x3-guard.pages.dev/)
- [X3Lang source](https://github.com/Cyptopimpinainteazy/x3lang)
- [Judge guide](https://github.com/Cyptopimpinainteazy/x3lang/blob/master/docs/hackathon/JUDGE_GUIDE.md)
