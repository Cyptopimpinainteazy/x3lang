# NEAR Hackathon submission copy

**Project:** X3 Shield (X3Lang)

**Tracks:** AI Security & Defense (`security`), Agent Economy (`agents`), Unstoppable DeFi (`defi`)

## 20–2,000 character description

In May 2026 an attacker drained ~$150K from Grok's AI-agent wallet: a gifted NFT lifted its limits, then a Morse-coded order made the agent send funds. The model's output was the authorization. X3 Shield moves authorization out of the model.

The owner writes spending rules in .x3, a typed language: recipients, asset, per-payment and daily caps, slippage and fee ceilings, refund address, approvals, expiry. The agent can only propose: each payment is an .x3 program. A compiler and a NEAR contract (x3-guard) check it, and the signed 1Click quote, against the mandate. Nothing the agent holds, reads or decodes changes the rules.

Mainnet: an agent key limited to three Guard methods accepted a signed 1Click quote after 21 checks passed. The owner's wallet sent 25 USDC on Solana; 24.9952 USDC reached x3lang.near 84 s later. The agent then recorded the outcome on Guard: Settled, floor met. The .x3 payment and mandate files match the hashes Guard stored.

Testnet and local:
1. Guard refused an agent payment to attacker.near, accepted the permitted control, and refused a signed official 1Click quote on recipient mismatch.
2. The Grok drain replayed with a scripted model that falls for every step: 3/3 attack programs refused, owner's payment allowed. 8/8 scripted attacks refused; 9 unsafe edits fail to compile.
3. An .x3-specified Base Sepolia to NEAR testnet trade finalized (1.0 to 0.9974 test USDC) via a trusted relay.

For teams shipping paying agents and owners who need limits they can audit and revoke. The README's "do X, see Y" steps mostly run offline, no wallet or key.

Limits: one ~$25 mainnet trade, funded manually from the owner's wallet. Guard doesn't custody funds, so an unrestricted key (including the owner's recovery key on the agent account) can bypass it. Guard doesn't verify the outcome's tx hashes. No cryptographic cross-chain atomicity; X3VM doesn't run on-chain. The compiler and VM are pre-hackathon baseline; the NEAR integration is new.

## Public links

- Repository: https://github.com/Cyptopimpinainteazy/x3lang
- Testnet evidence: `docs/hackathon/GUARD_TESTNET_EVIDENCE.md`
- Mainnet evidence: `evidence/near-guard-mainnet-invoice06-2026-11-12/README.md`
- Mainnet `.x3` sources (hash-matched to Guard): `evidence/near-guard-mainnet-invoice06-2026-11-12/source-recovery/README.md`
- Mainnet outcome recording: `evidence/near-guard-mainnet-invoice06-2026-11-12/outcome-recording/README.md`
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
- Mainnet invoice_06, agent `agent_record_outcome` (restricted key, `Settled`):
  `9sTn9pJ1ZbCVazBm9Wz9oEDh6aWw4FUVMoAJEKCCzuqE`
- Mainnet invoice_06 sources: payment `.x3` SHA-256
  `9ee29a5603cfa1e7d9fdbd866cf330ce35e6b5f0907c6844dd2ef1a213768ba0`, mandate `.x3` SHA-256
  `eea021d5cc1118801f4e1d3b9c0efe89fc59a7f654031f9ff1951aa1f2f3d515`

## Owner-only fields

Leave the form's `website` field empty; HACKATHON.md says a non-empty value
rejects the submission. The human must supply lead name, contact email, team size, mainnet NEAR payout
account, public video/X links, and organizer-contact consent. Do not fabricate
or infer these fields. Do not submit the form until the owner confirms the
complete payload.
