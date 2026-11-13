# NEAR Hackathon submission copy

**Project:** X3 Force Field (X3Lang; formerly X3 Shield)

**Tracks to select on submission form:** AI Security & Defense (`security`, primary), Agent Economy (`agents`); Unstoppable DeFi (`defi`) if selected as third track. Not a claim of organizer acceptance.

## 20–2,000 character description (1846 characters, LF)

X3 Force Field (formerly X3 Shield) is a programmable security boundary for AI-agent payments, powered by X3Lang, a typed .x3 language and compiler—not a wallet settings page.

Owners write reviewable .x3 mandates defining recipients, assets, per-payment and daily caps, approvals, fees, slippage, refunds and expiry. Agents propose .x3 payment programs; deterministic compiler/policy checks and the NEAR x3-guard contract enforce the rules against signed NEAR Intents / 1Click quote terms. The model cannot grant itself permission.

Mainnet invoice_06: the agent's restricted NEAR key accepted a signed quote after 21 Guard checks, following owner approval. Its mandate capped spend at 25 USDC per payment and per 24h, restricted the recipient to x3lang.near, pinned refunds, and bounded slippage/fees. The policy floor was 24.75 USDC. The owner manually deposited 25 USDC from Solflare on Solana; 24.9952 USDC reached x3lang.near about 84 seconds later. The agent recorded Settled and floor_met on NEAR. The offline invoice verifier rejects 8 unsafe edits of that same payment. Both original .x3 source files match the SHA-256 hashes stored by Guard.

Security evidence: 3/3 Grok-drain replay attacks refused locally while the authorized control passed; 8/8 scripted attacks refused; 9 unsafe payment edits failed compilation. Testnet Guard refused attacker.near; a separate Base Sepolia-to-NEAR testnet transfer delivered 0.9974 of 1 test USDC via a trusted relay.

Limits: one owner-assisted ~$25 mainnet settlement, not autonomous custody. Guard does not hold Solana funds or validate the agent-reported outcome transaction hashes; the agent account retains an owner FullAccess recovery key. No cryptographic cross-chain atomicity or native X3VM on-chain execution. X3Lang compiler/VM predate the hackathon; NEAR integration is the new work.

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
- Mainnet invoice_06, NEAR payout: `8jfbACLSxrzi3fnajfAVGc4Fgo6XLCWA1mpiJWXPw43P`

## Owner-only fields

Leave the form's `website` field empty; HACKATHON.md says a non-empty value
rejects the submission. The human must supply lead name, contact email, team size, mainnet NEAR payout
account, public video/X links, and organizer-contact consent. Do not fabricate
or infer these fields. Do not submit the form until the owner confirms the
complete payload.
