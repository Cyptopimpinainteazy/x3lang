# X3 BNB Chain → NEAR: routing, activation, and economics

**Status:** Experimental connector branch. No independent bridge, funds movement, account activation, or settlement deployment has occurred.

## What this PR adds

- BSC.BNB asset/chain mapping to the 1Click provider's currently published native-BNB token, nep245:v2_1.omni.hot.tg:56_11111111111111111111. Near.wNEAR maps to nep141:wrap.near. Check current provider tokens at runtime.
- BSC refund addresses must be complete 20-byte EVM addresses, not DSL placeholder fragments.
- NEAR RPC account-existence lookup for every NEAR destination, including 64-hex implicit accounts. New addresses that are not activated on-chain are refused before requesting a 1Click quote.
- Existing quote-signature, timestamp, refund, fee, slippage and nonce guards remain mandatory.
- Native BNB deposits and refunds can be checked from BSC JSON-RPC transaction, receipt and
  finality data. NEAR delivery remains a separate NEP-141 balance observation.

## 0.001 BNB milestone

The checked-in dry reference is
[`x3-lang/examples/bsc_bnb_to_near_wnear_dry.x3`](../../x3-lang/examples/bsc_bnb_to_near_wnear_dry.x3).
It uses 0.001 BNB (`1000000000000000` base units), active NEAR account `wrap.near`, and
intentionally non-owned placeholder addresses.

On 2026-11-05 the live 1Click token catalog resolved:

- BSC BNB: `nep245:v2_1.omni.hot.tg:56_11111111111111111111`
- NEAR wNEAR: `nep141:wrap.near`

NEAR RPC confirmed `wrap.near` is active before the provider request. The real quote request
was then refused by 1Click:

```text
HTTP 400: Temporary swap limits: minimum swap amount is $1,000
correlationId: 8b22d3af-0b93-473c-ae08-735950672178
```

The complete structured result is in
[`evidence/bsc-near-2026-11-05/quote-001-bnb.json`](../../evidence/bsc-near-2026-11-05/quote-001-bnb.json).
This is a provider-imposed blocker, not a successful quote. The amount was not increased,
the recipient was not changed to bypass validation, and no deposit was made.

## CLI workflow

These commands are offline or read-only until a live quote is explicitly requested:

```bash
# Live token catalog, local lowering, no quote request.
PYTHONPATH=x3-lang python3 x3-lang/near_intents.py plan \
  x3-lang/examples/bsc_bnb_to_near_wnear_dry.x3

# A real provider request. --out writes private success or failure evidence.
PYTHONPATH=x3-lang python3 x3-lang/near_intents.py quote \
  x3-lang/examples/bsc_bnb_to_near_wnear_dry.x3 \
  --out evidence/bsc-near-2026-11-05/quote-001-bnb.json
```

After a successful **live** quote using user-controlled addresses:

```bash
# Prepare register_intent / accept_quote calls, deposit instructions and evidence commands.
# This prints commands; it does not sign or broadcast them.
PYTHONPATH=x3-lang python3 x3-lang/near_intents.py prepare \
  --receipt <live-receipt.json> \
  --program <exact-program.x3> \
  --contract <x3-guard-account> \
  --signer <agent-or-owner-account> \
  --key-file <key-file-path>

# Only after the user sends BNB from their own wallet:
PYTHONPATH=x3-lang python3 x3-lang/near_intents.py submit-deposit \
  --deposit-address <signed-live-quote-address> \
  --tx-hash <bsc-transaction-hash> \
  --receipt <live-receipt.json>
PYTHONPATH=x3-lang python3 x3-lang/near_intents.py status \
  --deposit-address <signed-live-quote-address> \
  --wait --receipt <live-receipt.json>
PYTHONPATH=x3-lang python3 x3-lang/near_intents.py verify-deposit \
  --receipt <live-receipt.json>
PYTHONPATH=x3-lang python3 x3-lang/near_intents.py verify-settlement \
  --receipt <live-receipt.json>
```

`prepare` requires the exact `.x3` source, re-verifies the 1Click signature and request echo,
and checks the source SHA-256 against the receipt. It always reports
`prepared_not_authorized`, `broadcasted=false`, and `wallet_signature_required=true`.

The not-yet-implemented MPC signing boundary is specified separately in
[`CHAIN_SIGNATURES_AUTHORIZATION.md`](CHAIN_SIGNATURES_AUTHORIZATION.md). No Chain Signatures
account, signer contract, MPC signature or funded BSC transaction is claimed here.

BSC evidence requires each configured endpoint to agree on the transaction and to place its
block at or below the endpoint-reported `finalized` block. A failed, missing, unfinalized,
wrong-recipient or underfunded transfer is unverified. A refund must be finalized after the
verified deposit block. Cross-chain causality remains unproven.

Both default BSC endpoints were probed read-only on 2026-11-05 and returned finalized blocks:
[`evidence/bsc-near-2026-11-05/bsc-finality-probe.json`](../../evidence/bsc-near-2026-11-05/bsc-finality-probe.json).
This proves endpoint compatibility only; it is not transaction evidence.

## Three monetizable but separate routing paths

1. X3 → 1Click: retain X3 program, authorization and evidence control; use 1Click's solver execution. Integrators may request disclosed app fees and receive an eligible share under applicable partner rules.
2. X3 → NEAR Intents direct: future separate protocol adapter to interact with solver relay and Verifier. X3 could earn an app fee, and separately earn market-making spread if it actually operates a funded, competitive solver.
3. Independent X3 bridge: not implemented. Requires audited BSC and NEAR contracts, collateral/liquidity, finality verification, secure relayers, recovery, monitoring and refunds. Bridge and liquidity-provider fees would be possible only after reliable operations.

These are alternate routes, not three tollbooths on one transaction. Fees require user consent, full total-cost disclosure and integrity checking. Current adapter docs note the 1Click appFees request field is outside its signed-request field set; enabling fee collection requires a separate review and cryptographic/accounting safeguards. This PR **does not enable or collect fees**.

Official docs:
- https://docs.near-intents.org/resources/fees
- https://docs.near-intents.org/integration/distribution-channels/1click-api/fee-config

## Uninitialized NEAR implicit-account safety

A 64-character NEAR implicit account ID can be derived before it exists on chain. The NEAR account must be created through an initial native-NEAR Transfer receipt (a sponsoring payer may transfer zero NEAR while paying the creation cost and gas). A wNEAR NEP-141 token quote is not the same as creating a native NEAR account. Verify account activation and any token storage registration before asking a wallet to sign.

- Never send BNB directly to a NEAR address through a normal MetaMask transfer.
- Never claim an uninitialized address can receive a 1Click settlement merely because its syntax is valid.
- Do not sign with user keys, fabricate receipts, or claim an account is owned without evidence.
- Never turn a dry quote into an instruction to deposit.

Official NEAR reference: https://github.com/near/docs/blob/master/integrations/accounts.mdx

## Example X3 intent (illustrative only)

Replace BOTH placeholders with validated user-controlled addresses. The receiver must already exist on NEAR.

    intent bsc_near_dry_quote {
        from Bsc.BNB amount 1000000000000000 receiver <YOUR_METAMASK_BSC_ADDRESS>
        to Near.wNEAR receiver <YOUR_ACTIVE_NEAR_ACCOUNT>
        route {
            bridge NearIntents Bsc.BNB -> Near.wNEAR;
        }
        require slippage <= 1%
        require fees <= 100 bps
        require nonce unused bsc_near_001
        timeout 10m refund Bsc.BNB
        on_fail rollback
    }

All amounts are base units. Provider fee schedules and minimum swaps may make small transfers uneconomical. A quote being refused is a valid security outcome.

### BSC RPC identity gate

Every native-BNB evidence endpoint must return `eth_chainId = 0x38` (56); Ethereum and BSC testnet endpoints are refused before a deposit/refund can verify. The chain ID is included in the evidence observation hash. This checks the configured RPC identity but is not a trustless proof of endpoint honesty.
