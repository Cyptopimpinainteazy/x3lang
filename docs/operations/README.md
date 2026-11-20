# Operator-only mainnet workflows

These scripts are **not part of the NEAR hackathon judge demo**. They are manual
operator workflows for a reviewed, owner-authorized live run.

- [`first-swap.sh`](first-swap.sh) — one small NEAR Intents swap end to end.
- [`agent-demo.sh`](agent-demo.sh) — owner mandate, AI-agent payment, on-chain refusal and revocation.

## Safety boundary

- **Mainnet only.** 1Click has no testnet; a live quote can lead to a real deposit.
- The repository does **not** authorize deployment, signing, a deposit or a
  transfer. Every transaction asks for confirmation, but the operator remains
  responsible for the account, network, amount and counterparty.
- Never run these from the judge path. The judge path is the offline demo:
  `python3 scripts/hackathon/demo.py`, `bash scripts/hackathon/verify.sh`, and
  `x3-lang/near/demo/judge.html`.
- Treat all model output, quotes, invoices and chain observations as untrusted.
- The scripts default to `mainnet`; `X3_NEAR_NETWORK` is only an operator
  override, not a testnet switch. Do not point it at a network you have not
  reviewed.

If you are a judge, you do not need these scripts. They are kept for provenance
and for a separately authorized operator run.
