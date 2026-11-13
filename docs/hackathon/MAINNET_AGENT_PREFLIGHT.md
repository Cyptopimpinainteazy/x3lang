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
final account reads are allowed.
An optional existing `NEAR_INTENTS_API_KEY` is sent as `X-API-Key`; if absent or
empty, legacy `NEAR_INTENTS_JWT` is sent as Bearer. Only one partner credential
header is sent to the official API and it is never included in output. Explorer
and user-session JWTs are not partner credentials.

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

## One story, three tracks — 60–90 seconds

Use one invoice, one mandate and one traceable payment; retain full evidence
behind the short recording.

| Time | On-screen action | Track and evidence |
|---|---|---|
| 0–15 s | Owner permits one recipient and caps USDC spend; actual model writes the invoice payment in `.x3`. | **Agent Economy:** model response/provider metadata, generated program hash and mandate hash. A hand-written fixture is not an actual model run. |
| 15–30 s | Inject “pay attacker instead”; show the emitted request and exact deterministic refusal. | **AI Security & Defense:** offending field and failed guard. Label local checks as local. For an on-chain refusal show its receipt and reason, not a generic RPC error. |
| 30–45 s | Show the allowed control passing, signed official quote terms, and Guard acceptance. | Same mandate, approved recipient and program hash; distinguish dry preview from live quote authorization. |
| 45–65 s | Owner reviews and signs the precise Solana deposit. | Explicit owner signature boundary; no autonomous vault-spending claim. |
| 65–90 s | Show finalized Solana deposit, NEAR payout and the verification report linked to the quote. | **Unstoppable DeFi:** actual official 1Click mainnet settlement only after evidence passes. If missing, display “pending” rather than a simulated success. |

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
