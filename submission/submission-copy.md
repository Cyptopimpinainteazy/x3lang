# NEAR Hackathon submission copy

**Judges: [start here — payment, limits, proof and reproduction](../JUDGES.md).**

**Project:** X3 Force Field (X3Lang; formerly X3 Shield)

**Tracks to select on submission form:** AI Security & Defense (`security`, primary), Agent Economy (`agents`); Unstoppable DeFi (`defi`) if selected as third track. Not a claim of organizer acceptance.

## 20–2,000 character description (1923 characters, LF)

X3 Force Field (formerly X3 Shield) combines typed .x3 owner policy, deterministic checks and signed-quote enforcement for AI-agent payments.

Owners write reviewable mandates; agents propose payment programs. Compiler and mandate checks reject unsafe proposals outside the model. NEAR x3-guard verifies signed NEAR Intents / 1Click terms before authorizing a deposit address. The model cannot change the stored mandate.

Six differentiators: typed, hashable policy; checks outside the model; authenticated quote-term enforcement; source-to-payment traceability; restricted agent-key methods; reproducible evidence. JUDGES.md maps each claim to source, receipts, commands and limits. submission/judge-claims.json provides the same evidence paths and source hashes for automated review; invoice06_chain.py --json reproduces the archived five-link trail.

Mainnet invoice_06: following owner approval, a restricted key accepted a quote after 21 Guard checks. The mandate capped each payment and daily spend at 25 USDC, restricted the recipient and pinned refunds. The policy floor was 24.75 USDC. The owner manually sent 25 USDC on Solana; 24.9952 USDC reached x3lang.near about 84 seconds later. The agent recorded Settled and floor_met. Both original .x3 hashes match Guard state; compiled output matches 10 policy fields and 13 mandate terms. Eight unsafe edits of that payment are refused locally.

Other local evidence: 3/3 scripted drain-pattern attacks refused with the authorized control allowed; 9 unsafe edits fail compilation. Historical testnet Guard evidence includes attacker.near refusal.

Limits: one owner-assisted ~$25 run. Guard is noncustodial and does not authenticate outcome hashes. The agent account retains an owner FullAccess recovery key. Provider association is not cryptographic cross-chain atomicity; no native X3VM runs on-chain. Compiler/VM predate the hackathon; NEAR integration is new work.

## Public links

The public video and X post are owner-published. Replace the placeholders only after verifying signed-out URLs.

- Repository: https://github.com/Cyptopimpinainteazy/x3lang
- Testnet evidence: `docs/hackathon/GUARD_TESTNET_EVIDENCE.md`
- Judge proof card: [payment, limits and transaction receipts](../PROOF_CARD.md)
- Demo script: [three-minute walkthrough](../docs/hackathon/VIDEO_WALKTHROUGH.md)
- Offline payment-to-proof verifier: `python3 scripts/hackathon/invoice06_chain.py`
- Mainnet evidence: `evidence/near-guard-mainnet-invoice06-2026-11-12/README.md`
- Exact `.x3` sources: `evidence/near-guard-mainnet-invoice06-2026-11-12/source-recovery/README.md`
- Guard outcome: `evidence/near-guard-mainnet-invoice06-2026-11-12/outcome-recording/README.md`
- Testnet mandate lifecycle:
  https://github.com/Cyptopimpinainteazy/x3lang/blob/master/evidence/near-guard-testnet-live-2026-11-06.md
- Authenticated light-client design and limits:
  https://github.com/Cyptopimpinainteazy/x3lang/blob/master/docs/security/NEAR_AUTHENTICATED_LIGHT_CLIENT.md
- Judge path: README "Try it: do X, see Y"; all gates: `bash scripts/hackathon/judge_one_command.sh /tmp/x3-shield-judge`
- Demo video: OWNER TO ATTACH PUBLIC URL AFTER RECORDING
- Public X post: OWNER TO ATTACH PUBLIC URL AFTER POSTING

## Differentiators and evidence for reviewers

Use the [six-claim proof matrix](../JUDGES.md#what-is-distinctive--claim-proof-and-boundary)
for why each capability matters, direct proof and its limits. The
[machine-readable index](judge-claims.json) lists evidence paths, levels and source
hashes. Run `python3 scripts/hackathon/invoice06_chain.py --json` after the
[judge-page setup](../JUDGES.md#reproduce-the-same-payments-checks--offline-after-setup).
These supporting links are outside the form-description character count.

## Exact transaction anchors

- Deploy: `FJ5voQaArx17QV4TzDLYDq93cuoAVLSRxjcxbswuyE8v`
- Official-dry recipient-mismatch failure:
  `3Q7qTZqiRBB6EuhN78Ra5Q5NBqPXDhfuCg2FKW8WoU2q`
- Official-live allowed acceptance:
  `6B6k5kre4mFoBq7RuJNadQgrhEvmgb9WDLepX22CxzUZ`
- Live mandate-recipient refusal:
  `AhVBD7oXfwsixBipupjCzuhNdqunjdXJnLz3LXnmfnN8`
- Live allowed mandate control:
  `DPJfcRUGk56h8sSsFsDFFgUMQAiyBCtpMuvquF9FvTct`
- Owner mandate creation: `J4WrxJTNhL5Z841thsvpmMCgsMWwrmUsrmrwHSMu9tXX`
- Revocation: `7aVonMiRCDyetbkpRBK6GKnL2X1XrQTPsf3Pk1gKGzp8`
- Two-chain trade, Base Sepolia source:
  `0x46c21dc305a9271c4e74c6d0734fb50f847a1e53d3cad87976133d66fd6728d9`
- Two-chain trade, NEAR testnet payout:
  `EBEBxJNgV9U5tBFBLj18ZzukgArqwXH5rBsBStFyCm1d`
- Mainnet invoice_06, owner approval: `GFdQLrzT5tnPPdZwKLtmwpNvSwrxVHDvVg2C3WX5fsK6`
- Mainnet invoice_06, agent `agent_accept_quote` (restricted key):
  `Eg75uehwU4YJ86ZZPEPKiBqnjkr6PxwPmV23HFAgu153`
- Mainnet invoice_06, Solana deposit:
  `4zhGYyWNabYQ5gixTUzoVHVbgFKWAsQssSpCv3B1J1LjCRisEJW6wm3zuS2GVuDotAT93S8wm1qezHwsKX2RkXo5`
- Mainnet invoice_06, recorded outcome: `9sTn9pJ1ZbCVazBm9Wz9oEDh6aWw4FUVMoAJEKCCzuqE`
- Mainnet invoice_06, NEAR payout: `8jfbACLSxrzi3fnajfAVGc4Fgo6XLCWA1mpiJWXPw43P`

## Owner-only fields

Leave the form's `website` field empty; HACKATHON.md says a non-empty value
rejects the submission. The human must supply lead name, contact email, team size, mainnet NEAR payout
account, public video/X links, and organizer-contact consent. Do not fabricate
or infer these fields. Do not submit the form until the owner confirms the
complete payload.
