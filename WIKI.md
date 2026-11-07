# Wiki - X3Lang Ship

The project wiki is in `wiki/`. Read it before investigating architecture,
release rules, evidence boundaries, or known security constraints.

## How to use

- **Query:** use `/wiki-query` or `shipframe:wiki-query`.
- **Sync:** use `/wiki-sync` after meaningful architecture, security, or
  release changes.
- **Forge:** use `/wiki-forge` to ingest source documents into linked pages.

## Entry point

`wiki/index.md` is the master catalog. Current source-of-truth documents remain
`AGENTS.md`, the root `README.md`, code, tests, and exact-head CI.
