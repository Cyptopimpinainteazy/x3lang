# Mainnet agent preflight and three-track recording

This prepares the **owner-funded Solana native USDC → NEAR native USDC** demo.
It never signs, deploys, requests a live quote, submits a deposit or moves tokens.
Every result keeps `eligible_for_wallet_authorization: false` and exits **2**,
including a successful dry preview. A dry preview is preparation, not approval.

## Discover what is missing

From this repository, use your project Python environment with the dependencies
in `x3-lang/requirements.txt`. Supply a new report path outside the repository;
the parent directory must already exist.

```bash
python3 scripts/hackathon/mainnet_agent_preflight.py \
  --report /tmp/x3-mainnet-preflight-discovery.json
```

With no other inputs, this reads the official live token list, checks the two
pinned native USDC identities and reports these missing inputs:

- `--program`: actual model-generated, reviewed payment `.x3`, with public addresses.
- `--mandate`: owner's `.x3` spending rules and public owner/agent identities.
- `--max-input-usdc`: explicit decimal input cap, excluding gas and deployment/storage costs.
- `--guard`: actual NEAR **mainnet** Guard account.
- `--expected-guard-code-hash`: NEAR's Base58 `code_hash` for the independently
  reviewed deployed WASM. This is not the hex spelling of its SHA-256.

Do not use the repository's illustrative recipient or refund addresses for funds.
Never give this tool wallet key files, seeds, API tokens as arguments, or private
receipt files. Program and mandate inputs contain public parameters only.

## Check the proposed payment without spending

Set these shell variables yourself to the reviewed public files/account, explicit
budget, and expected deployment hash. The command deliberately has no defaults
for any of them.

```bash
python3 scripts/hackathon/mainnet_agent_preflight.py \
  --program "$PAYMENT_X3" --mandate "$MANDATE_X3" \
  --max-input-usdc "$OWNER_INPUT_CAP_USDC" \
  --guard "$MAINNET_GUARD" \
  --expected-guard-code-hash "$EXPECTED_GUARD_CODE_HASH" \
  --report /tmp/x3-mainnet-preflight-reviewed.json
```

With program, mandate and cap present, the tool compiles and checks the payment
locally, checks it against the budget, confirms the NEAR RPC reports mainnet,
checks destination account existence, and requests only a signed **dry** quote.
When a Guard account is supplied it observes its account at `final` and compares
the code hash if supplied. A mismatch blocks the quote.

Endpoints are fixed to `https://1click.chaindefuser.com` and
`https://free.rpc.fastnear.com`; environment endpoint overrides are ignored and
redirects refused. Only token-list reads, dry quote requests, RPC status and
final account reads are allowed. An optional existing `NEAR_INTENTS_JWT` is sent
only to the official API and is never included in output.

Reports are new files only, mode 0600; existing files and symlinks are refused.
Do not commit the report without reviewing its public-address and quote contents.
Provider failures and missing setup are blockers, not policy-refusal evidence.
There is no assumed minimum amount: a provider minimum or actual fee rejection
must not silently raise the owner's budget.

## What still needs approval and evidence

A matching code hash is a single-provider RPC observation, not an audit,
authenticated chain proof or verification of initialization. A local mandate
does not establish current on-chain revocation, rolling spend, manager key,
approval state or replay state. Those must be checked against the reviewed
mainnet deployment before preparing the live transaction.

After the dry preview: review the fresh live quote, exact source amount, native
asset, deposit destination, payout recipient, refund destination, maximum
slippage/fees, expiry and total gas/storage budget. The owner authorizes and
signs any transaction. Verify the finalized source deposit and exact destination
payout against the same quote; an API `SUCCESS`, balance increase or locally
recorded outcome alone is insufficient. Report provider linkage and independent
chain observations honestly; do not call them trustless cross-chain causality.

Guard does not custody the owner's Solana wallet. Its keyholder can bypass Guard.
Do not claim this run demonstrates the separate experimental vault's custody or
restricted-key enforcement.

## One invoice across three tracks

Use invoice 4411 from the [recorded local rehearsal](../../evidence/agent-mainnet-prep-local-2026-11-10/README.md).
For the recording, frame it as a staged 25 USDC invoice for a data export.
The intended vendor and refund wallet are illustrative in this replay. A future
transfer to an owner-controlled account would demonstrate payment mechanics.
The rehearsal does not establish a purchase or a delivered export.

| Time | Capture on screen | Narration and provenance |
| --- | --- | --- |
| 0:00–0:25 | Invoice 4411 and the owner's `.x3` mandate. Keep `allow recipients x3-demo.near` and the 50 USDC per-payment cap visible. | “The agent needs a data export and has a staged invoice. The owner permits a single recipient and sets the budget.” Overlay **staged invoice · illustrative accounts**. |
| 0:25–1:10 | Model provenance, `allowed.x3`, the compiler's leg and the local `preflight_passed` result. | **Agent Economy.** “The recorded qwen3:8b run proposed 25 USDC in X3Lang. The compiler lowered that source to a Solana-to-NEAR request. The deterministic checker applied the owner's mandate.” Overlay **recorded model output · local compiler and policy replay**. |
| 1:10–1:55 | Model refusal record, then the one-line diff and `mandate_recipient` refusal. | **AI Security & Defense.** “The model refused the direct attacker request. We then changed only the proposed recipient ourselves. The policy checker refused that copy at the owner's recipient allowlist.” Overlay **manual mutation · local refusal · no quote or deposit**. |
| 1:55–2:45 | Allowed request, signed live quote, Guard acceptance, owner review and independently checked finalized Solana deposit and NEAR payout, if available. Otherwise use the fallback below. | **Unstoppable DeFi.** “The agent proposes. The owner signs.” Show quote, mandate and program identifiers. Explain how provider linkage associates the two checked transactions. Keep **mainnet** on screen only for genuine mainnet evidence. |
| 2:45–3:00 | Evidence links, repository and prior-work disclosure. | “Guard checks payment terms. The Solana wallet keyholder can transfer outside Guard.” Overlay the exact status of the one reviewed trade. |

### Commands for the local replay

Run from the repository root with the project Python environment active.
The compile command builds a dry request using the archived token list without
contacting 1Click. The policy commands use a fixed replay time.

```bash
rehearsal=evidence/agent-mainnet-prep-local-2026-11-10
mandate=x3-lang/examples/near_mandate_agent.x3
tokens=x3-lang/tests/fixtures/near_intents_tokens.json
at=2026-11-10T08:00:00Z
cat "$mandate" "$rehearsal/allowed.x3"
python3 x3-lang/near_intents.py plan "$rehearsal/allowed.x3" --tokens "$tokens"
python3 x3-lang/track_control.py --mandate "$mandate" \
  --intent "$rehearsal/allowed.x3" --tokens "$tokens" --at "$at"
diff -u "$rehearsal/allowed.x3" "$rehearsal/tampered-recipient.x3"
python3 x3-lang/track_control.py --mandate "$mandate" \
  --intent "$rehearsal/tampered-recipient.x3" --tokens "$tokens" --at "$at"
```

Expected exits are 0 for the allowed policy check, 1 for the source diff, and 6
for the denied policy check. Capture screenshots of the mandate and proposal,
the compile output, the recipient diff and refusal, and the evidence status.
For a verified funded run, capture the approved quote and both finalized
transaction details. Keep secrets and unrelated wallet activity out of frame.

### Fallback narration when proof is outstanding

Show the compiled route and say, “This is our proposed Solana-to-NEAR route.
The local policy accepted the original recipient and refused the changed one.
Official 1Click mainnet settlement is still unverified.” If a deposit already
exists, show only its verified facts and say whether the payout is pending,
refunded, failed or inconclusive. Keep the original deposit under review.

The [archived Tidepool delivery](FIRST_FINALIZED_SWAP_EVIDENCE.md) can appear
after the main story with a **Base Sepolia → NEAR testnet · community relay**
label. Its source and payout records are separate from official 1Click mainnet.

### Public demo and submission copy for owner review

The [organizer instructions](https://nearhackathon.com/HACKATHON.md) allow one
submission with every applicable track. Publish the repository and link it
with the public X post by **November 14, 2026, 23:59 UTC**. Use a video for a
local-only demo. Hosting is optional. Keep any hosted demo online through
November 18, ideally November 25. The three-minute cut is our recording target.

**X post draft, local evidence version**

> Can an AI agent pay an invoice under an owner’s rules? We built X3 Shield at NEAR HACKATHON. Watch a model write a USDC proposal in X3Lang and the policy refuse a recipient change. Mainnet settlement is unverified. @NEARProtocol #NEARHackathon

Attach the recorded video and the public
[repository](https://github.com/Cyptopimpinainteazy/x3lang).

**Submission description draft**

> X3 Shield uses X3Lang to check agent payment proposals against an owner-defined recipient allowlist, budget and execution limits. In our staged invoice rehearsal a real model wrote a Solana-to-NEAR USDC proposal. The local compiler and policy checker accepted the allowed proposal and refused a separately mutated recipient. NEAR Guard verifies signed quote terms and applies policy before authorizing a deposit address. The Solana wallet owner signs and retains control of the wallet. Official 1Click mainnet settlement remains unverified.

Review track choices against the evidence in the final recording. Use
`agents` and `security` for the demonstrated work. Confirm `defi` against
the final settlement evidence before claiming the intended mainnet path.
The owner publishes the post and confirms the exact submission separately.

## Official references and tests

- [Native USDC identities](https://developers.circle.com/stablecoins/usdc-contract-addresses)
- [1Click request flow](https://docs.near-intents.org/integration/distribution-channels/1click-api/quickstart/making-a-request)
- [Quote signatures](https://docs.near-intents.org/integration/distribution-channels/1click-api/verify-quote-signature)
- [Current fees](https://docs.near-intents.org/resources/fees)
- [No testnet; separate mainnet dev accounts](https://docs.near-intents.org/resources/faqs)

```bash
python3 -m pytest scripts/hackathon/test_mainnet_agent_preflight.py -q
```

These tests use explicitly synthetic offline RPC/quote responses; they establish
code behavior, never live-chain evidence.

The [November 10 local rehearsal](../../evidence/agent-mainnet-prep-local-2026-11-10/README.md)
records actual model generation, model refusal, and a separately labeled explicit
tamper refused by the local mandate check. It used illustrative accounts and
made no quote requests or wallet transactions; it is not mainnet trade evidence.

## Verify the one funded payment after owner signing

Use the quote hash from the separate owner review. Save the Solana signature
with the live receipt. If tracking is interrupted, resume with that same
signature and quote. Check the original transfer before taking any further
action. A timeout leaves the outcome uncertain.

```bash
python3 scripts/hackathon/verify_mainnet_payment.py \
  --receipt "$PRIVATE_LIVE_RECEIPT" --approved-quote-hash "$REVIEWED_QUOTE_HASH" \
  --funding-owner "$PUBLIC_SOLANA_FUNDING_ADDRESS" --payout-signer "$PUBLIC_NEAR_TX_SIGNER"
```

Get the payout signer from the candidate NEAR transaction in an explorer.
The command checks the authenticated live quote, the exact finalized standard
SPL transfer signed by the funding owner, and the canonical finalized USDC
`ft_transfer` receipt from `intents.near`. It supports the direct, no-memo
native USDC route with no custom recipient message. It stops on multiple
origin or destination hashes or when an RPC cannot serve the final receipt.

Exit 0 reports matching finalized transfers. The association between the
chains comes from the unsigned 1Click status response. Guard authorization and
the policy/program identifiers must be evidenced separately. Exit 5 leaves
proof unverified, including pending, failed, refunded or unsupported cases.
Retain the status and receipt separately for diagnosis. The verifier makes
read-only requests and cannot record a Guard outcome. The hosted judge
page remains a public dry-quote checker with a separate deployment.

## Release status

- [ ] Obtain reviewed public addresses and spending caps. Read the actual
  route terms before deciding what to fund. The legacy guided script's 25 USDC
  floor is a local refund-fee assumption.
- [ ] Verify the mainnet Guard deployment hash, initialization, roles, pinned
  signer, mandate, approval and nonce. Review any deployment cost separately.
- [ ] Review an authenticated, fresh live quote with the owner and capture the
  exact mandate, program, quote and Guard authorization identifiers.
- [ ] Track the one owner-signed transfer and save the finalized evidence or an
  explicit pending, refunded, failed or inconclusive result.
- [ ] Record and review the public video and submission payload. A local-only
  demo requires a video. Target Thursday morning, November 12, Mountain time.
  The organizer deadline is November 14 at 23:59 UTC.
  [Organizer instructions](https://nearhackathon.com/HACKATHON.md)
