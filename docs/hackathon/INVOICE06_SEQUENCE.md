# invoice_06, as one sequence

Nonce `mainnet_demo_nov11_invoice_06`. About $25. NEAR mainnet and Solana mainnet.

This is the path that is not a spending-limit dashboard. The same two `.x3` files are the written program, the program the compiler rejects when edited, the terms Guard enforced, the payment that settled, and the intent whose outcome was recorded.

From the repository root:

```bash
python3 scripts/hackathon/invoice06_sequence.py
```

Expect `PASS`. The command is offline. It does not sign, deploy, or send. Python 3 is enough; it does not need the judge virtualenv.

## 1. Written .x3

| File | SHA-256 | Where Guard stores it |
|---|---|---|
| [payment-proposed.x3](../../evidence/near-guard-mainnet-invoice06-2026-11-12/source-recovery/payment-proposed.x3) | `9ee29a5603cfa1e7d9fdbd866cf330ce35e6b5f0907c6844dd2ef1a213768ba0` | `get_intent` policy |
| [mandate-proposed.x3](../../evidence/near-guard-mainnet-invoice06-2026-11-12/source-recovery/mandate-proposed.x3) | `eea021d5cc1118801f4e1d3b9c0efe89fc59a7f654031f9ff1951aa1f2f3d515` | `get_mandate` terms, version 4 |

The payment pays `x3lang.near`, 25 USDC, from the owner's Solana refund wallet, under that mandate, with nonce `mainnet_demo_nov11_invoice_06`.

## 2. Compiler rejection

The written payment compiles. These edits of that file do not:

| Edit | Refusal |
|---|---|
| Drop `require slippage` | `X3_NEAR_SLIPPAGE_REQUIRED` |
| Drop the timeout and refund | `X3_NEAR_TIMEOUT_REQUIRED` |
| Drop the nonce | `X3_NEAR_NONCE` |
| Add a second swap leg | `X3_NEAR_ROUTE_SHAPE` |
| Pay a Cyrillic look-alike of `x3lang.near` | `X3_NEAR_TYPECHECK` (`X3_INVALID_RECEIVER`) |
| Pay an amount of 0 | `X3_NEAR_TYPECHECK` (`X3_INVALID_AMOUNT`) |

A file that does not compile never becomes a quote. The separate nine-edit demo in `x3_wont_compile.py` uses a different example. This stage uses the mainnet payment.

## 3. Deterministic Guard enforcement

Both files say 74 hours. Compiling them at 2026-11-12T01:49:21.152Z, the instant those windows imply, lowers to the mandate terms and the payment policy Guard stored, including both program hashes, the 25 USDC cap, the recipient, the refund wallet, 90 bps slippage, and the 100 bps fee cap.

`check_mandate` is the contract's check, in the same order and with the same names. It allows that policy. The same payment with the recipient changed to `attacker.near` still compiles, and `check_mandate` refuses it with `mandate_recipient` only.

On chain, before any deposit, [`Eg75uehw…u153`](https://nearblocks.io/txns/Eg75uehwU4YJ86ZZPEPKiBqnjkr6PxwPmV23HFAgu153) returned 21 checks, all passing. The first ten are those mandate checks. `mandate_approvals` is 1 of 1. The rest read the signed 1Click quote: live quote, freshness, echoed request, amount, minimum output, slippage, fees, refund fee, deposit address, and deadline. The signer was the restricted key `ed25519:J3NiU8…P11x`, not the owner's FullAccess key.

This command checks the archived receipt. It does not re-verify the quote signature. Guard enforced stored terms. It did not compile `.x3` on chain.

## 4. Real mainnet payment

Acceptance is NEAR block 224149400. The deposit and the payout come after it.

| Step | Transaction | What moved |
|---|---|---|
| Owner approval | [`GFdQLrzT…fsK6`](https://nearblocks.io/txns/GFdQLrzT5tnPPdZwKLtmwpNvSwrxVHDvVg2C3WX5fsK6) | Block 224148959 |
| Agent accepts the quote | [`Eg75uehw…u153`](https://nearblocks.io/txns/Eg75uehwU4YJ86ZZPEPKiBqnjkr6PxwPmV23HFAgu153) | Block 224149400, 21/21 |
| Solana deposit | [`4zhGYyWN…RkXo5`](https://solscan.io/tx/4zhGYyWNabYQ5gixTUzoVHVbgFKWAsQssSpCv3B1J1LjCRisEJW6wm3zuS2GVuDotAT93S8wm1qezHwsKX2RkXo5) | 25.000000 USDC to the authorized deposit address |
| NEAR payout | [`8jfbACLS…w43P`](https://nearblocks.io/txns/8jfbACLSxrzi3fnajfAVGc4Fgo6XLCWA1mpiJWXPw43P) | Block 224150052, 24.995200 USDC to `x3lang.near` |

The owner's Solana wallet signed the deposit. Guard authorized the deposit address and did not hold the USDC.

## 5. Recorded outcome

[`9sTn9pJ1…zuqE`](https://nearblocks.io/txns/9sTn9pJ1ZbCVazBm9Wz9oEDh6aWw4FUVMoAJEKCCzuqE), block 224319990. The same restricted key called `agent_record_outcome` with zero NEAR attached. The finalized readback is `Settled`, `amount_out` `24995200`, `floor_met: true`, and both settlement hashes above.

The earlier public-RPC observation still has `outcome: null`. That file was taken before this call. The sequence uses the later readback and checks that the earlier one is unchanged.

## What this does not prove

- Guard does not compile `.x3` and does not run X3VM.
- Guard does not custody the Solana funds. The owner's recovery key is still on the agent account. It did not sign these calls.
- `floor_met` compares the amount the agent supplied with the quote floor. Guard does not authenticate the supplied transaction hashes.
- Nothing on either chain proves the Solana deposit caused the NEAR payout. The 1Click status record associates them.
- One demo trade of about $25.

The archived chain reads and their checksums are in [the evidence directory](../../evidence/near-guard-mainnet-invoice06-2026-11-12/README.md). A fresh read-only RPC pass of the settlement, not of the later outcome, is `python3 scripts/hackathon/observe_mainnet_invoice06.py`.

## Local forcefield

```bash
python3 scripts/hackathon/forcefield.py
```

Twenty checks beside this sequence: refusal certificates, a denylist, a program-hash pin, a mandate hash chain, an attempt budget, and the rest. They run locally. The deployed Guard state is unchanged. The archived FullAccess recovery key stays under `open`. The [proof page](forcefield-proof.html) recomputes the two file hashes.

## Local enforcement path

```bash
python3 scripts/hackathon/agent_law.py
```

The same two files go through birth, compile, test, authorize, revocation, and court. The [exhibit](agent-law-proof.html) shows the result. The receipt stays on the X3 route. It does not broadcast, and it does not settle this payment a second time. Solana custody stays outside the path.
