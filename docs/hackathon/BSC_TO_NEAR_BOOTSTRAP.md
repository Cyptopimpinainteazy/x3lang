# X3 BNB Chain → NEAR: routing, activation, and economics

**Status:** Experimental connector branch. No independent bridge, funds movement, account activation, or settlement deployment has occurred.

## What this PR adds

- BSC.BNB asset/chain mapping to the 1Click provider's currently published native-BNB token, nep245:v2_1.omni.hot.tg:56_11111111111111111111. Near.wNEAR maps to nep141:wrap.near. Check current provider tokens at runtime.
- BSC refund addresses must be complete 20-byte EVM addresses, not DSL placeholder fragments.
- NEAR RPC account-existence lookup for every NEAR destination, including 64-hex implicit accounts. New addresses that are not activated on-chain are refused before requesting a 1Click quote.
- Existing quote-signature, timestamp, refund, fee, slippage and nonce guards remain mandatory.

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
        from Bsc.BNB amount 20000000000000000 receiver <YOUR_METAMASK_BSC_ADDRESS>
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
