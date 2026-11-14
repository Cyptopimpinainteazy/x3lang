# NEAR Hackathon submission copy

**Judges: [start here — payment, limits, proof and reproduction](../JUDGES.md).**

**Project:** X3 Force Field (X3Lang; formerly X3 Shield)

**Tracks:** AI Security & Defense (`security`, primary), Agent Economy (`agents`), Autonomous Business (`business`) and Unstoppable DeFi (`defi`). Owner-confirmed selection; not a claim of organizer acceptance or a submission receipt.

## 20–2,000 character description (1780 characters, LF)

AI agents are getting wallets, and their spending limits should not live inside the model. In May 2026 an attacker drained about $150,000 from an AI agent's wallet in two moves; no key was stolen, because the model's output was the authorization.

X3 Force Field takes authorization out of the model. The owner writes spending rules as a small typed .x3 mandate: who may be paid, in which asset, per-payment and daily caps, slippage and fee ceilings, refund address, approval threshold and expiry. The agent can only propose payments, as .x3 programs. A compiler and deterministic checks refuse unsafe proposals outside the model: locally, 9/9 unsafe edits fail to compile and 3/3 replayed attack payments are refused while the owner's normal payment is allowed.

How NEAR fits: the x3-guard contract on NEAR mainnet verifies the NEAR Intents / 1Click quote's Ed25519 signature, then checks recipient, asset, minimum output, refund address and deadline against the owner's mandate before authorizing a deposit address. The agent signs with a FunctionCall key limited to three Guard methods.

Mainnet proof (invoice_06): after owner approval, the agent's restricted key submitted a signed quote and Guard accepted it; the owner's Solana wallet sent 25 USDC and 24.9952 USDC reached x3lang.near through NEAR Intents about 84 seconds later; the agent recorded the outcome. Both original .x3 files hash-match Guard state. JUDGES.md maps every claim to receipts and one-command checks.

Limits: one owner-funded ~$25 payment. Guard does not hold funds or authenticate the outcome the agent reports, the agent account keeps an owner FullAccess recovery key, and there is no cryptographic cross-chain atomicity. The X3 compiler/VM predate the hackathon; the NEAR integration is new work.

## Public links

Reproduction: [one pinned commit, setup and expected outputs](../JUDGES.md#reproduce-the-same-payments-checks--offline-after-setup).
Nine capabilities = six core payment claims + three additional local capabilities.
Video/X links below remain pending; a recording script is not a published demo.

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

## Additional implemented innovations

Beyond the six main payment differentiators, the [additional-innovation matrix](../JUDGES.md#additional-innovations--separate-local-evidence)
links proof for signed child admission and budget reservations, durable retirement
and signed incident review, and real local NEAR Sandbox vault execution.
All three also appear in [judge-claims.json](judge-claims.json). These are local
capabilities; no public-chain custody, independent AI jury or automatic policy
rewrite is claimed. They are separate from invoice_06.

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
