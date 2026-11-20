# Research prototypes (not in the judge path)

These files are kept for provenance and future work. They are **not** part of
the NEAR hackathon judge demo, are not deployed, and do not move funds in this
repository.

| Prototype | Status | Where |
|---|---|---|
| NEAR/BSC HTLC and bridge | Research only; storage economics open (issue #71) | [HTLC_STATUS.md](HTLC_STATUS.md), [`x3-lang/bridge/`](../../x3-lang/bridge/) |
| Solana mandate vault | Research only; `program-test` only | [`x3-lang/solana/x3-mandate-vault/README.md`](../../x3-lang/solana/x3-mandate-vault/README.md) |
| Value-fee preview | Non-billable arithmetic preview; no on-chain fee | [`COMMERCIAL_MODEL.md`](../../COMMERCIAL_MODEL.md), [`x3-lang/value_fee.py`](../../x3-lang/value_fee.py) |
| BSC → NEAR bootstrap | Read-only bootstrap notes; no settlement claim | [`docs/hackathon/BSC_TO_NEAR_BOOTSTRAP.md`](../hackathon/BSC_TO_NEAR_BOOTSTRAP.md) |

The submitted on-chain policy surface is [`x3-guard`](../../x3-lang/near/x3-guard),
and the judge path is the offline policy demo (`scripts/hackathon/demo.py`,
`x3-lang/near/demo/judge.html`).
