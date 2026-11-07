# X3 Guard: NEAR Testnet deployment-readiness ceremony (not approved)

> **Historical pre-deployment record.** This page captured a read-only
> feasibility check before the owner-authorized Task 4 deployment. Its
> conclusions about `x3lang.testnet` and the then-current build are superseded
> by [`GUARD_TESTNET_EVIDENCE.md`](GUARD_TESTNET_EVIDENCE.md), which records the
> deployed `x3lang-guard-20261106.testnet` code identity and finalized
> transactions. It remains here as provenance for the fail-closed preflight.

**Historical status at preflight: STOP — no deploy was authorized yet.** A later,
separately approved Task 4 performed the documented testnet-only deployment.
This page is not a funded-deployment runbook and provides no private-key
commands. Do not copy the legacy `network-config mainnet` commands from
`x3-lang/near/x3-guard/README.md` for any testnet work.

## 1. Reproduce read-only account and storage checks

Verify the source revision and obtain a local unsigned WASM build:

```bash
(cd x3-lang/near/x3-guard && cargo test --locked && cargo check --locked --target wasm32-unknown-unknown && cargo build --locked --release --target wasm32-unknown-unknown)
python3 scripts/hackathon/guard_readonly_deployment_preflight.py \
  --account x3lang.testnet \
  --wasm x3-lang/near/x3-guard/target/wasm32-unknown-unknown/release/x3_guard.wasm \
  --report /tmp/x3-guard-readonly-preflight.json
```

The Python command makes ONLY JSON-RPC read requests: `status`, `EXPERIMENTAL_protocol_config`, and `query(view_account)`. It uses two testnet RPC services and an explicitly pinned known testnet genesis hash; it fails on malformed responses or conflicting balance/code/staking data. It **never** calls transaction broadcast, queries key files, creates an account, signs, deploys or moves tokens. It returns exit **3** if the account is funded below the lower-bound WASM byte storage requirement; exit **2** on invalid/incomplete evidence, and exit **0** only for the numeric/account-view precheck (NOT deployment authorization). The report always states `deploy_authorized=false`, `keys_loaded=false`, `funds_moved=false` and `independently_authenticated_chain=false`.

On 2026-11-06 UTC a prior local build of `x3_guard.wasm` was 545,640 bytes with SHA-256 `b2b3c397d088c5d813d9efea1cd18e6250baeb743ad701d5f7d48166f0b9fae3`. Two public NEAR Testnet RPC services returned `x3lang.testnet` exists with no deployed code (code hash `11111111111111111111111111111111`), 2 NEAR account balance, and current storage staking `10000000000000000000` yoctoNEAR per byte. The contract code alone therefore requires at least **5.4564 NEAR** of storage stake; existing state, program storage, creation costs and gas add further headroom requirements. The public account's ownership **has not been proven**. The account name and balance do not authorize us to use its keys or deploy software.

This lower-bound calculation is a **storage-balance check**, not a predicted deploy gas fee, and neither key possession nor ownership is checked by the script.

## 2. Real signed-quote availability blocker

The official 1Click SDK uses a pinned production manager public key (`ed25519:reYaWhvwu8Jzo3WUM3zhn6VrhuMEF4eADL17qtRVifc`), which is embedded in X3 Guard. Real 1Click is documented for mainnet; no official NEAR Testnet 1Click settlement environment has been independently confirmed. A community project called Tidepool announced a 1Click-compatible **third-party** testnet API on the NEAR Forum on 2026-11-02. The project's own description says it is **not affiliated with NEAR**. A sandbox quote signed by its own key cannot pass the current immutable production manager key check without a separately audited design/deployment, and that would be a different demonstration with different trust assumptions.

Never claim that a locally signed test fixture (with a throwaway key) is an official real 1Click quote. Never reuse an expired historical signed quote and call it a fresh authorized quote. A newly issued *genuine 1Click production-manager-signed quote*, if obtained through a separately authorized interaction, might be verifiable by a testnet-deployed X3 Guard as a **signature/policy-gate demonstration only**. It would NOT be a genuine testnet 1Click swap or testnet solver settlement and must be labeled as such. Authorization to retrieve a live quote is separate from authorization to fund any provider deposit address; no deposits or wallet interactions are part of this preflight.

For a high-scoring judge demonstration, prove **both** that the quote's exact bytes verify under the key in the deployed contract and that a *prohibited signed term* causes failure **after signature verification**. A false signature or expired quote does not prove the policy-bound signed-quote differentiator. Verify operator policy for testnet-specific quotes before any live-chain demonstration.

## 3. Owner-controlled deployment decision gates (not performed)

Before any deployment/signing, owner must explicitly authorize:

1. Exact fully qualified **testnet** contract account, verified ownership/access and financing sufficient for code+state+fees; never assume `x3lang.testnet` is owned merely because it exists.
2. Reviewed exact commit, independently reproducible WASM (source lockfile/toolchain/sha256), and a clear assessment of contract storage/rent, account access, upgrade and migration risks.
3. Initializer `new()` semantics and deployment-network identity. The Guard's pinned manager key cannot be rotated in place; a mistaken deployment can require re-deployment.
4. Funding plan, isolated testnet signing keys handled **only by the owner**, maximum allowed transaction/deposit values, and rollback/recovery plan. No secret key material enters ChatGPT, a terminal transcript, the repository, or published evidence.
5. Real signed quote provenance: source URL, issuance time, signer key checked, no future mainnet deposit execution, expiry, exact permitted/forbidden terms, and no claim of payment settlement.
6. Record approved transaction/receipt ID, block height, deployed account, **on-chain code hash**, exact state and finality evidence. Verify read-only via two RPC providers and distinguish public-RPC confirmation from independently trusted light-client finality. For rejection, NEAR may record a failed transaction/receipt and no persisted Guard event; inspect the finalized failure receipt and the unchanged contract state rather than fabricating an on-chain `quote_denied` event.

## 4. Hackathon claims while blocked

**Already evidenced:** deterministic `.x3` policy preview, pinned-manager signature checking in source/tests, 51 native Guard unit tests, successful WASM build, real archived NEAR validator signature checks (under an untrusted bootstrap checkpoint), reproducible judge quick gate.

**NOT evidenced:** on-chain Guard accepted quote, on-chain rejection of a valid manager-signed but policy-prohibited quote, independent trust anchor for resulting receipt, authentic NEAR escrow state, cross-chain causality, or authorized deployment.

Shorter safe judge alternative until approval: reproduce **local** contract tests showing a correctly signed test-key quote rejected after policy evaluation, and exhibit the immutable production manager-key verification against archived genuine 1Click-signed quotes separately. Label them as different evidence classes. This does not replace the required live-chain demonstration.

Sources for reference: NEAR account storage documentation at `https://github.com/near/docs/blob/master/integrations/accounts.mdx`; official 1Click API docs at `https://github.com/defuse-protocol/gitbook-docs/blob/main/integration/distribution-channels/1click-api.md`; community third-party testnet announcement `https://gov.near.org/t/tidepool-a-1click-compatible-testnet-sandbox-for-near-intents/42746`.
