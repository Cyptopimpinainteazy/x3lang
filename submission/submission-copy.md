# NEAR Hackathon submission copy

**Judges: [start here — payment, limits, proof and reproduction](../JUDGES.md).**

**Project:** X3 Force Field (X3Lang; formerly X3 Shield)

**Tracks:** AI Security & Defense (`security`, primary), Agent Economy (`agents`), Autonomous Business (`business`) and Unstoppable DeFi (`defi`). Owner-confirmed selection; not a claim of organizer acceptance or a submission receipt.

## 20–2,000 character description (1861 characters, LF)

X3 Force Field gives AI-agent payments typed owner policies and checks outside the model. Owners write reviewable .x3 mandates; agents propose payment programs. Compiler and deterministic checks reject unsafe proposals. NEAR x3-guard authenticates signed NEAR Intents / 1Click quote terms before authorizing a deposit address.

Two separate mainnet proofs show the boundaries. In invoice_06, an owner-approved restricted key accepted a quote after 21 Guard checks. The owner manually deposited 25 USDC on Solana; 24.9952 USDC reached x3lang.near, and the agent recorded the outcome on Guard. Original .x3 hashes match Guard state.

On November 14, the owner funded a separate Solana vault and approved one exact signed quote. A dedicated agent signed the 5-USDC transfer from the policy-PDA-owned token account; 4.999746 USDC reached x3lang.near, above the signed 4.974757 minimum. The vault enforced 5-USDC payment and rolling-day caps, nonce and expiry. Finalized source event/SPL CPI, NEAR token-transfer event and provider SUCCESS are archived. The approval was consumed. This pilot made no new NEAR Guard call; its operator-written .x3 intent checked quote terms off chain.

Judges can reproduce compiler refusals, local attack controls and archived receipts from JUDGES.md. The new vault archive has an offline verifier and tamper tests. Its links and limitations are also in submission/judge-claims.json.

Limits: two bounded pilots, not production reliability or an external security audit. Owners retain recovery/upgrade authority. The vault does not verify the provider signature on chain. One RPC per chain supported the new pilot; provider linkage is not cryptographic cross-chain atomicity. The generic receipt verifier's deadline mismatch is disclosed. Compiler/VM predate the hackathon; NEAR integration is new work. No native X3VM runs on chain.

## Public links

Reproduction: [one pinned commit, setup and expected outputs](../JUDGES.md#reproduce-the-same-payments-checks--offline-after-setup).
Six core payment claims and three additional local capabilities remain indexed separately from the new mainnet vault pilot.
Video/X links below remain pending; a recording script is not a published demo.

The public video and X post are owner-published. Replace the placeholders only after verifying signed-out URLs.

- Repository: https://github.com/Cyptopimpinainteazy/x3lang
- Testnet evidence: `docs/hackathon/GUARD_TESTNET_EVIDENCE.md`
- Judge proof card: [payment, limits and transaction receipts](../PROOF_CARD.md)
- Demo script: [three-minute walkthrough](../docs/hackathon/VIDEO_WALKTHROUGH.md)
- Offline payment-to-proof verifier: `python3 scripts/hackathon/invoice06_chain.py`
- Owner-approved, agent-executed 5-USDC vault proof: [November 14 mainnet pilot](../evidence/solana-vault-mainnet-2026-11-14/README.md)
- Offline vault-pilot verifier: `python3 evidence/solana-vault-mainnet-2026-11-14/verify_archived.py` (requires solders 0.26.0; setup in the proof README)
- Earlier NEAR Guard / owner-funded 25-USDC evidence: `evidence/near-guard-mainnet-invoice06-2026-11-12/README.md`
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

## New mainnet vault pilot — separate proof

The November 14 pilot used owner-approved source custody in the Solana vault, not a new NEAR Guard authorization. The owner signed deployment, setup/funding and exact quote approval; the dedicated agent signed execution. Its PDA-authorized SPL Token TransferChecked moved exactly 5 USDC to the quoted deposit account. The finalized NEAR token event paid 4.999746 USDC to x3lang.near. The policy caps payment and rolling 24-hour spend at 5 USDC; nonce 1 is consumed and the vault balance is zero.

The [proof bundle](../evidence/solana-vault-mainnet-2026-11-14/README.md) archives the signed quote, owner and agent transactions, source event/CPI, payout event, post-state and checksums. Source and destination observations each used one RPC. Provider SUCCESS links the two legs, without proving cryptographic solver causality or atomic settlement. The bundle discloses that the generic receipt verifier rejects the differing request/program deadlines; the separate vault commitment and direct chain verifiers were used instead.

- Dedicated Solana agent: `6R4nioW9tPke3PFuWLaQY3gRAwM5SMi1vbY925WJZtPS`
- Owner quote approval: [finalized on Solana](https://explorer.solana.com/tx/4ReKsqwxpwtyx2CNmah6UCkHWgb4GbiJjXGTZLwtHaMMBoWv8vqSki4XXzDrBmqgURJSX7A1fHAKm3jSf3fnWwkV)
- Agent execution: [5-USDC vault transfer](https://explorer.solana.com/tx/3Bu3eWLwiSLoTNHgLPCzn5JyTAUnfzbXJENQG36oEt7kZm9e1M4rRB5CtUZxG92gRaGmPvuFZUQ1EqBzkL9rdLBf)
- NEAR payout: [4.999746 USDC delivered](https://nearblocks.io/txns/CXoXgJ5FwcSZcyPDVu7PrNby7KULfY8bJxUBRT9fu4ia)

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
