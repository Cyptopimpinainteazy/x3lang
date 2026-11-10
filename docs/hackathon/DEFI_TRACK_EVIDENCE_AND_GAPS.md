# Unstoppable DeFi: evidence and judge path

This is a proposed third-track assessment, not a claim of entry, victory, or release readiness.

## Working demonstration

The repository documents an owner-authorized Tidepool community testnet route from Base Sepolia to NEAR testnet. A historical X3Lang Python frontend policy and NEAR Tidepool Guard authorization preceded a Base transfer of 1 test USDC; the recorded destination receipt credited 0.997400 test USDC, above the quoted minimum of 0.987426. See [first-swap walkthrough](FIRST_SWAP_JUDGE_WALKTHROUGH.md), [source transaction](https://sepolia.basescan.org/tx/0x46c21dc305a9271c4e74c6d0734fb50f847a1e53d3cad87976133d66fd6728d9), [destination transaction](https://testnet.nearblocks.io/txns/EBEBxJNgV9U5tBFBLj18ZzukgArqwXH5rBsBStFyCm1d), and [proof library](TWO_CHAIN_PROOF_LIBRARY.md).

The saved strict evidence is historical; rerun the documented archival verification and record the exact commit and result before submission. A matching finalized transfer on each chain does not independently establish cryptographic cross-chain causality. Tidepool is a trusted testnet relayer. This was not the newer scoped vault, a native X3VM bytecode execution, official NEAR Intents 1Click mainnet settlement, or a trustless atomic bridge. Do not reuse the historical quote, nonce or deposit address.

## Offline archive check on master

At master commit `2caa28e95014068e0cbd273552aa797b613cd870`, `PYTHONPATH=x3-lang:scripts/hackathon python3 scripts/hackathon/verify_first_swap_archive.py` returned PASS in `offline-signed-evidence` mode; the seven `test_first_swap_archive.py` unit tests passed. This checks saved signed evidence and internal relationships, not current chain state. A `--live` recheck was attempted but interrupted while waiting for RPC access; it yielded no result. Repeat both checks at the final submission commit and record independent RPC results before claiming fresh verification.

## Local judge demo check

At master `2caa28e95014068e0cbd273552aa797b613cd870`, `python3 scripts/hackathon/demo.py` exited 0: the allowed scenario passed local preflight, the threshold case required approval, and the wrong-recipient and prompt-injection scenarios were refused by `mandate_recipient`. This is an offline policy preview using archived token fixtures and scripted examples, not an AI run, signed quote, wallet authorization, or on-chain enforcement. `node --test x3-lang/near/demo/test/*.test.mjs` passed 88 tests, including checks of archived official quote signatures and tampering. These tests do not establish a fresh 1Click response. Python pytest checks were not run because pytest was unavailable in the environment. The final release head and actual browser/API flow still require verification.

## Track fit and remaining work

The [organizer's track description](https://nearhackathon.com/) emphasizes trading and cross-chain settlement through NEAR Intents. The Tidepool demonstration supports a real testnet cross-chain trading story, but does not by itself establish an official NEAR Intents settlement. The existing 1Click positive control stopped at deposit-address acceptance. Do not conflate the two.

1. Reproduce the historical proof against the exact release candidate using the documented archival procedure; retain the results and RPC limitations.
2. Demonstrate an actual official NEAR Intents route through deposit and destination settlement only if feasible with fresh owner-authorized testnet terms and independently verifiable receipts. Otherwise label this gap explicitly; do not present a preview or address acceptance as settlement.
3. Make a concise judge journey: policy terms and pretrade authorization; source transaction; destination receipt and balance change; trusted-relayer boundary; separately labeled local adverse cases. Verify each referenced test against its own exact head.
4. Reconcile README, submission copy, checklist and video script so the separate Tidepool trade and official 1Click demo are both accurately described. Select DeFi only if the owner elects to submit it after reviewing fit and evidence.
5. Verify the public repository, reproducible judge commands, public demo post and actual submission before the organizer deadline. Preserve prior-work disclosure and exact-head CI gates.

Winning cannot be guaranteed. Record any independent review, competing entries or official scoring rubric only when supported by current evidence.
