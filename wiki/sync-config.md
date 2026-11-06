# Wiki Sync - Repository Configuration

Read automatically before `/wiki-sync`.

## Repositories

| Name | Relative path | Stack |
|---|---|---|
| x3lang | `.` | Python 3.12, Rust 1.90, NEAR SDK 5.30, Solana 2.3, Foundry/solc 0.8.24, Node.js 22 tests |

## Patterns with wiki impact

- `x3-lang/compiler/**`
- `x3-lang/crates/**`
- `x3-lang/vm/**`
- `x3-lang/near/**`
- `x3-lang/solana/**`
- `x3-lang/bridge/**`
- `crates/x3bc-core/**`
- `scripts/proof/**`
- `.github/workflows/**`
- `docs/security/**`
- `docs/hackathon/**`
- `AGENTS.md`
- `.shipframe/profile.md`

## Patterns without wiki impact

- `**/target/**`
- `**/__pycache__/**`
- `**/.pytest_cache/**`
- Generated evidence JSON with a documented regeneration command
- Lockfiles unless the dependency change affects an API or release claim
- Formatting-only and lockfile-only churn

