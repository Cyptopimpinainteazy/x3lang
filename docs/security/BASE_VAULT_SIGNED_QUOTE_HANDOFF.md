# X3 Shield — signed Tidepool quote to Base vault binding

Status: private, unsigned, read-only review implementation. NO deployment,
funding, wallet signing, ERC-20 approval, broadcasting or settlement proof.

This handoff connects the existing strict testnet Ed25519 Tidepool maker
signature checker, the actual X3Lang Python frontend financial policy,
the read-only finalized NEAR Guard policy reader, and the proposed immutable
terms of the standalone Base Sepolia ERC-20 custody vault. It does NOT add
Tidepool Ed25519 verification to Solidity. The deployed vault would trust
immutable parameters explicitly approved by a separate owner.

## Tests and security boundaries

The binding compares these five distinct authorities:

1. Tidepool community testnet maker signature on compact signed JSON with
   a fixed pinned Ed25519 key. Source 84532 test USDC, exact one-USDC amount,
   origin refundTo EOA, signed deposit address, destination x3lang.testnet,
   minimum NEAR USDC out, issued-at freshness and quote expiry are checked.
2. The real repository .x3 frontend parses/typechecks the fixed testnet program,
   verifying exact SHA-256, source, minimum out, refund address, nonce, and
   time limit. This is Python-frontend policy, NOT native X3VM bytecode.
3. **Two independent NEAR testnet RPCs** must agree on the SAME
   canonical finalized checkpoint, the reviewed on-chain Guard account
   `x3guardlaunch2026.testnet`, its SHA-256-pinned WASM code, and the
   owner/nonce/policy on the same block. Each provider also checks its
   **latest finalized code and policy**. Any finality-head lag >60 NEAR
   blocks, divergence, missing historical block, unexpected revocation
   or substituted Guard account/code fails closed.
   Two public RPCs are **NOT an authenticated validator-light-client proof**.
   Synthetic policy rows in tests verify comparison logic only, not chain facts.
4. Owner-provided review terms explicitly pin proposed Base Sepolia constructor
   agent, owner, asset code hash, one allowed recipient equal to the maker's
   signed deposit address, program and quote hashes, one 1,000,000-unit payment,
   and total spending budget of the same amount.
5. The proposed vault's Solidity EIP-712 order template binds the same one-use
   nonce and quote/program commitments, but cannot create a digest or signature
   without a real, verified deployed vault address and chain ID.

A signed quote alone does not prove its refund address is owned by the intended
owner. A review terms file is NOT a digital owner signature and cannot be
used for autonomous authorization. Funds outside the Base vault remain
bypassable if an unrestricted owner/agent EOA key is compromised.

## Read-only command

Supply a NEW and genuinely signed live Tidepool testnet quote, classified as
LIVE_QUOTE_UNFUNDED_UNAUTHORIZED_NOT_SETTLED by the existing read-only capture,
and an independently prepared owner reviewer-terms file. Both should be
private to the operator and must contain NO wallet secrets.

The reviewer JSON requires exactly these fields:

- source_owner, agent_signer, origin_chain_id, token_contract,
  token_code_sha256, destination_account, refund_to, deposit_address,
  amount_base_units, max_per_payment, max_total, approved_quote_sha256,
  approved_program_sha256.

The owner must independently check that source_owner is the approved EVM
source wallet and that agent_signer is a distinct public EVM address. The
existing signed quote requires the refund address to be that source owner's
EOA, NOT the vault. Treat this as an explicit return-to-owner trust boundary.

    python3 scripts/hackathon/base_vault_quote_binding.py \
      --quote-file /private/path/to/fresh-signed-quote.json \
      --reviewer-terms-file /private/path/to/owner-reviewed-terms.json \
      --guard-account x3guardlaunch2026.testnet \
      --guard-wasm-sha256 1d7a8b1ee97b7cdc500db1c19b8f3a2bef06e986576ec428f62f8c3ea1b5a145

The script cannot sign, deploy, approve, or transfer anything. It does not
emit executable Solidity payment calldata or a completed EIP-712 digest.
In addition to the original Base funding/chain quorum, it now invokes the
reviewed two-provider token attestor from base_vault_token_preflight.py:
real token bytecode SHA-256, historical/finalized canonical checkpoints,
and standard EIP-1967 implementation/beacon slots must all pass. A zero
standard proxy slot cannot rule out custom token upgrade mechanisms.
It rechecks maker-signature quote freshness and the two-RPC Guard policy quorum
after slow Base RPC calls. The final time check runs AFTER the final
network read; a quote expiring during RPC access must be denied.
Any Guard account or WASM hash other than the exact reviewed deployment
fails closed, even if both RPC servers agree on alternate attacker-owned code.
Failure exits 2 and prints HARD_DENIED_NO_SIGNING_OR_DEPLOYMENT.

The authentic historical fixture
x3-lang/near/x3-tidepool-guard/src/signed_live_fixture.json
is used only for negative attack tests and for testing cryptographic
consistency at its HISTORICAL issuance timestamp. Running the CLI now with
that old quote fails as stale BEFORE any RPC access. This is intentional.
The old Guard nonce x3_base_testnet_trade_001 also requires a fresh new
compiled intent/nonce and a new owner-approved on-chain Guard state before
a new actual payment. Do not reuse the old paid swap proof.

### Recorded independent Guard read-only observation

One **real**, historical two-provider read-only observation of the existing
Guard is preserved in
`evidence/base-vault-local/guard-finalized-quorum-20261107.json`
(SHA-256 `7efa87b89402622d79c1eac2848e10136989c2654404bb052ac54b2f63978512`).
This is **not** a fresh authorized quote or a cryptographically authenticated
validator signature proof. The previously executed maker quote is EXPIRED.

Re-run the read-only observation when needed with:

    PYTHONPATH=x3-lang:scripts/hackathon python3 scripts/hackathon/near_guard_quorum.py       --guard-account x3guardlaunch2026.testnet       --owner x3lang.testnet       --nonce x3_base_testnet_trade_001       --wasm-sha256 1d7a8b1ee97b7cdc500db1c19b8f3a2bef06e986576ec428f62f8c3ea1b5a145       --output /tmp/x3-guard-new-readonly-observation.json

Use a NEW output path on each rerun. No signing or transfer action exists here.

## Release-critical work NOT completed

- Solidity currently verifies the agent's EIP-712 signature over owner-pinned
  quote/program hashes, NOT Tidepool maker Ed25519 directly. The independent
  owner must securely register correct terms. This builder does not attest a
  signed owner approval or remove an owner key from the agent runtime.
- After owner-approved deployment, verify vault chain ID, actual deployed
  bytecode, on-chain immutable owner, agent, recipient, asset, token code,
  limits, expiry and hashes at independent Base RPCs. Do not trust a preview
  as a deployed contract.
- The existing two-chain settlement verifier expects a direct ERC-20 call
  from the historical EOA. It CANNOT certify a new vault-origin transaction.
  A separate independent proof must require a finalized Base transaction
  whose to address is the vetted vault, a PaymentExecuted event with the
  exact digest/nonce, a matching ERC-20 Transfer FROM THE VAULT to the
  signed Tidepool deposit address, correct token amount and canonical
  receipt. Then require finalized destination NEAR USDC transfer receipt
  AND independently observed recipient balance delta. Tidepool relayer
  causality remains a third-party trust assumption; this is not atomicity.
- A fresh maker quote, independent owner approval of gas/refund/limits,
  independently audited vault and token, fresh Guard mandate, funded owner
  account, public deployment and signed testnet payment are all absent.
  There is no permission to do any of those actions in this PR.
- Both x3lang and xxxstar stay private until explicit owner publication
  approval. Do not merge overlapping financial/security branches without
  independent exact-head review, and do not disable the peer proof gate.

## Offline reproducibility

    PYTHONPATH=x3-lang:scripts/hackathon python3 -m unittest \
      scripts.hackathon.test_base_vault_quote_binding       scripts.hackathon.test_near_guard_quorum -v

The existing Base guarded admission and maker signature regression suites
are also required. Source path:
scripts/hackathon/base_vault_quote_binding.py.
