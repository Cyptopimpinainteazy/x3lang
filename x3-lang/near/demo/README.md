# X3 Guard demo

**Live: https://x3-guard.pages.dev**

One static page: edit an X3 intent, get a real 1Click dry quote, and see every rule checked.
Each verdict is shown next to the line of the program it came from.

- **Nothing to install, no keys, no funds.** The page calls the 1Click API (`/v0/tokens`,
  `/v0/quote` with `dry: true`) and NEAR RPC (`view_account`) directly from the browser.
- **The signature is checked first.** `x3guard.mjs` rebuilds the payload 1Click signs, the
  same way `@defuse-protocol/one-click-sdk-typescript` 0.1.26 does, and verifies it with
  the browser's WebCrypto Ed25519 under the pinned manager key. A forged or altered quote stops
  there, and nothing else in it is read.
- **Same answers as the Python adapter.** `x3guard.mjs` ports the lowering and guards of
  `x3-lang/near_intents.py`. `x3-lang/tests/test_demo_parity.py` runs both on 16 programs,
  2 real 1Click quotes and 15 live-quote cases, and requires identical results.

## Run it

```bash
cd x3-lang/near/demo
python3 -m http.server 8000      # ES modules need http://, not file://
# open http://localhost:8000
```

It needs a browser with Ed25519 in WebCrypto: current Chrome, Firefox or Safari. Any static
host serves it; there is no build step.

## Test it

```bash
node --test x3-lang/near/demo/test/*.test.mjs        # 10 tests, incl. real 1Click signatures
python3 -m pytest x3-lang/tests/test_demo_parity.py  # JS and Python agree (needs Node 20+)
```

## What it does not do

It requests **dry** quotes only, so it never holds keys, never gets a deposit address and never
moves funds. For a real swap, `near_intents.py quote --live` and the `x3-guard` contract
(`../x3-guard`) check the same rules again, the contract on NEAR itself.
