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

- **AI agents under a mandate.** "Try it as an AI agent" loads an agent's payment and its
  owner's mandate. The mandate is checked in the browser before 1Click is asked for anything,
  with the contract's checks in the contract's order. A payment to someone the owner didn't
  list is refused, and each failed term is marked on its line of both programs.

## Run it

```bash
cd x3-lang/near/demo
python3 -m http.server 8000      # ES modules need http://, not file://
# open http://localhost:8000
```

It needs a browser with Ed25519 in WebCrypto: current Chrome, Firefox or Safari. Any static
host serves it; there is no build step.

## Deploy the static demo

Stage the following files in the Cloudflare Pages publish root, preserving paths:

- `index.html`, `index.md`, `judge.html`, `bnb.html`, `x3guard.mjs`
- `judge/judge.mjs`, `judge/scenarios.json`, `judge/bnb-readiness.json`
- `404.html`, `_headers`, `robots.txt`, `sitemap.xml`

After deployment, check that the public home page and `/judge` serve distinct
pages without sign-in, `/judge/scenarios.json` parses as JSON, `/index.md`
returns Markdown, `/sitemap.xml` returns XML, and an unused URL returns HTTP 404.
The public demo’s canonical URL is https://x3-guard.pages.dev/.

## Test it

```bash
node --test x3-lang/near/demo/test/*.test.mjs        # 35 tests, incl. real 1Click signatures
python3 -m pytest x3-lang/tests/test_demo_parity.py  # JS and Python agree (needs Node 20+)
```

## Judge console

`judge.html` (open http://localhost:8000/judge.html) shows one owner mandate deciding nine agent
payments: two allowed (one held for approval) and seven forbidden. The forbidden ones exceed the
cap, pay an unlisted recipient, redirect the refund, raise the fee cap, break the 24-hour cap, run
after revocation, and reuse a nonce. Five panels: the owner's policy, the agent's request, the
decision with each check marked on its line, the cross-chain leg, and the evidence.

- **One checker.** The page runs `x3guard.mjs`'s `checkMandate`, as the X3 Guard page does.
  `judge/scenarios.json` carries Python's `track_control.evaluate` answer for each scenario, and
  the page shows whether the two agree, check by check.
- **Every program is in the repository.** Each scenario is a checked-in example or that example
  with a stated line changed. Two use an assumed Guard state (revoked, already spent), shown as an
  assumption. Nonce reuse is shown as decided by the contract, citing x3-guard's tests, because
  the local checker cannot see used nonces.
- **Each claim is labelled with the evidence behind it.** Only "locally valid policy" holds. The
  page states that no approval, deployment, quote, transaction or settlement is behind it.

```bash
python3 scripts/hackathon/demo.py                         # same scenarios, one terminal command
bash scripts/hackathon/verify.sh                          # tests + demo + readiness report
python3 x3-lang/near/demo/judge/build_scenarios.py            # after changing an example
python3 x3-lang/near/demo/judge/collect_evidence.py           # run the tests; panel 5 shows them
python3 x3-lang/near/demo/judge/collect_evidence.py --with-contract   # also includes the scoped signed-quote refusal test
```

`judge/evidence.json` is a local record and is not committed: it names the commit it ran on, and
a committed copy would always describe an earlier one.

The [judge package](../../../docs/hackathon/JUDGE_PACKAGE.md),
[three-minute walkthrough](../../../docs/hackathon/VIDEO_WALKTHROUGH.md) and
[submission checklist](../../../docs/hackathon/SUBMISSION_CHECKLIST.md) provide the
judge-facing narrative and evidence boundaries.

## What it does not do

It requests **dry** quotes only, so it never holds keys, never gets a deposit address and never
moves funds. For a real swap, `near_intents.py quote --live` and the `x3-guard` contract
(`../x3-guard`) check the same rules again, the contract on NEAR itself.
