# Recovered invoice_06 source and live Guard hash verification

The original payment and mandate source bytes were recovered on `x3gpu1` from
`/home/lojak/Documents/ChatGPT/x3langnearhackathon/release-evidence/mainnet-deadline-resolution-20261111/`.
They are copied verbatim, including their final newlines. They were not reconstructed
from the stored policy or edited to match a checksum.

| Source | Bytes | SHA-256 |
|---|---:|---|
| [payment-proposed.x3](payment-proposed.x3) | 440 | `9ee29a5603cfa1e7d9fdbd866cf330ce35e6b5f0907c6844dd2ef1a213768ba0` |
| [mandate-proposed.x3](mandate-proposed.x3) | 396 | `eea021d5cc1118801f4e1d3b9c0efe89fc59a7f654031f9ff1951aa1f2f3d515` |

[guard-source-verification.json](guard-source-verification.json) records fresh,
read-only `call_function` responses with `finality: final` from
`https://free.rpc.fastnear.com` on `x3lang.near`:

- `get_intent({"owner":"x3lang.near","nonce":"mainnet_demo_nov11_invoice_06"})`,
  block 224318792: `policy.program_sha256` equals the recovered payment SHA-256;
  `mandate_version` is 4.
- `get_mandate({"owner":"x3lang.near","agent":"x3-agent.x3lang.near"})`,
  block 224318793: `terms.program_sha256` equals the recovered mandate SHA-256;
  `version` is 4.

This establishes the exact source-byte link to the mainnet Guard records. It does
not by itself prove how those bytes were authored, that Guard compiled source on
chain, wallet custody, or cryptographic causality between settlement legs.

## Verify the archived byte link

From this directory:

```bash
sha256sum --check SHA256SUMS
python3 - <<'PYVERIFY'
import hashlib, json
from pathlib import Path
r = json.loads(Path("guard-source-verification.json").read_text())
i, m = r["intent"]["value"], r["mandate"]["value"]
assert i["mandate_version"] == m["version"] == 4
assert i["nonce"] == "mainnet_demo_nov11_invoice_06"
for name, stored in [("payment-proposed.x3", i["policy"]["program_sha256"]),
                     ("mandate-proposed.x3", m["terms"]["program_sha256"])]:
    assert hashlib.sha256(Path(name).read_bytes()).hexdigest() == stored
    print(name, "matches archived finalized Guard response")
PYVERIFY
```

For fresh network verification, import `near_view` from the repository's
`scripts/hackathon/observe_mainnet_invoice06.py` and repeat the two calls above;
compare their returned `value` hashes to `sha256sum` of these files. The report
records the original base commit, endpoint, observation time and block hashes.
No keys, private quotes, wallet files or API credentials are included.

## Funding wallet control

The invoice_06 funding/refund wallet `58tbMfadmi8HWzUndLqkX3prBbkiZMEaYMTDYSzhB3cQ`
was owner-operated in Solflare. The owner manually signed the 25 USDC deposit and
provided transaction `4zhGYyWNabYQ5gixTUzoVHVbgFKWAsQssSpCv3B1J1LjCRisEJW6wm3zuS2GVuDotAT93S8wm1qezHwsKX2RkXo5`.
This attribution comes from recovered first-party operator records, not an
on-chain identity proof or an assertion that Guard controlled the wallet.

Local provenance checked on `x3gpu1` (private files intentionally not copied):

- The original `source-transfer-owner-review.json` in the source evidence directory
  names this wallet, requires a wallet signature, and states that the agent did
  not send the deposit. This is a pre-send review, not by itself execution proof.
- The original operator conversation records instructions to sign in Solflare,
  the owner's send confirmation, and the owner supplying the transaction above.
  The public Solana transaction in the parent evidence independently verifies
  the signer and the transfer; it does not identify a human controller.
- The inspected `mainnet-control/agent-runtime.json` configures the separate
  restricted NEAR agent `x3-agent.x3lang.near` with public key
  `ed25519:J3NiU8J63HAfz2MTDu1zQVhFRYS5Pio21h1kNtJdP11x`.
- No mainnet Solana signer for this address was configured in the inspected
  runtime, `.x3`, and subsequent model-run evidence. The two local devnet wallet
  public addresses differ. The subsequent model run records no signing or
  broadcasting. This bounded search cannot establish the absence of all copies
  of a wallet key.

At the source-recovery observation, Guard's `get_intent.outcome` was null. The
source recovery itself signed no calls. The [subsequently authorized outcome
recording](../outcome-recording/README.md) now establishes finalized `Settled` state.
