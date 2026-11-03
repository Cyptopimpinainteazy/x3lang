# X3 Shield threat model and unresolved-risk register

**Snapshot:** prepared against `main@e44a9931` on November 3, 2026. Update this document when
authorization, settlement, or deployment code changes.

This document describes the trust boundaries of the code currently in the repository. It is not
a mainnet approval, a claim of contract immutability, or evidence that a live payment has
occurred.

## Security objectives

1. A model, prompt, invoice, quote, or frontend cannot authorize a payment outside the owner's
   compiled policy.
2. A quote is accepted only when its signed terms match the registered policy and the caller is
   authorized by the mandate.
3. Failed, refunded, stale, unverified, or causally ambiguous transactions never generate a
   success fee.
4. A performance fee is possible only when an independently obtainable reference route proves
   that the user's post-fee result is better.
5. Evidence remains reproducible and distinguishes signed terms, observed chain facts, and
   proven cross-chain causality.

## Assets and authority

| Asset or authority | Current location | Security relevance |
|---|---|---|
| Origin-chain funds | Owner wallet or agent-controlled wallet, outside X3 Guard | The largest unresolved custody boundary |
| Approved spending terms | `x3-guard` contract storage | Per-payment/daily limits, recipients, assets, refund, expiry, approvals |
| 1Click signed quote | Supplied by caller and verified in Guard | Binds provider-signed amounts, assets, receiver, deposit address, and deadline |
| Owner/approver signatures | NEAR transaction signatures | Authorize mandate creation, replacement, revocation, and large payments |
| Compiled program hash | Guard policy and agent approval prompt | Binds authorization to the reviewed X3 source compilation |
| Settlement receipt | Private local file plus public chain RPC observations | Evidence input only; local `verified_outcome` fields are untrusted |
| Guard upgrade authority | NEAR account key used for code deployment | Can replace enforcement code unless independently locked or governed |

## Trust boundaries

### 1. Model output is untrusted

`x3-lang/near_agent.py` treats the English request, attached documents, and model response as
untrusted input. The model may write a `.x3` program, but the pipeline subsequently enforces:

- one fenced program and an issued nonce;
- `under mandate <owner>`;
- successful compilation and verification;
- deterministic mandate checks;
- owner approval bound to the exact program SHA-256 before a live quote.

The local `qwen2.5-coder:7b` red-team run held all eight repository attacks, but a different model,
prompt, or sampling configuration is not covered by that result.

### 2. Guard is not a wallet

`x3-lang/near/x3-guard` holds policy state, not origin-chain funds. It can reject Guard methods
that violate a mandate. It cannot stop a key holder from sending funds directly from an
origin-chain wallet or from a wallet integration that ignores Guard.

Wallet-level enforcement therefore requires a trusted wallet contract, multisig, or executor
whose signing policy independently checks Guard authorization. This is not implemented.

### 3. Signed quote terms are not payment proof

The pinned 1Click manager key authenticates the quote payload. A valid signature proves the
signed terms; it does not prove that a deposit or delivery occurred.

The 1Click status response is not signed by 1Click. `record_outcome` records an owner or agent
attestation and its consistency with the accepted quote; it is not independent settlement
evidence.

### 4. Chain observations are not causal attribution

A finalized Solana SPL transfer and a NEAR balance increase can be independently re-read. They do
not prove that a particular solver caused both events. The judge evidence gate must continue to
report cross-chain causality as unproven unless a real protocol provides a verifiable linkage.

### 5. RPC endpoints are external dependencies

Chain reads fail closed on malformed, missing, stale, or disagreeing responses. One RPC endpoint
is not a verification quorum. Operators must configure multiple trusted endpoints when making a
public evidence claim.

### 6. Contract upgrade authority is outside the functions

The contract exposes no owner/admin setter, but a NEAR contract account may still be upgraded by
whoever controls its deployment key. Immutability requires removing or governing that key.
Source code cannot prove the deployed account's key policy.

## Implemented controls

| Control | Evidence |
|---|---|
| On-chain 1Click Ed25519 signature verification | `x3-lang/near/x3-guard/src/lib.rs`; quote signature tests |
| Exact asset, amount, receiver, slippage, fee-cap, nonce, and deadline checks | Guard quote guards and adapter tests |
| Mandate per-payment, rolling 24-hour cap, recipient/asset allowlists, pinned refund, expiration, revocation, versioning, and N-of-M approvals | `x3-lang/near/x3-guard/src/mandate.rs`, Python parity tests |
| Model output cannot bypass compiler or mandate | `x3-lang/near_agent.py`, `x3-lang/tests/test_near_agent.py` |
| Provider timeout/outage is a typed error and red-team records it as unheld | PR #8 tests |
| Allocator advisory removed | PR #5; no `wee_alloc` in the Guard dependency graph |
| Read-only evidence gate rejects forged `verified_outcome` and dry-to-live relabeling | Draft PR #7 tests |
| Fee arithmetic is opt-in and preview-only | Draft PR #4 tests; `collectible: false` |

## Attack and abuse cases

| Attack | Expected result | Current status |
|---|---|---|
| Forged or altered 1Click signature | Refuse before reading quote facts | Implemented and tested |
| Dry quote relabeled as a live payment | Refuse as payment evidence | Implemented in draft PR #7 |
| Prompt injection redirects recipient or refund | Compile/mandate rejection | Tested with scripted and local models |
| Agent overspends or splits a payment | Cap or approval refusal | Implemented and tested in Guard |
| Agent uses another owner's mandate | Refuse | Implemented and tested |
| Replayed nonce | Refuse | Implemented and tested |
| Refund redirected to the agent | Refuse | Implemented and tested |
| Provider outage or timeout | Typed provider error; no success verdict | Fixed in PR #8 |
| Duplicate outcome or duplicate success fee | Refuse duplicate outcome; fees are not collectible | Outcome protection implemented; fee collection not implemented |
| Unprofitable candidate route | Fee must be zero | Preview arithmetic implemented; live mechanism absent |
| Direct wallet transfer bypasses Guard | Must be prevented by wallet-level enforcement | **Open architectural gap** |
| Destination asset and recipient are individually allowed but not a valid pair | Refuse | **Open; current lists are not tuple-bound** |
| Agent registers unlimited unaccepted intents | Bound active state per owner | **Open; intent storage is unbounded** |
| Upgrade from pre-mandate state destroys or misreads records | Explicit migration with fail-closed decoding | Proposed in draft PR #9 |
| Deployment key replaces Guard code | Lock or govern the contract account key | **Open deployment risk** |
| Compromised RPC reports favorable balances | Require independent endpoints and disagreement checks | Operational control only; no public quorum evidence |
| Solver causality inferred from two transfers | Keep causality unproven | Correctly reported unproven |

## Prompt-injection and model supply chain

The model provider is selected by environment and can be a local OpenAI-compatible server. Provider
credentials must never enter generated X3 source, model context, receipts, or evidence artifacts.
Model transport output is bounded and parsed as data; provider failures are typed errors.

Remaining risks:

- A model can still generate syntactically valid but economically harmful programs that pass all
  checks if the owner's policy itself permits the harm.
- Model/provider version, weights digest, and sampling configuration are not yet pinned in a
  committed evidence artifact.
- Denial of service through slow inference remains possible; timeouts now fail the affected case
  rather than crashing the whole suite.

## Financial arithmetic

Fee logic must use integer base units and checked arithmetic. The proposed invariant is:

```text
F <= floor(S x P)
F <= approved absolute cap
C - F >= R + minimum user benefit
```

where `R` is the reference net output, `C` is the candidate net output before X3's fee, `S` is
positive improvement, `P` is the approved savings share, and `F` is the fee.

The current draft computes only a non-billable preview. It has no authenticated comparable quote,
wallet authorization, Guard fee policy, settlement-linked collection, or observed savings. The
correct live default remains zero.

## Evidence taxonomy

| Class | Meaning |
|---|---|
| Local unit/component test | Code behavior under local test doubles |
| Local integration test | Multiple repository components exercised together |
| Exact-head CI | Repository workflows passed for the named commit |
| Signed quote | Provider-signed terms; no funds moved |
| Chain observation | Re-read finalized transaction/balance facts from RPC |
| Proven settlement | Requires an independently verifiable link from authorization through both chains |

Repository CI does not establish the last two classes. Public judges should receive explicit
transaction and deployment links only after owner-authorized live actions.

## Release blocking conditions

Do not describe X3 Shield as production-ready until all of the following are resolved:

1. Wallet-level enforcement prevents bypass of Guard-authorized spending.
2. Destination asset, chain, and recipient pairs are bound by policy.
3. Active intent storage is bounded and terminal records follow a defined lifecycle.
4. Versioned contract migration is reviewed, merged, and exercised on a testnet deployment.
5. Contract upgrade authority is explicitly locked or governed.
6. Independent chain evidence is reproduced from a real owner-approved transaction.
7. Live fee logic, if ever enabled, proves the economic invariant and reconciles its receipt.
8. An external security review covers Guard, model pipeline, and deployment configuration.

## Reporting security issues

Do not publish private receipts, RPC credentials, model API keys, deployment keys, or unredacted
wallet data. Report suspected vulnerabilities privately to the repository owner and preserve the
raw evidence locally. Public reports must contain only sanitized artifacts and exact commit IDs.
