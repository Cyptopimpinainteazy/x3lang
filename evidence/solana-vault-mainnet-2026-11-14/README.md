# Mainnet vault pilot: 5 USDC from Solana, 4.999746 USDC on NEAR

On November 14, 2026, a dedicated agent key signed an X3 Solana vault execution
that transferred **5.000000 native USDC** from the policy-PDA-owned token account
to the signed 1Click deposit account. The provider reported **SUCCESS** and
associated that source transaction with a finalized NEAR payout of
**4.999746 USDC to x3lang.near**, above the signed minimum **4.974757 USDC**.

The owner signed deployment, setup/funding and exact quote approval in Solflare.
The agent signed execution; the owner did not sign that execution transaction.
This is an **owner-approved single-quote vault pilot**, not unrestricted agent spending.

## Public transaction trail

| Step | Mainnet transaction | Evidence |
| --- | --- | --- |
| Deploy reviewed bytes | [Solana deployment](https://explorer.solana.com/tx/62a2Ufv9Qtw1HTNniwm2RTmD9Q3J7GfiCUrKXeCQkWULmWoDwPkJhPMwES2HENacjSLFNsX2ZZVD7c2LWvfweqtJ) | [deployment.json](deployment.json), [signed transaction](deployment-transaction.json) |
| Create policy; fund vault with 5 USDC and agent with 0.001 SOL | [Owner setup](https://explorer.solana.com/tx/32FhzL7fxQKPgTTopxjJnwGXMpTgaGVgAzDDsQkevw4YY1kDvKuHdPmjV773ynWcr9m6dPofuedKRPin8abPh9Un) | [setup.json](setup.json), [signed transaction](setup-transaction.json) |
| Approve exact nonce, amount, destination, digest and expiry | [Owner quote approval](https://explorer.solana.com/tx/4ReKsqwxpwtyx2CNmah6UCkHWgb4GbiJjXGTZLwtHaMMBoWv8vqSki4XXzDrBmqgURJSX7A1fHAKm3jSf3fnWwkV) | [finalized-approval.json](finalized-approval.json), [signed transaction](approval-transaction.json) |
| Execute from the vault, agent signature only | [5-USDC source transfer](https://explorer.solana.com/tx/3Bu3eWLwiSLoTNHgLPCzn5JyTAUnfzbXJENQG36oEt7kZm9e1M4rRB5CtUZxG92gRaGmPvuFZUQ1EqBzkL9rdLBf) | [Finalized source transaction and vault event](finalized-execution-primary.json), slot 469314055 |
| Pay native USDC to x3lang.near | [NEAR payout](https://nearblocks.io/txns/CXoXgJ5FwcSZcyPDVu7PrNby7KULfY8bJxUBRT9fu4ia) | [Finalized transaction](near-payout-transaction.json): token contract NEP-141 ft_transfer event, 4999746 base units |

## Accounts and exact authority

| Field | Value |
| --- | --- |
| Owner; refund wallet; retained upgrade authority | `58tbMfadmi8HWzUndLqkX3prBbkiZMEaYMTDYSzhB3cQ` |
| Dedicated agent | `6R4nioW9tPke3PFuWLaQY3gRAwM5SMi1vbY925WJZtPS` |
| Executable program | `68ssmMccnqQFUHWipQKy1W4DcycjA9jip9ZUi7EJyUZn` |
| Policy PDA | `ATXS8Y9cDVAN3BxTDa7CXt7FuHU7Y6LJHDqGDDuQtXGC` |
| Vault token account; SPL authority is the policy PDA | `FTToXuCe3RG6L78Ctpno5SL5BpXDNAYc6JnjhjqGWndU` |
| Native Solana USDC mint | `EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v` |
| Signed deposit owner | `HR876DcfqzX5NmYADVvJZQRKjkchpRSTuracYFQsm3G1` |
| Deposit token account | `9BCGvA3kLoHCGv93hLUMK4HqSXmi4ghdNJDa7aYGMZvd` |
| Deployed payload SHA-256 | `aa96de859a8d29c4e41eece9ff336545f3f99e40c8412f78bc16dd25b562c270` |
| Quote digest bound in owner approval and vault event | `e53cd55f8ef6b923fe312fa213e6f9f43a0b6d4f230b411f59e58c1589b11b3d` |
| Quote hash | `GRrAXUy1vFCjPZ53mvX9dMLhXZ7NXgbyekmMPod7tw2L` |

The policy caps each payment and the rolling 24-hour spend at **5 USDC**.
Its expiry is **2026-11-15 11:19:18 UTC**. The exact payment was nonce **1**,
with approval cutoff **2026-11-14 12:01:49 UTC**. The finalized post-state
shows vault balance **0**, no pending approval, last_nonce **1**, one
5-USDC spend, and agent balance **995000 lamports**. See
[finalized-accounts.json](finalized-accounts.json) and [settlement-summary.json](settlement-summary.json).

## What ran and what was proven

- The operator-written [payment-proposed.x3](payment-proposed.x3) was parsed,
  type-checked and used to check the authenticated quote off chain.
  Its source hash matches the saved [live receipt](live-quote.near-receipt.json).
  This pilot does not claim an AI generated that source or that X3VM executed on chain.
- The owner-approved destination and digest match the manager-signed quote.
  The agent execution contains one vault instruction. Its finalized receipt
  contains the matching X3 vault event and inner classic SPL Token
  TransferChecked for exactly 5000000 units, authorized by the policy PDA.
- The provider recognized this specific CPI-funded deposit and associated
  its source signature with the payout. This demonstrates compatibility for
  **this one native-USDC pilot**, not every token, provider or CPI route.
- The final NEAR receipt is successful and contains the correct token-contract
  event paying 4999746 units from intents.near to x3lang.near.
  The independently read balance increased from 24995200 to 29994946 units.
- [Unsigned mainnet simulations](agent-execution-simulation.json) exercised
  the deployed program: success, wrong digest, wrong agent and replay.
  Wrong-digest/agent failures were Custom 11/1; replay returned Custom 10.
  These refusal tests are simulations, not paid failed transactions.
  The post-execution replay simulation in the summary also returned Custom 10.

## Costs and quote timing

The signed quote expected 4.999756 USDC; the observed payout was 4.999746 USDC.
The quoted refund fee was 0.010034 USDC; **no refund occurred**.
Owner SOL debits: deployment **0.000838120**, setup **0.009788320**,
approval **0.001493440**. These include rent and the **0.001 SOL** agent
funding, not just network fees. Agent execution paid **0.000005 SOL**.
Deposit-ATA rent of **0.001488440 SOL** funded a provider-owned token account;
this bundle makes no owner-recovery claim for that rent.

The provider returned a quote deadline 72 hours after the signed request deadline.
The proposal used a 74-hour program timeout and narrowed the requested deadline
to 30 minutes; the standard quote signature, echo, amount, fee, slippage and
program-timeout checks passed. The Solana approval used the **earlier request
deadline**, not the provider deadline of **2026-11-17 12:01:49.430 UTC**.
The original receipt is preserved unchanged.

## Reproduce the archived checks without a wallet

Use Python 3.12+ and the repository Python sources. The only additional package
for decoding the archived signed Solana packets is solders 0.26.0:

```bash
python3 -m venv /tmp/x3-vault-proof-env
/tmp/x3-vault-proof-env/bin/python -m pip install solders==0.26.0
/tmp/x3-vault-proof-env/bin/python evidence/solana-vault-mainnet-2026-11-14/verify_archived.py
```

Run the tamper regressions with:

```bash
/tmp/x3-vault-proof-env/bin/python -m unittest discover -s evidence/solana-vault-mainnet-2026-11-14 -p test_verify_archived.py -v
```

The verifier reads only local files. It checks the manifest, source identity,
compiled payment fields, provider signature and guards at historical issuance,
owner and agent signatures, exact source event/CPI, deployed byte hash, consumed
policy, finalized payout event, balance delta and provider association.
It never connects a wallet, reads keys, signs or sends transactions.
[manifest.json](manifest.json) records the archived file hashes.

## Explicit limits

1. The proof observes finalized RPC responses, not a light-client consensus proof.
   Source verification used **one Solana RPC**. Attempts at distinct secondary
   endpoints failed or required credentials; [quorum verification did not pass](source-quorum-verification.json).
   The payout was read through one NEAR RPC.
2. The provider associates the two finalized legs. This is **not cryptographic
   solver-level causality or cross-chain atomicity**.
3. This pilot did **not** register or record a new outcome on NEAR Guard. The
   source spending boundary was the Solana policy PDA. Earlier invoice_06
   NEAR Guard evidence remains a separate demonstration.
4. The vault does not authenticate the provider's Ed25519 signature on chain.
   Off-chain verification and the owner's exact approval bind that digest.
   Owner and upgrade authority remain trusted. No independent external
   contract audit, broad production reliability or source-to-SBF reproducible
   build identity is established by this bundle.
5. The generic near_intents._signed_quote receipt verifier rejects the narrower
   request deadline differing from the compiled leg deadline. **It did not pass.**
   Vault quote commitment, source event/CPI and direct NEAR payout checks were
   performed separately; the offline verifier labels this distinction.
6. Archive hashes detect byte changes; they do not authenticate an RPC operator,
   guarantee fresh state or establish that the repository revision built the SBF.
   [capture-environment.json](capture-environment.json) identifies the packaging base.
7. This was one owner-approved 5-USDC payment. It authorizes no repeat payment.
