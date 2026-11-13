# Mainnet settlement under an X3 Guard mandate: invoice_06

**Network:** NEAR mainnet and Solana mainnet. **Nonce:** `mainnet_demo_nov11_invoice_06`.
**Value:** 25 USDC in, 24.9952 USDC out (about $25).

A restricted agent key accepted a signed 1Click quote on the mainnet X3 Guard after Guard ran
21 checks against the owner's mandate, all passing. The owner's wallet then funded the
authorized deposit address on Solana and NEAR Intents paid out to `x3lang.near` 84 seconds
later.

[`public-rpc-observation.json`](public-rpc-observation.json) is the raw output of
[`scripts/hackathon/observe_mainnet_invoice06.py`](../../scripts/hackathon/observe_mainnet_invoice06.py),
run read-only on GitHub Actions against public NEAR and Solana RPC
([run 43307893891](https://github.com/Cyptopimpinainteazy/x3lang/actions/runs/43307893891)),
observed 2026-11-12 21:15 UTC. 18 checks passed, 0 failed, 1 cannot be read on-chain.
Checksum in [`SHA256SUMS`](SHA256SUMS). It was not edited by hand.

## Transactions

| Step | Chain | Transaction | Block / slot | Signer (key) | Result |
|---|---|---|---|---|---|
| Owner approval (`approve_intent`) | NEAR | [`GFdQLrzT…fsK6`](https://nearblocks.io/txns/GFdQLrzT5tnPPdZwKLtmwpNvSwrxVHDvVg2C3WX5fsK6) | 224148959 | `x3lang.near` | final, success |
| Agent storage credit (`fund_agent_storage`) | NEAR | [`Ax9yNKUi…9ZA`](https://nearblocks.io/txns/Ax9yNKUicGoyR3fcStygFQypUQkmfSKEFYDmRxbK9ZA) | 224149317 | `x3lang.near` | final, success |
| Agent accepts quote (`agent_accept_quote`) | NEAR | [`Eg75uehw…u153`](https://nearblocks.io/txns/Eg75uehwU4YJ86ZZPEPKiBqnjkr6PxwPmV23HFAgu153) | 224149400 | `x3-agent.x3lang.near` (`ed25519:J3NiU8…P11x`, FunctionCall) | final, success, 21/21 guards ok |
| Fee top-up (context) | Solana | [`2BsJuqvq…6Hp7`](https://solscan.io/tx/2BsJuqvqrY2pz9avVf4ix24kA8pUhBWgRjP7qJC4h3rP2o4JqZdjAt5ePEsDqqpjtPc5bNrYmdQhhk1Fgp286Hp7) | 468345798 | `BzyMJz…pH5Rf` | 0.197 SOL to `58tbMf…B3cQ` |
| Deposit | Solana | [`4zhGYyWN…RkXo5`](https://solscan.io/tx/4zhGYyWNabYQ5gixTUzoVHVbgFKWAsQssSpCv3B1J1LjCRisEJW6wm3zuS2GVuDotAT93S8wm1qezHwsKX2RkXo5) | 468354078 | `58tbMf…B3cQ` | finalized; 25.000000 USDC to `C6978A…UdkN` |
| Payout (`execute_intents`) | NEAR | [`8jfbACLS…w43P`](https://nearblocks.io/txns/8jfbACLSxrzi3fnajfAVGc4Fgo6XLCWA1mpiJWXPw43P) | 224150052 | `intents.near` | final; `ft_transfer` 24995200 to `x3lang.near` |

The Guard on `x3lang.near` runs code hash `BPdpJAw8…Fb15T`, deployed in
[`BVXX3hJE…3faF`](https://nearblocks.io/txns/BVXX3hJEmThWVZNpUknZv9HCpGezisiN8SQKsZgA3faF).
Its WASM (SHA-256 `9a614640…a632`) was built from reviewed commit `9dcea3c`, whose Guard
source is byte-identical to `x3-lang/near/x3-guard/` on `master`
([deployment evidence](../near-guard-mainnet-2026-11-11/deployment-verification.json), PR #207).

## What the chain shows

- **The agent used its restricted key, not the owner's.** `Eg75…` was signed by
  `ed25519:J3NiU8…P11x`, a FunctionCall key limited to `agent_register_intent`,
  `agent_accept_quote` and `agent_record_outcome` on `x3lang.near`. The agent account also
  holds the owner's FullAccess recovery key `ed25519:8teaX…c23u`, which did not sign this call.
- **Guard checked the mandate and the signed quote before the deposit.** Mandate v4 allows one
  recipient (`x3lang.near`), one source and one destination asset, a 25 USDC per-payment and
  daily cap, a pinned refund wallet, 100 bps slippage and fee ceilings, and requires one
  owner approval. The returned guard list includes `mandate_recipient`, `mandate_per_payment`,
  `mandate_daily`, `mandate_refund_to`, `mandate_approvals` (1 of 1), `quote_kind` (live,
  not dry), `min_receive` (floor 24750000, guaranteed 24770243) and `deposit_address`.
- **Order.** Approval (block 224148959) and acceptance (224149400) precede the deposit
  (Solana block time 02:15:13 UTC) and the payout (224150052).
- **Stored terms.** `get_intent` returns the agent-registered policy with
  `program_sha256 = 9ee29a56…68ba0` and the accepted quote's facts; `get_mandate` returns
  mandate `program_sha256 = eea021d5…d515`.
- **Source-byte link.** Both [original `.x3` files](source-recovery/README.md) were recovered
  verbatim and SHA-256 verified against fresh finalized `get_intent` and `get_mandate`
  responses for mandate v4.
- **Delivery.** `intents.near` emitted `ft_withdraw` and `ft_transfer` of 24995200 native
  USDC to `x3lang.near`; `ft_balance_of(x3lang.near)` read 24995200 at observation.

## What it does not show

- **On-chain human identity.** [Recovered first-party operator records](source-recovery/README.md#funding-wallet-control)
  identify `58tbMf…B3cQ` as the owner-operated Solflare wallet; the owner manually signed
  invoice_06. The inspected backend configures a separate restricted NEAR key. The chain
  verifies the Solana signer and transfer, but cannot identify its human controller.
- **Custody.** Guard authorized the deposit address; it did not hold or move the USDC. A
  holder of the funding wallet's key could have paid without Guard.
- **Cross-chain causality.** The 1Click status record associates the Solana deposit with the
  NEAR payout. Nothing on either chain proves that link cryptographically.
- **Compilation and authorship.** The [recovered original `.x3` files](source-recovery/README.md)
  match both stored hashes in fresh finalized Guard reads. Hash identity does not prove
  on-chain compilation or how the source was authored.
- **Outcome on Guard.** `agent_record_outcome` has not been called; `get_intent` shows no outcome.
- **Scale.** One small trade, labelled `demo` in its nonce.
- **The `agent_register_intent` transaction** is not listed; its stored result is read via `get_intent`.

## Reproduce

```bash
python3 scripts/hackathon/observe_mainnet_invoice06.py --out /tmp/observation.json
```

Read-only, standard library only, needs outbound HTTPS to public RPC. Live reads (balance,
key allowance, final block) change over time; transaction records do not.
