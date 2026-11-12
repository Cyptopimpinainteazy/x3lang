# X3Lang Implementation Status

Status date: 2026-11-03

## Current architecture authority

| Surface | Canonical path |
|---|---|
| Language compiler | `x3-lang/compiler` |
| Language VM | `x3-lang/vm` |
| Shared bytecode/opcode/receipt core | `crates/x3bc-core` |
| Language specification and opcode declarations | `x3-lang/spec` |
| Compiler/tooling CLI | `x3-lang/crates/x3-tools` (`x3c`) |
| NEAR quote adapter | `x3-lang/near_intents.py` |
| NEAR policy contract | `x3-lang/near/x3-guard` |
| Historical provenance | `PRIOR_WORK.md`, `docs/provenance` |

Root `crates/x3-*` integration surfaces are not independent language
authorities. Any future synchronization must preserve these boundaries.

## Confirmed strengths

- The `.x3` parser, typed AST, semantic/IR verification, lowering, X3BC
  emission, VM interpreter, Guard, and signed-quote validation have substantial
  local test coverage.
- `cargo test -p x3-lang-vm` passes 358 tests on a clean `main` checkout.
- The Guard contract passes 42 tests, including real signed quote fixtures.
- `wee_alloc` is absent from both the wasm32 dependency graph and lockfile.
- GitGuardian passes on current PR heads.
- Historical source provenance and the October 18 baseline are preserved.

## Confirmed blockers and risks

| Priority | Blocker | Evidence | Required action |
|---|---|---|---|
| P0 | The legacy X3 Guard serialized-state layout lacks an upgrade migration | PR #9; mandate audit | Land and independently verify the versioned migration |
| P0 | Guard holds no origin-chain funds; direct wallet-key spending bypasses policy | Guard README and mandate audit | Wallet or smart-account enforcement must be proven |
| P0 | There is no current seven-validator, finalized execution evidence | Repository/infrastructure audit | Run the seven-validator and endurance suite |
| P0 | No live NEAR authorized settlement and refusal evidence is public | Judge guide marks it pending | Owner-authorized deployment, payment and refusal run |
| P0 | Trustless (atomic or cryptographic) cross-VM settlement remains unproven | 2026-11-07: an `.x3`-specified 1 test USDC transfer from Base Sepolia (EVM) to NEAR testnet finalized on both chains, with the program hash bound on NEAR first ([evidence](hackathon/FIRST_FINALIZED_SWAP_EVIDENCE.md)); the two chains were linked by a trusted Tidepool relay | Trust-minimized bridge proof; X3VM bytecode execution |
| P1 | Real-model Anthropic evidence and 10-request benign accuracy are missing | `ANTHROPIC_API_KEY` unavailable | Run and commit provider/model/response/session evidence |
| P1 | Register/stack-depth verifier coverage is incomplete | `verifier.rs` currently validates structure and branch boundaries | Add typed register/stack verification |
| P1 | Repository mirror gate is an active PR | PR #16 | Validate after merge from both repositories |
| P2 | Studio/LSP/debugger are not canonical implementations | Tooling inventory | Design and implement in a later phase |

## Verification evidence

```text
PYTHONPATH=x3-lang python3 -m pytest x3-lang/tests -q
node --test x3-lang/near/demo/test/*.test.mjs
cd x3-lang && cargo test -p x3-lang-vm --locked
cd x3-lang && cargo fmt --all -- --check
cd x3-lang && cargo clippy -p x3-lang-vm --all-targets --locked -- -D warnings
cd x3-lang/near/x3-guard && cargo test --locked
cd x3-lang/near/x3-guard && cargo check --target wasm32-unknown-unknown --locked
```

Evidence from an older commit is not evidence for a later release commit.

## Next critical task

Finish the P0 NEAR Guard state-migration review and wallet-enforcement boundary,
then produce the authorized settlement/refusal evidence bundle. No mainnet or
seven-validator readiness claim is valid before those are complete.
