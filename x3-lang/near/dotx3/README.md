# .x3 intent workbench for dotx3

This frontend and its compiler API live in **x3lang**. The existing dotx3.net
front door is a separate Grok/Vercel application; this change does not replace
it or claim a deployment there.

## Run

From the repository root, with a project virtual environment containing
`x3-lang/requirements.txt`:

```bash
python3 scripts/hackathon/dotx3_workbench.py
```

Open `http://127.0.0.1:8793`. Load the staged example, compile, then try a
recipient mutation. The real Python frontend compiles both requests; the
mandate denies only the attacker. Restore the original, remove its nonce,
and the compiler refuses before policy evaluation. Inspect the source hashes,
individual checks and JSON report. Download a review packet for the exact
source; it is not a transaction or authorization.

To enable **read-only** mainnet checks explicitly:

```bash
python3 scripts/hackathon/dotx3_workbench.py --enable-mainnet-readonly
```

Supply public owner-reviewed source, mandate, explicit input budget, Guard
account and reviewed deployed Base58 code hash. This invokes the existing
`mainnet_agent_preflight.py`: fixed official token API, mainnet identity,
Guard code-hash observation, recipient storage and signed dry quote checks.
Missing inputs, fee/amount limits and network failures remain blockers.
No live quote, signing, deployment, storage funding, deposit or settlement
endpoint exists. There is no model invocation: source is user-supplied or a
labeled checked-in example. The compiler does not run Rust X3VM on-chain.

## dotx3 integration boundary

The UI is split into `index.html`, `workbench.css`, and `workbench.mjs`.
Its same-origin API contract is:

| Method / path | Inputs | Behavior |
| --- | --- | --- |
| `GET /api/health` | None | Reports compiler availability and whether read-only mainnet checks are enabled |
| `GET /api/examples` | None | Labeled checked-in mandate and payment sources |
| `POST /api/compile` | JSON `program`, `mandate` source strings | Actual Python compile/lower and deterministic policy evaluation |
| `POST /api/dry-preflight` | Sources plus `max_input_usdc`, `guard`, `expected_guard_code_hash` | Opt-in read-only preflight; never wallet authorization |

The supplied development server accepts loopback Host/Origin only. **Do not
publish this development HTTP server as an internet backend.** The dotx3
production app must expose the API through its authenticated/limited server
boundary and run the shared Python evaluator in a bounded worker. Preserve
32 KiB/source and 96 KiB/request limits, same-origin checks, no request-body
logging, no public secrets, and no filesystem paths supplied by clients.
Only the compile API is needed for the public compiler demonstration; network
preflight can remain operator-local. Browser policy preview is not a substitute
for running the Python compiler. No public deploy credentials or editable
source project for dotx3 were available when this component was prepared.

All paths shown in the UI are rooted at `/`. When mounting under a subpath,
update asset/API paths together or map these exact paths in the existing app.
Keep the front door's existing navigation and use this view for its demo route.

## One payment, three tracks

1. **Agent Economy:** show the staged invoice/request and owner mandate; paste
   actual model output only when its provider/model provenance is retained.
2. **AI Security & Defense:** run the allowed original, recipient-only mutation
   and missing-nonce mutation. Show which failure is policy versus compilation.
3. **Unstoppable DeFi:** run read-only readiness with reviewed public inputs.
   Follow `docs/hackathon/MAINNET_AGENT_PREFLIGHT.md` for the separate
   owner-signed funded run and `scripts/hackathon/verify_mainnet_payment.py`
   for read-only finalized source/payout verification. Never mark this intent
   settled from a local pass or a provider SUCCESS string.

The historical Tidepool transfer remains separately labeled testnet evidence.
Guard does not custody the Solana wallet; its keyholder can bypass it.

## Checks

```bash
python3 -m pytest scripts/hackathon/test_dotx3_workbench.py -q
node --check x3-lang/near/dotx3/workbench.mjs
```
