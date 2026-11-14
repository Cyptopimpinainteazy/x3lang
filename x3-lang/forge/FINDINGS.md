# X3 Agent Forge — Step 0 findings

Read-only survey of `x3lang` at `3b433f1` before any Forge code was written. Everything the
Forge enforces comes from the modules named here. The Forge adds a UI, a registry of skills and
connectors, and a small HTTP host. It adds no new enforcement logic.

## What compiles, and to what

| Forge stage | Real code it calls | What it produces |
|---|---|---|
| Mandate compile | `near_mandate.parse_mandate` → `lower_mandate` (via `near_intents.mandates()`) | x3-guard `create_mandate` arguments: `{owner, agent, terms}`. `terms.program_sha256` binds the mandate source. |
| Agent intent compile (Proving Ground) | `near_intents.compile_program` → `build_quote_request(dry=True)` | A lowered NEAR Intents leg and its 1Click quote request. No network call. |
| Mandate check (Proving Ground) | `near_intents.check_against_mandate` and `near_mandate.check_mandate` | The x3-guard contract's own check list (`mandate_active`, `mandate_asset`, `mandate_recipient`, `mandate_refund_to`, `mandate_per_payment`, `mandate_daily`, `mandate_slippage`, `mandate_fees`, `mandate_deadline`), in the contract's order. |
| Delegation check | `agent_constitution.check_admission` | Fail-closed child-admission decision (capabilities ⊆ parent, surfaces ⊆ parent, budget, expiry, spawn limit, lineage, approval). |
| Birth | `agent_runtime.Runtime.genesis` | Durable root identity in the Agent Law runtime's SQLite registry, with a hash-chained `born` event. Needs the operator's Ed25519 signature over `genesis_payload`. |
| Revoke | `agent_runtime.Runtime.revoke`, plus `near_intents.py mandate revoke` args | Runtime status `revoked` with a `revoked` event; the `revoke_mandate` call the owner sends to x3-guard. |
| On-chain mandate | `near_intents.py mandate args` (same lowering) | The `create_mandate` call. The Forge prints it. The owner signs and sends it. The Forge never signs or sends a NEAR transaction. |

Token IDs come from a 1Click `/v0/tokens` list. The Forge defaults to the committed snapshot
`tests/fixtures/near_intents_tokens.json` (6 tokens: Solana USDC/SOL, NEAR USDC/wNEAR, Ethereum
USDC, BSC BNB) and can load a live list with `--tokens` or `--live-tokens`. The UI always shows
which list it compiled against.

## The mandate grammar (what a skill can compile to)

`owner`, `agent`, `spend <Chain>.<Asset>`, `per_payment N`, `per_24h N`,
`allow recipients a, b`, `allow assets <Chain>.<Asset>, …`, `refund_to R`,
`require slippage <= x%`, `require fees <= y bps`, `approve above N by K of a, b`, `expires D`.
Amounts are base units. The parser is strict: an unknown line is an error.

## Enforcement hooks

- **x3-guard (NEAR contract, `near/x3-guard/src/mandate.rs`)**: enforces mandate terms on
  Guard-mediated agent actions. It holds no funds. A wallet key held outside Guard bypasses it
  (AGENTS.md, proof-card limits).
- **Agent Law runtime (`agent_runtime.py`)**: local, durable. Signed genesis/admission,
  budget reservation, revocation, hash-chained event log. Not on chain, not an OS sandbox.
- **Admission check (`agent_constitution.py`)**: pure preflight. A pass is not a certificate.

## Connection targets found

Referenced in this repo: `guard.x3star.net`, `judge.x3star.net`, `controlroom.x3star.net`,
`x3lang.x3star.net`. From the owner: `dotx3.net`, `ide.dotx3.net` and the dotx3.net subdomains
`command, general, teamsix, radar, phantom, armory, recon, provingground, force, dex, fund, docs,
status`. External: `1click.chaindefuser.com` (1Click API), `nearblocks.io` (explorer).

None of these were reachable from the build sandbox (outbound allowlist), so liveness is not
claimed here. The Forge checks each one at runtime from its own host and shows the real result.
No target page implements a Forge return callback yet. The Forge implements the callback and
its `state` check, and falls back to an owner confirmation that is labelled as such.

## Gaps (no .x3 or runtime construct exists — shown in the UI, not faked)

1. **Cooldown between payments**: no mandate term. Left out.
2. **Non-payment capabilities** (repo read, propose patch, run CI, deploy, agent messaging,
   monitoring, evidence verification): recorded as Agent Law runtime capabilities and checked by
   admission for delegation, but no production adapter enforces them yet. The UI labels each
   skill with what actually enforces it.
3. **Connections**: recorded as runtime surfaces (checked on delegation). No contract binds a
   connection. A connection never grants authority.
4. **Runtime ↔ Guard link**: a runtime birth does not create the Guard mandate or a NEAR key
   (documented in `docs/security/X3_AGENT_CONSTITUTION_V1.md`). The Forge does both halves and
   shows them separately.
5. **Bytecode**: the mandate compiles to Guard terms, not X3BC. The creature seed is the hash of
   the normalized build, which the compiled terms bind through `program_sha256` (the hash of the
   generated mandate source). Compiled terms embed a compile-time expiry, so hashing them would
   change the creature on every recompile of the same build.
