# Invoice_06 outcome recorded on mainnet Guard

At 2026-11-13T05:31:17.710294+00:00, the explicitly authorized outcome call had finalized:

- Transaction: [`9sTn9pJ1ZbCVazBm9Wz9oEDh6aWw4FUVMoAJEKCCzuqE`](https://nearblocks.io/txns/9sTn9pJ1ZbCVazBm9Wz9oEDh6aWw4FUVMoAJEKCCzuqE).
- Execution block: **224319990**; finalized `get_intent` readback block:
  **224320007**.
- Signer: `x3-agent.x3lang.near`, public key
  `ed25519:J3NiU8J63HAfz2MTDu1zQVhFRYS5Pio21h1kNtJdP11x`.
- Restricted FunctionCall key; receiver `x3lang.near`; one
  `agent_record_outcome` action; **zero NEAR attached**, 10 Tgas prepaid.
- Outcome: **`Settled`**, `amount_out: "24995200"` (24.9952 USDC),
  `refunded_amount: null`, **`floor_met: true`**.
- Both finalized settlement hashes are stored in `outcome.tx_hashes`:
  Solana deposit `4zhGYyWNabYQ5gixTUzoVHVbgFKWAsQssSpCv3B1J1LjCRisEJW6wm3zuS2GVuDotAT93S8wm1qezHwsKX2RkXo5`;
  NEAR payout `8jfbACLSxrzi3fnajfAVGc4Fgo6XLCWA1mpiJWXPw43P`.
- All **21** exact-call and finalized-readback checks passed. Policy,
  accepted quote, approvals and mandate version 4 were preserved.
- Gas fee: `287165526854800000000` yoctoNEAR; owner-funded storage credit debit:
  `1730000000000000000000` yoctoNEAR. No settlement tokens were moved by this call.

## Evidence chain

1. [settlement-reverification.json](settlement-reverification.json): the existing
   `verify_mainnet_payment.py` passed against live public RPC and provider status
   **before submission**, including authenticated quote identity, Solana mainnet
   finalized USDC instruction/signer/amount/deposit destination, and the canonical,
   finalized NEAR USDC payout receipt with the exact recipient and amount.
2. [approved-preflight.json](approved-preflight.json) and
   [fresh-send-preflight.json](fresh-send-preflight.json): exact arguments, empty
   prior outcome, accepted quote, Guard code identity, restricted key permission,
   allowance/cost check and prepaid storage credit. The latter was refreshed before
   signing. Key files and their contents are excluded.
3. [send-journal.jsonl](send-journal.jsonl): the signing helper emitted one candidate
   hash and attempted one send. Its provisional response is deliberately unverified;
   the following independent reconciliation establishes final success.
4. [finalized-outcome-verification.json](finalized-outcome-verification.json): full
   public transaction/receipt response, finalized intent readback and 21 checks.
5. [execution-environment.json](execution-environment.json): base commit, software,
   helper hash, operation scope, authorization, single-send and rollback boundaries.

The original parent `public-rpc-observation.json` remains unchanged: it was a
historical observation before the outcome was recorded. These files provide the
later observation and transaction. Storage uses the already funded agent credit;
no funding/top-up or owner-signed transaction was needed.

[independent-finalized-readback.json](independent-finalized-readback.json) preserves
a separate reviewer's fresh public RPC responses and 18 passing checks at finalized
readback block **224320183**. It also confirms the key remains FunctionCall-scoped
and every intent field except the newly added outcome matches the earlier record.

## Read-only reproduction

From this directory, run `sha256sum --check SHA256SUMS`.
For a live check, call NEAR public JSON-RPC `tx` with:

```json
{"tx_hash":"9sTn9pJ1ZbCVazBm9Wz9oEDh6aWw4FUVMoAJEKCCzuqE","sender_account_id":"x3-agent.x3lang.near","wait_until":"FINAL"}
```

Require final success, no failed receipts, the restricted public key above, one
expected method with exactly the arguments in `fresh-send-preflight.json`, and
zero attached deposit. Then query `get_intent` on `x3lang.near` at `finality: final`
with `{"owner":"x3lang.near","nonce":"mainnet_demo_nov11_invoice_06"}`.
Require its outcome to match the returned transaction outcome, both settlement
hashes, `Settled`, `24995200` and `floor_met: true`.

## What this proves

The restricted agent recorded the settlement report and Guard persisted it in
finalized state after separate transaction verification. Guard's amount comparison
sets `floor_met`; it does not independently authenticate the supplied transaction
hashes or prove solver-level cross-chain causality. Provider linkage remains an
unsigned association. This is an agent-reported outcome backed by the separately
reverified finalized transfers, not an on-chain cross-chain proof verifier.
