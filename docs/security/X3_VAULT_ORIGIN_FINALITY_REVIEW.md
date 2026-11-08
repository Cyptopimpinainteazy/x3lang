# X3 Shield — vault-origin Base finality verifier (private local review artifact)

**Status: unmerged source-review implementation. Not production- or custody-audited. No public deployment, signing, funding or network tests performed in this artifact environment.**

This patch addresses the **Base vault-origin part** of private X3Lang [Issue #141](https://github.com/Cyptopimpinainteazy/x3lang/issues/141). It must be reviewed against the exact PR #138 Solidity code and stacked after its separate maker/Guard binding PRs #140/#142. The existing historical Base Sepolia → NEAR transfer from #122 was a direct EOA → ERC20 transaction, **not** a payment from a deployed X3 Base vault; the verifier explicitly refuses it.

## Files

- `keccak_min.py`: no-dependency Ethereum Keccak-256 for method/event hashes, self-tested against ERC20 selector/topic.
- `vault_origin_base_proof.py`: strict Base Sepolia tx, receipt, event, canonical/finalized block, deployed code and immutable policy verifier using **two independent HTTPS JSON-RPCs**. Includes a READ-ONLY CLI. No write methods or keys.
- `test_vault_origin_base_proof.py`: clearly **synthetic** JSON-RPC response fixtures and negative tests. A positive synthetic fixture is NOT presented as a real chain transaction.

These three Python files are intended to be placed together under `scripts/hackathon/` on an owner-approved private feature branch. They do not need third-party Python dependencies.

## Review manifest — not an owner signature

Create a file with **exactly** these 15 expected fields plus `tx_hash` and `rpc_urls`. All addresses are `0x` 20-byte hexadecimal. SHA-256 values are lowercase 64-hex strings (without `0x`). `nonce` and transaction hash are `0x` + 64 hex.

```json
{
  "vault":"0x<40 hex>",
  "owner":"0x<40 hex>",
  "agent":"0x<40 hex>",
  "token":"0x<40 hex>",
  "deposit_address":"0x<40 hex>",
  "quote_sha256":"<64 hex>",
  "program_sha256":"<64 hex>",
  "token_runtime_sha256":"<64 hex>",
  "vault_runtime_sha256":"<64 hex>",
  "nonce":"0x<64 hex>",
  "amount":1000000,
  "deadline":1790000000,
  "expires_at":1790000300,
  "max_per_payment":1000000,
  "max_total":1000000,
  "tx_hash":"0x<64 hex>",
  "rpc_urls":["https://<provider-a>","https://<independent-provider-b>"]
}
```

`deadline` and `expires_at` above are **illustrative**, not valid quote authority. Use real reviewed values and an independently reproducible pinned vault runtime SHA. Do not fabricate hashes or supply the historical direct EOA transaction.

```bash
python3 scripts/hackathon/vault_origin_base_proof.py \
  --review-manifest /private/path/to/reviewed-vault-manifest.json \
  --output /private/path/to/new-unmodified-vault-origin-evidence.json
```

The CLI checks two independent RPCs, chain 84532, transaction/receipt hashes and destinations, calldata for `executePayment`, static ABI offset/signature size, nonce, recipient, payment amount/deadline, actual vault and token runtime SHA-256 at the inclusion block, on-chain owner/agent/token/recipient/quoteHash/programHash/budget/expiry/nonce views, read-only `paymentDigest`, exactly one `PaymentExecuted` and one ERC20 `Transfer` from the VAULT, canonical inclusion, 12+ confirmations, and finalized L2 head across providers. It **does not** establish L1 withdrawal finality or trustless bridge atomicity.

A passing CLI result is classified `READ_ONLY_BASE_VAULT_ORIGIN_FINALIZED_EVIDENCE_NOT_CROSSCHAIN_PROOF` and is SHA-256 hashed into a new immutable file. A failure prints `HARD_DENIED_NO_VAULT_ORIGIN_PROOF` and exits 2. It never authorizes payment.

## Important remaining gates

1. The manifest values are **not owner-signed**, and this source-only verifier does **not** itself validate Tidepool Ed25519 maker signatures, X3Lang program compilation, or real on-chain Guard history. Stack with the reviewed #140 + #142 read-only attestation implementation. A stale quote remains forbidden regardless of an apparent valid source receipt.
2. Issue #141 also requires **NEAR final execution, canonical transfer receipt, actual USDC balance delta, and Guard approval BEFORE the Base source transfer**. Those must be verified using independent NEAR RPCs (and cryptographic validator/finality evidence if claiming trustlessness). A source-only verified receipt must **never** be relabeled a full crosschain proof.
3. Two agreeing public RPC providers do not constitute a cryptographic light client. Token upgrade/blacklist powers, owner/agent key isolation, testnet source-vault funding, Tidepool relayer trust, EVM gas budget, redemption/refunds and upgrade migration require independent security review.
4. The new Base Sepolia vault has **not been publicly deployed or funded**. No new vault-origin source tx exists in the evidence reviewed here. A **new** public testnet trade requires explicit owner approval of exact wallet, code SHA, contract account, token, quote, program nonce, recipient, gas and loss cap. Do not deploy or sign autonomously.
5. Both repos were intended private until November 14. The GitHub connector reported `x3lang` publicly visible on November 7; resolve that before publishing any new PR or evidence.

## Offline tests

```bash
cd scripts/hackathon
python3 -m unittest -v test_vault_origin_base_proof
```

The tests use synthetic RPC data and do not contact any chain. Positive fixture output must **not** be used as an evidence certificate. Real RPC runs remain untested in this local review environment.

## Destination NEAR and combined read-only proof

The patch also includes `near_delivery_receipt_proof.py` (requires real NEAR
`EXPERIMENTAL_tx_status` with **FINAL** execution, a successful token-contract
`ft_transfer` action and receipt outcome, canonical finalized transfer block,
independently queried recipient `ft_balance_of` before and after, and canonical
historical Guard code/policy approved **before** the Base source transfer).
It requires two credential-free HTTPS NEAR RPC providers with agreeing receipt,
Guard policy and balance evidence. **The audit now additionally requires a
`guard_pre_source_block_hash` in the NEAR review manifest.** The verified
historical Guard approval is not enough by itself: the verifier re-queries
Guard code and policy at the *last canonical NEAR block preceding the Base
source inclusion second*, confirms the immediate successor block is later
than that second, and rejects prior revocation or changed policy/code.
This is strict near-source-time state evidence subject to timestamp precision
and two RPCs' trust assumptions, not cross-chain synchronous consensus.
Missing archive-state support fails closed rather than substituting an older
approval or an optimistic snapshot. The historical Guard policy must bind owner,
new nonce, exact quote/program hashes, amount, signed refund address,
slippage, signed quote deadline and the compiled `.x3` minimum. It does not
represent two RPCs as cryptographic validator-signature authentication.

`vault_two_chain_pairing.py` verifies source/destination chronology, matching
quote and compiled `.x3` hash, source amount, minimum payout, Guard timing,
identical Base/NEAR tx identifiers and the **exact deterministic #140 EIP-712
nonce derivation**. The result explicitly denies relayer causal certainty and
trustless cross-chain atomicity.

`vault_origin_two_chain_cli.py` composes the **real pinned X3Lang maker signature
validator and `.x3` compiler**, the reviewed #142 Guard account/code pin, both
independent RPC source/destination verifiers, and the pure pairing gate. It
requires two JSON reviewer manifests (Base and NEAR) plus a maker-signed quote
file. The current historical `.x3` owner nonce
`x3_base_testnet_trade_001` is **explicitly forbidden** for a new vault
trade; a fresh real intent and unique Guard approval must be produced with
separate owner authorization first. A successful future run writes a
create-only SHA-256-hashed, read-only evidence bundle, not an authorization to
send funds. No automatic deployment/signing/funding path exists.

**The combined CLI has not been exercised with public vault settlement RPC
receipts because no such new deployed/funded vault and transaction have been
verified.** The two source/destination attestations now record the
independent provider **hostnames** used; credential-bearing query strings or
URL fragments are rejected instead of being retained in reports. Its synthetic test coverage is a development safety gate, not proof
of a public cross-chain swap. It also depends on X3Lang source modules
`base_sepolia_guarded_admission`, `base_sepolia_x3_intent`, and
`near_guard_quorum` already present in the target branch stack; if they are
unavailable the CLI fails closed.

## Proposed CI (not yet installed)

`.github/workflows/x3-vault-origin-finality-readonly.yml` runs all offline
negative tests on exact PR SHA with Python 3.12. Tests are deliberately labeled
**synthetic JSON-RPC fixtures**. This workflow does not claim to have run in
GitHub and does not replace any private `xxxstar` mirror gate.

## Additional fail-closed hardening (November 8, 2026)

The original supplied patch's 31 synthetic tests were reproduced offline.
Independent review added 4 tests (35 total) and corrected two evidence
weaknesses:

1. A historical valid Guard policy alone cannot disprove a later revocation.
   The verifier now requires the **last canonical pre-source block** and its
   canonical immediate successor; its Guard code and policy must remain
   identical to the reviewed approval and unrevoked. Adversarial mutations of
   the pre-source policy, code/block timing and block identity fail closed.
2. Provider names are now included in the returned attestations. HTTPS RPC
   URLs with query strings or fragments are forbidden; this helps keep
   credentials or unreviewed request parameters out of the evidence packet.

The 35 tests remain **synthetic/offline**; they do not demonstrate a deployed
Base vault, successful vault-origin funds movement, public NEAR settlement,
or trustless relayer causality.

**Further release gates:** the existing `compile_testnet_intent()` currently
uses the historical fixed nonce `x3_base_testnet_trade_001` and the combined
CLI deliberately rejects reusing it. A fresh owner-reviewed `.x3` program,
nonce, maker quote, on-chain Guard approval, audited vault runtime hash and
real receipt pair are still required. Do not bypass this rejection. A
manifest-provided runtime SHA-256 alone is **not an independent security
audit or release signature**; verify it against reproducible source and the
owner's approved code release before any public funds.
