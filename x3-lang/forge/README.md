# X3 Agent Forge

A video-game-style character builder for X3 agents. You pick a species, learn skills from a skill
tree, plug the agent into its connections, and an alien grows from what you built. The build
compiles to a real `.x3` mandate and an Agent Law runtime manifest. The creature is a picture of
those compiled artifacts and nothing else.

**Status: implemented and tested. Deploy script for forge.dotx3.net included; not on chain.** The Forge prints the
x3-guard `create_mandate` / `revoke_mandate` calls. The owner signs and sends them. The Forge
never holds a NEAR key, an operator private key or an agent private key.

## What is real

| Stage | What runs | Code |
|---|---|---|
| Compile | Generated `.x3` mandate → the repository's mandate compiler (same lowering as `near_intents.py mandate args`) | `core.py` → `near_mandate.compile_mandate` |
| Proving Ground | Real agent intents → `compile_program` → dry 1Click request → x3-guard checks (`mandate check`); delegation attempts → Agent Law admission | `proving.py` → `near_intents.check_against_mandate`, `near_mandate.check_mandate`, `agent_constitution.check_admission` |
| Birth | Operator-signed genesis in the Agent Law runtime. The signed nonce carries the build hash. | `store.py` → `agent_runtime.Runtime.genesis` |
| Revoke | Operator-signed runtime revocation, plus the `revoke_mandate` call for the owner | `store.py` → `Runtime.revoke` |
| Mesh | Agent-signed messages through the Forge mesh connection, refused unless the agent is active and holds the capability, surface, live connection, peer and message type | `store.py` → `mesh_send` |

Three probes supply chain state the compiled file cannot hold: an expired clock, prior spending and a
revoked mandate. They say so in the UI. See `FINDINGS.md` for the gaps: no cooldown term, and
non-payment capabilities and connections recorded but not enforced by a production adapter.

## Run it

```bash
cd x3-lang/forge/web && npm ci && npm run build && cd -
PYTHONPATH=x3-lang python3 -m forge.server --data ~/.x3-forge --port 8787
# prints: High Command is unclaimed. One-time setup token: …
```

Open `http://127.0.0.1:8787`, go to **Birth**, and paste the token. This creates the High Command
Ed25519 key in your browser (non-extractable, stored in IndexedDB) and claims the Forge. Only
that browser profile can then birth or revoke agents.

Options:
- `--tokens FILE` or `--live-tokens`: 1Click token list (default: the committed snapshot
  `tests/fixtures/near_intents_tokens.json`).
- `FORGE_EXTRA_DOMAINS=example.com,…`: allow more connector domains. The default allowlist is
  dotx3.net, x3star.net, x3star.org, nearblocks.io and chaindefuser.com.
- `FORGE_SETUP_TOKEN`: choose the claim token instead of a random one.
- `--secure-cookie`: set this when the host is served over HTTPS.

The host binds to 127.0.0.1 and refuses any other address unless `FORGE_ACCESS_MODE=access`,
which makes it refuse every request without the owner's Cloudflare Access JWT
(`FORGE_ACCESS_TEAM_DOMAIN`, `FORGE_ACCESS_AUD`, `FORGE_OWNER_EMAIL`). To host it at
`forge.dotx3.net`, use `deploy/forge/deploy.sh up` (see `deploy/forge/README.md`).

## Connections

Each connector card opens its page with `x3_forge_state` and `x3_forge_return` in the query. A page
that supports the Forge sends the owner back to `/api/connect/callback?connection=…&state=…`. The
state is one-time, session-bound and expires after 10 minutes. No X3 page implements the return
yet, so the card also offers **I finished setup**, and the connection is labelled
"owner confirmed". Health checks run from the host: a redirect to Cloudflare Access counts as
reachable and login-gated, and nothing else is followed. API keys are stored only under
`<data>/secrets/` with mode 0600. They never appear in manifests, exports or logs, and a test
checks this.

## Mind: thinking on the X3 GPU mesh

The **Mesh Mind** skill compiles to the runtime capability `ai.infer`, enforced by the Forge
inference gate (`inference.py`). Its parameters are fixed at birth: the models the agent may use,
max tokens per request and a token budget per rolling 24 h. The **X3 GPU mesh** connector
(`gpu_mesh`) points at a `gpu_mesh` coordinator. Its app key is stored server-side like any other
key, and the coordinator's `/healthz` is the health check.

Every request is signed by the agent's own key, over a payload the host builds:
`X3/FORGE/INFER/1` for one raw completion, `X3/FORGE/RUN/1` for a task. Before the mesh is
called, the gate refuses on any of these: signature, expiry, replay, agent not active, missing
capability or surface, missing or revoked mesh connection, model not on the list, max_tokens over
the cap, budget exhausted. Refused requests never reach a GPU. The claim (nonce plus full
`max_tokens`) is persisted before the call. A parallel or replayed request is refused, and a request
stranded by a crash stays charged. One signed request authorizes exactly one model call.

A **run** is `near_agent.run`, unchanged, with the gate as its model. The model output is untrusted.
Its `.x3` goes through the real compiler and the agent's own compiled mandate. It is dry: no quote is
requested, and nothing is signed or sent. An agent without Armory Access gets a plain-text answer
instead. Revoking the agent, or its GPU-mesh connection, cuts off model access. The Proving Ground
runs the gate's own `decide` against the build: allowed model, other model, budget spent, revoked
agent, forged signature.

The profile page has a **Mind** panel, signed in the browser. To run headless, export the agent key
from its profile:

```bash
PYTHONPATH=x3-lang python3 -m forge.agent_cli --url http://127.0.0.1:7420 --agent vexa.x3-owner.near \
  --key vexa.x3-owner.near.agent-key.jwk.json --model qwen3:8b run "Pay x3-demo.near 10 USDC for invoice 77."
```

The CLI signs locally, and the key never leaves the machine. Exit code 0 means the run was answered
and allowed. Against an Access-protected Forge, set `FORGE_ACCESS_JWT`.

## Tests

```bash
PYTHONPATH=x3-lang python3 -m pytest x3-lang/tests/test_agent_forge.py -q   # host, compile, proving, births, connections, HTTP
PYTHONPATH=x3-lang python3 -m pytest x3-lang/tests/test_forge_mesh.py -q    # gate + real gpu_mesh coordinator/worker + CLI
cd x3-lang/forge/web && npx tsc -b && npx vitest run                        # creature genome determinism
npm run build && PYTHON=python3 npx playwright test                         # full browser flow incl. Mind on a real mesh (uses /usr/bin/google-chrome)
```

## Layout

- `registry/`: skills, connectors and archetypes (JSON, schema-checked at start; a bad entry stops the host)
- `core.py`: build validation, `.x3` generation, compile, manifest, stats, creature traits
- `proving.py`: Proving Ground
- `inference.py`: inference gate (signed payloads, `decide`, mesh client, budget)
- `agent_cli.py`: headless agent runner (signs with an exported agent key)
- `connections.py`: Comms Rig (allowlist, setup state, health, server-side secrets)
- `store.py`: High Command claim, births, roster, revocation, mesh, export/import
- `server.py`: stdlib HTTP host and JSON API
- `web/`: React + TypeScript + Vite + react-three-fiber app; `src/creature/genome.ts` is the pure creature generator
