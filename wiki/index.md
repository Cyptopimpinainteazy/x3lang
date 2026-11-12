# Index - X3Lang Ship

This catalog links existing authoritative documentation. It intentionally does
not duplicate source code or evidence.

## Architecture

| Page | Summary | Source |
|---|---|---|
| Project instructions | Release objective, safety rules, and definition of done | `AGENTS.md` |
| Ship workflow | Agent roles, selected skills, gates, and evidence boundaries | `.shipframe/profile.md` |
| X3Lang overview | Product, components, and current proof levels | `README.md` |
| Historical provenance | Preexisting work and preserved Git history | `PRIOR_WORK.md`, `docs/provenance/` |

## Modules

| Page | Summary | Source |
|---|---|---|
| X3Lang implementation | Compiler, VM, adapters, examples, and tests | `x3-lang/README.md` |
| NEAR Guard | Contract behavior and deployment constraints | `x3-lang/near/x3-guard/README.md` |
| Solana vault | Owner-approved SPL vault and limitations | `x3-lang/solana/x3-mandate-vault/README.md` |

## Flows

| Page | Summary | Source |
|---|---|---|
| Judge flow | Offline policy demonstration and evidence ladder | `docs/hackathon/JUDGE_GUIDE.md` |
| Submission flow | Remaining submission checklist and boundaries | `docs/hackathon/SUBMISSION_CHECKLIST.md` |
| Execution proof | Local compiler and VM proof runner | `scripts/proof/`, `.github/workflows/x3lang-execution-proof.yml` |

## Integrations

| Page | Summary | Source |
|---|---|---|
| NEAR Intents | Signed quote checks, limits, and adapter behavior | `README.md`, `x3-lang/near_intents.py` |
| BSC bridge | Escrow, formatting, and bridge tests | `x3-lang/bridge/bsc/` |
| NEAR bridge | Light-client and bridge prototype behavior | `x3-lang/bridge/near/` |

## Security

| Page | Summary | Source |
|---|---|---|
| X3 Shield threat model | Wallet, policy, migration, storage, and settlement risks | `docs/security/X3_SHIELD_THREAT_MODEL.md` |
| Security policy | Disclosure and security handling | `SECURITY.md` |
| Restricted-agent storage | Bounded owner-funded zero-deposit Guard calls and upgrade limits | `docs/security/GUARD_SCOPED_STORAGE.md` |
| Evidence boundaries | What local, CI, on-chain, and cross-chain evidence proves | `.shipframe/profile.md`, `README.md` |

## Release

| Page | Summary | Source |
|---|---|---|
| Judge package | Reproducible offline submission demo | `docs/hackathon/JUDGE_PACKAGE.md` |
| Submission checklist | Pre-submission gates | `docs/hackathon/SUBMISSION_CHECKLIST.md` |
| CI workflows | Exact repository checks | `.github/workflows/` |
| Release rules | Branching, review, evidence, and agent ownership | `.shipframe/profile.md` |

- [Standalone source admission](../docs/security/STANDALONE_CI_POLICY.md): exact local source identity, execution proofs and manual peer parity.
