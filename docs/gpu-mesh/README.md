# X3 GPU contributor mesh

Contributed GPUs serve the models behind X3's agents, the Super IDE and dotx3.net. Apps call
one OpenAI-compatible endpoint; contributors run a small daemon next to Ollama that long-polls
for work. Code: [`x3-lang/gpu_mesh/`](../../x3-lang/gpu_mesh/). Tests:
[`x3-lang/tests/test_gpu_mesh.py`](../../x3-lang/tests/test_gpu_mesh.py).

```
 apps (server side only)                coordinator (x3star)                contributors
 ─────────────────────────              ─────────────────────                ─────────────
 near_agent / Team Six   ──┐            POST /v1/chat/completions            worker daemon
 Super IDE backend       ──┼─ app key ─>  route by model, tier, load   <───  long-poll (signed)
 dotx3.net server routes ──┘            check signed result                  └─> local Ollama
                                         append signed, hash-chained receipt
```

## What it guarantees, and what it does not

| Property | How |
|---|---|
| A contributor can't impersonate another | Every worker message is an Ed25519-signed envelope; the key never leaves the machine |
| Captured messages can't be replayed | Per-key nonces plus a ±120 s timestamp window |
| A result can only answer the job it was given | Job id, request hash, assigned worker, model and advertised model digest all have to match, plus a separate result signature |
| Fleet-only apps never reach strangers | App keys default to fleet-only; `--allow-public` is an explicit opt-in per app |
| A contributor that doesn't run the model is caught | Public contributors get canary jobs (first poll, then every 15 min) and get no real work until one passes; two failures in a row suspend them |
| Broken machines drop out | Failed jobs are retried on the next contributor; three failures in a row suspend any machine, fleet included |
| Usage can't be quietly rewritten | Receipts are hash-chained and signed by the coordinator; `admin verify-receipts` recomputes the chain and every worker and coordinator signature |
| Admin can't be attacked over HTTP | There is no admin endpoint; the CLI edits the coordinator's SQLite database on the host |

It does **not** make model output trustworthy. A contributor that runs the model and alters
some answers is not detectable by canaries. That is fine for X3 by design: agents only
*propose* `.x3` programs, and the compiler, the owner's mandate and Guard decide what may
happen (see `docs/hackathon/AGENT_LAW.md`). Responses are labelled `"untrusted_output": true`.

Public contributors **can read the prompts they run**. Don't give `--allow-public` to an app
whose prompts hold anything private (IDE buffers, unreleased code, user data).

GPU details (model, VRAM, utilisation) are self-reported.

## Run it on your fleet

Everything runs from a checkout of this repository with a virtual environment:

```bash
git clone https://github.com/Cyptopimpinainteazy/x3lang /opt/x3lang && cd /opt/x3lang
python3 -m venv .venv && .venv/bin/pip install -r x3-lang/requirements.txt
export PYTHONPATH=/opt/x3lang/x3-lang
alias mesh='/opt/x3lang/.venv/bin/python -m gpu_mesh'
```

**1. Coordinator (x3star).** It binds loopback; publish it with a Cloudflare tunnel or reverse
proxy at e.g. `https://gpu.x3star.net`. Do not put Cloudflare Access in front of `/mesh/*` or
`/v1/*`: workers and apps authenticate themselves.

```bash
mesh coordinator serve --db /var/lib/x3-gpu-mesh/mesh.db --key /var/lib/x3-gpu-mesh/coordinator.key
# or: x3-lang/gpu_mesh/deploy/x3-gpu-mesh-coordinator.service
```

**2. Each GPU box.** Install Ollama, pull the models, start the worker, and enroll its key:

```bash
ollama pull qwen3:8b
mesh keygen --out ~/.x3/gpu-mesh/worker.key            # prints the public key
mesh worker run --key ~/.x3/gpu-mesh/worker.key --coordinator https://gpu.x3star.net \
    --label x3gpu1 --models qwen3:8b
# or: x3-lang/gpu_mesh/deploy/x3-gpu-mesh-worker.service

# on the coordinator host:
mesh admin enroll-fleet <public key> --label x3gpu1 --db /var/lib/x3-gpu-mesh/mesh.db
mesh admin list --db /var/lib/x3-gpu-mesh/mesh.db
```

On a private LAN you can point workers at `http://10.x.x.x:8790` with `--allow-http`; anywhere
else the worker refuses plain HTTP. App keys travel in the Authorization header, so apps
should always use HTTPS too.

**3. App keys.** One per app, shown once, kept server-side:

```bash
mesh admin app-add agents --rpm 60  --daily-tokens 2000000 --db ...
mesh admin app-add ide    --rpm 120 --daily-tokens 5000000 --max-tokens 4096 --db ...
mesh admin app-add site   --rpm 30  --daily-tokens 1000000 --max-tokens 1024 --allow-public --db ...
mesh admin app-list --db ...
```

## Using it from apps

**X3 agents (no code change).** `near_agent.py` already speaks OpenAI-compatible:

```bash
export X3_AGENT_PROVIDER=openai-compatible
export X3_AGENT_BASE_URL=https://gpu.x3star.net/v1
export X3_AGENT_MODEL=qwen3:8b
export X3_AGENT_API_KEY=<agents app key>
```

**Super IDE and dotx3.net.** Call `POST /v1/chat/completions` from the server (the IDE's
Python backend, a dotx3.net server route), never from the browser: the coordinator sends no
CORS headers on purpose, so an app key can't be lifted from page source. `stream: true` is
accepted and returns standard `chat.completion.chunk` events (today the whole answer arrives
as one content chunk, then the finish chunk). Each response carries an `x3_mesh` receipt with
the contributor, model digest, request and response hashes and both signatures.

Accepted: `model`, `messages` (text only), `temperature`, `top_p`, `max_tokens`, `seed`, `stop`,
`stream`, `stream_options.include_usage`, `user`. Refused with a 400 rather than silently
ignored: tools/function calling, `response_format`, logprobs, `n > 1`, image parts.

## Opening it to the public

1. The contributor installs Ollama and the worker, and runs it with `--payout-account
   <their.near>`. It registers as `public` / `pending`.
2. You review it (`mesh admin list`) and `mesh admin approve <key>`.
3. Its first poll gets a canary. Once one passes it can serve apps created with `--allow-public`.
4. `mesh admin suspend`/`revoke` take effect on the next request. A revoked key can never come
   back; the contributor has to make a new one and be approved again.

## Paying contributors

```bash
mesh admin ledger --db ... --key /var/lib/x3-gpu-mesh/coordinator.key \
    --since 2026-11-01 --until 2026-12-01 --out ledger-2026-11.json
```

The ledger refuses to build if the receipt chain doesn't verify. It lists each contributor's
jobs, tokens, busy time, canary results, apps served and payout account, and is signed by the
coordinator key (`gpu_mesh.ledger.verify`). It is accounting only: nothing in the mesh moves
funds. You pay from your own wallet after reviewing it.

## Admin reference

`mesh admin list | enroll-fleet | approve | suspend | revoke | set-tier | app-add | app-revoke |
app-list | verify-receipts | ledger | events` (all take `--db`). `mesh pubkey --key FILE` prints a
key's public half.

## Known limits

* Streaming is buffered: tokens arrive in one chunk once the job finishes, not token by token.
* The coordinator is a single process (Python `ThreadingHTTPServer`, SQLite). Sized for tens of
  contributors and modest request rates, not a public inference marketplace.
* Canaries use a fixed prompt shape; a determined cheater could learn to recognise them.
  They catch machines that don't run the model, not targeted manipulation.
* Rewards are computed, not paid; staking and slashing are not implemented.
* Jobs handed to workers are not signed by the coordinator; a worker relies on HTTPS to know
  it is talking to the real coordinator. (A forged job can only make a worker run inference
  and hand the answer to the forger. It can't touch funds or files.)
* Unknown keys are refused before anything is stored. Open registration challenges (2,000)
  and pending public registrations (500) are capped, and enrolled fleet machines are never
  blocked by the public queue.
