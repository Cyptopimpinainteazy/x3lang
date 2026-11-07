# X3Lang Wiki Conventions

Language: English.

Every page uses exactly one required category tag, zero to three area tags, and
at most one technical role tag.

## Category tags

| Tag | When to use |
|---|---|
| `architecture` | System boundaries and design decisions |
| `modules` | Per-component implementation contracts |
| `flows` | End-to-end execution and settlement paths |
| `integrations` | NEAR, Solana, BSC, 1Click, wallets, and external APIs |
| `security` | Trust boundaries, threats, authorization, and limitations |
| `release` | Testing, CI, deployment, evidence, and submission gates |

## Domain area tags

| Tag | Covers |
|---|---|
| `compiler` | Parsing, lowering, IR, bytecode, and manifests |
| `vm` | Deterministic execution and hostcalls |
| `mandates` | Owner policy and agent authority |
| `near` | NEAR contracts and light-client behavior |
| `solana` | Solana programs and SPL Token behavior |
| `bsc` | BSC contracts and bridge behavior |
| `evidence` | Proof collection, receipts, and claim boundaries |
| `browser` | Judge console and static demo behavior |

## Technical role tags

| Tag | When to use |
|---|---|
| `core` | Central component with high query frequency |
| `pattern` | Reusable design pattern |
| `contract` | Interface or agreement between layers |
| `setup` | Configuration or initialization instructions |

Do not invent tags. Add them here before using them.
