# NEAR Hackathon submission copy

**Project:** X3 Shield (X3Lang)

**Tracks:** AI Security & Defense (`security`), Agent Economy (`agents`)

## 20–2,000 character description

X3 Shield is a programmable transaction-policy layer for AI-agent payments.
An owner writes limits and allowed terms in `.x3`; the agent proposes a NEAR
Intents / 1Click payment; deterministic checks compare the signed quote terms
with that mandate before a Guard-mediated deposit route is accepted.

The judge demo shows a prompt-injected request to `attacker.near` refused by
`mandate_recipient`. Python and the browser evaluate the same checked-in denied
payload guard by guard. A separate local Guard contract test proves a
correctly TEST-signed quote is refused after signature verification.
A real local `qwen3:8b` red-team run held all 8 adversarial requests; seven
were refused by the model itself, and the eighth homoglyph case produced a safe
real-recipient payment neutralized by deterministic effect checks. Neither
half is presented as a model-failure-then-Guard-refusal proof.

NEAR testnet evidence: `x3lang-guard-20261106.testnet` deployed with WASM
SHA-256 `ac44fa53acf514d26c7adf883b31b49a125faf5a4c1074928d3e1fe94b36f8ab`
(on-chain code hash `CbU8SpgfTgG1egbw8ygMTK3deozvDnoFrF8DJ8qifSe6`). The
deployed Guard refused an official production-manager-signed dry quote after
`quote_signature ok:true` because its recipient did not match the registered
request. A separate official-live allowed quote was accepted as a positive
control; that acceptance authorizes a deposit address only. Both transactions
are finalized testnet observations.

The deployed `x3lang.testnet` Guard also ran the mandate lifecycle: a finalized
`agent_register_intent` to `attacker.near` failed at `mandate_recipient`, an
allowed control registered, and read-only state shows the mandate subsequently
revoked.

Limits: the live acceptance stopped before deposit. The repository does not
claim a live forbidden-quote refusal, wallet transfer, settlement, independent
NEAR finality, or cross-chain causality. Guard does not custody funds and
cannot stop an unrestricted wallet keyholder from bypassing it.

## Public links

- Repository: https://github.com/Cyptopimpinainteazy/x3lang
- Testnet evidence: `docs/hackathon/GUARD_TESTNET_EVIDENCE.md`
- Testnet mandate lifecycle:
  https://github.com/Cyptopimpinainteazy/x3lang/blob/master/evidence/near-guard-testnet-live-2026-11-06.md
- Authenticated light-client design and limits:
  https://github.com/Cyptopimpinainteazy/x3lang/blob/master/docs/security/NEAR_AUTHENTICATED_LIGHT_CLIENT.md
- Judge path: `bash scripts/hackathon/judge_one_command.sh /tmp/x3-shield-judge`
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

## Owner-only fields

The human must supply lead name, contact email, team size, mainnet NEAR payout
account, public video/X links, and organizer-contact consent. Do not fabricate
or infer these fields. Do not submit the form until the owner confirms the
complete payload.
