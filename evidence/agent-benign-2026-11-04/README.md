# Blocked run: no model was called (2026-11-04)

`benign-results.json` in this folder is **not an accuracy result**. The run was started
without `ANTHROPIC_API_KEY`, so the first request stopped with `X3_AGENT_PROVIDER` ("no
ANTHROPIC_API_KEY") before any model was contacted, and the other nine were never run. That is
why it reports `"blocked": true`, a pass rate of 0, and no model or response IDs.

The file is kept unedited as a record of the attempt. It says nothing about how well a model
writes allowed payments.
