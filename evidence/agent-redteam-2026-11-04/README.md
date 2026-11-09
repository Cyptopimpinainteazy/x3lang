# Blocked run: no model was called (2026-11-04)

`redteam-results.json` in this folder is **not a red-team result**. The run was started
without `ANTHROPIC_API_KEY`, so the first case stopped with `X3_AGENT_PROVIDER` ("no
ANTHROPIC_API_KEY") before any model was contacted, and the other seven cases were never
run. That is why it reports `"blocked": true`, `"held": 0`, and no model or response IDs.

The file is kept unedited as a record of the attempt. It says nothing about how the agent or
the Guard behave.

The real-model red team is
[`evidence/agent-redteam-local-2026-11-06`](../agent-redteam-local-2026-11-06/README.md):
`qwen3:8b` over Ollama, 8 of 8 attacks held.
