# NEAR Hackathon submission copy

**Project:** X3 Shield (X3Lang)

**Tracks:** AI Security & Defense (`security`), Agent Economy (`agents`), Unstoppable DeFi (`defi`)

## 20–2,000 character description

In May 2026 an attacker drained ~$150K from Grok's AI-agent wallet: a gifted NFT lifted its limits, then a Morse-coded order made the agent send funds. The model's output was the authorization. X3 Shield moves authorization out of the model.

The owner writes spending rules in .x3, a small typed language: recipients, asset, per-payment and daily caps, slippage and fee ceilings, refund address, approvals, expiry. The agent can only propose: each payment is an .x3 program. A deterministic compiler and a NEAR contract (x3-guard) check it, and the signed 1Click quote, against the mandate. Nothing the agent holds, reads or decodes changes the rules.

What runs today, on testnet or locally:

1. Replay of that drain with a model that falls for every step: 3/3 attack programs refused, owner's payment allowed. Separately, 8/8 scripted attacks refused; 9 unsafe edits fail to compile.

2. On-chain refusals: the deployed Guard refused an agent intent to attacker.near and accepted the permitted control. It verified an official 1Click quote's Ed25519 signature and refused it on recipient mismatch; a matching live quote was accepted and stopped before deposit.

3. One .x3 program specified a cross-chain trade: its hash was bound on Guard, then 1.000000 test USDC left Base Sepolia and 0.997400 arrived on NEAR testnet, both finalized. The link is a trusted testnet relay, not atomic settlement.

Who it's for: teams shipping paying agents, wallets and Intents integrators, and owners delegating spend who need limits they can audit and revoke.

The README has "do X, see Y" steps; most run offline with Python and Node, no wallet or key.

Limits: Guard does not custody funds, so an unrestricted wallet key can bypass it. No funded official 1Click settlement or cryptographic atomicity. The X3 compiler and VM predate the hackathon and are disclosed; the NEAR integration is the hackathon work.

## Public links

- Repository: https://github.com/Cyptopimpinainteazy/x3lang
- Testnet evidence: `docs/hackathon/GUARD_TESTNET_EVIDENCE.md`
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

## Owner-only fields

Leave the form's `website` field empty; HACKATHON.md says a non-empty value
rejects the submission. The human must supply lead name, contact email, team size, mainnet NEAR payout
account, public video/X links, and organizer-contact consent. Do not fabricate
or infer these fields. Do not submit the form until the owner confirms the
complete payload.
