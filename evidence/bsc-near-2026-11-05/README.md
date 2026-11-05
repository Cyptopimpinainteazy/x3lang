# BSC → NEAR quote evidence — 2026-11-05

Command:

```bash
PYTHONPATH=x3-lang python3 x3-lang/near_intents.py quote \
  x3-lang/examples/bsc_bnb_to_near_wnear_dry.x3 \
  --out evidence/bsc-near-2026-11-05/quote-001-bnb.json
```

Result: exit 1.

The live provider token catalog resolved BSC native BNB and NEAR wNEAR. The NEAR RPC
recipient check for `wrap.near` passed. 1Click then refused the quote with:

```text
HTTP 400: Temporary swap limits: minimum swap amount is $1,000
```

The structured JSON retains the exact request, source SHA-256, provider correlation ID,
provider minimum and account preflight. The example contains only intentionally non-owned
placeholder addresses. No key, JWT, nonce store, signed transaction or deposit is present.

This file proves provider behavior only. It is not a quote, authorization, transaction or
settlement.
