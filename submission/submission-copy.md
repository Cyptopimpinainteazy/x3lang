# NEAR Hackathon submission copy

**Project:** X3 Shield (X3Lang)

**Tracks:** AI Security & Defense (`security`), Agent Economy (`agents`), Unstoppable DeFi (`defi`)

## 20–2,000 character description

X3 Shield is a policy layer for AI-agent payments on NEAR. The owner writes spending rules in X3, a small typed language: who may be paid, in which asset, how much per payment and per day, slippage and fee ceilings, approval thresholds, expiry. The agent writes each payment as an .x3 program. A deterministic compiler and a NEAR contract (x3-guard) compare the two. Model output never authorizes anything.

What runs today, all on testnet or locally:

1. A cross-chain trade specified by one .x3 program. Its source hash was bound on a NEAR Guard contract first; then 1.000000 test USDC left Base Sepolia and 0.997400 arrived at x3lang.testnet on NEAR. Both transactions are finalized and public, and an offline verifier in the repo checks the archive. The link between the legs is a trusted Tidepool testnet relay, not atomic settlement.

2. On-chain refusals. The deployed Guard refused an agent intent to attacker.near (mandate_recipient) and accepted the permitted control. It also verified an official 1Click dry quote's Ed25519 signature; the quote was refused with recipient mismatch and separate dry-quote failures. A matching live quote was accepted and stopped before deposit.

3. The model is not the defence. A scripted model that complies with all 8 attacks gets 8 of 8 refused by deterministic checks. Nine unsafe edits to an agent payment fail to compile.

The README has ten "do X, see Y" steps; eight run offline with Python and Node, no wallet or key.

Limits: testnet only. Guard does not custody funds, so an unrestricted wallet key can bypass it. No funded official 1Click settlement, no mainnet, no cryptographic cross-chain atomicity. The X3 compiler and VM predate the hackathon and are disclosed as baseline; the NEAR integration is the hackathon work.

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
