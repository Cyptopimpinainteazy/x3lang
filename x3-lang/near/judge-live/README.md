# X3 Shield Live: judge site

Static site for NEAR HACKATHON 2026 judges. Judges send requests to the agent, watch
Guard approve or reject them on NEAR mainnet, and check every result against NEAR RPC
from their own browser.

## Files

| File | What it is |
|---|---|
| `index.html`, `styles.css`, `app.js` | The site. No build step, no framework. |
| `config.json` | Network, API base, RPC endpoints, accounts, setup transactions, featured `.x3` program. |
| `evidence/mainnet/feed.json` | Static copy of the attempt feed, shown when the runner is offline. The runner appends to it. |
| `API.md` | The runner API this site calls. Implement it exactly. |

Also here: `console.html`, the earlier single-file Guard console (mandate terms, local
policy playground, and RPC decoding of the two mainnet setup transactions). It needs
no runner.

## Not deployed

Nothing in this folder is served yet, and it is not linked from dotx3.net. The runner API
in `API.md` doesn't exist yet, so `index.html` opens in its offline state until it does.

## Already filled in

- `programs/base_sepolia_to_near_testnet_1usdc.x3` is a byte-for-byte copy of
  `x3-lang/examples/`. Its SHA-256 `1398f64e…df860` is the `.x3` source hash bound on
  Guard (`docs/hackathon/FIRST_FINALIZED_SWAP_EVIDENCE.md`); the site recomputes it in
  the browser.
- `featuredProgram.txs`: the Base Sepolia source transfer, the NEAR testnet Guard
  approval block, and the NEAR testnet maker-signed settlement, from the same evidence file.
- `test/config.test.mjs` checks all of the above: `node --test x3-lang/near/judge-live/test/*.test.mjs`.

## Before going live

1. Keep `programs/` in sync with `x3-lang/examples/` (the test fails if they differ).
2. Add each new owner-signed setup transaction (vault deploy, new mandate, agent key
   scoping, DX3HACK deploy) to `setupTxs` with its signer.
3. Check that `accounts` matches what exists. The site reads each account from chain
   and flags the agent if it has any full-access key.
4. Set `turnstileSiteKey` to the Cloudflare Turnstile site key. The runner verifies
   the token. Leave it empty only for local testing.
5. Implement the runner per `API.md` and serve it under `/api` on the same origin (reverse proxy), or set `apiBase`
   to its URL and allow CORS from the site's origin.
6. Remove Cloudflare Access from this hostname. Judges must reach it logged out,
   through at least Nov 25 2026.

## What the site shows when things aren't ready

- Runner unreachable: Try It is disabled with a plain message, and the feed falls back
  to `evidence/mainnet/feed.json`. Account, mandate-transaction and program checks
  still read NEAR directly.
- Runner paused (daily cap spent, low gas, owner pause): Try It shows the runner's
  `pauseReason`.
- Vault or merchant not created yet: their cards say "Not created yet".
- Program file missing: the program panel says where it expects the file.

## Local check

```
python3 -m http.server 8080
# open http://localhost:8080
```

Without a runner it opens in its offline state, which is what judges see if the
runner goes down.
