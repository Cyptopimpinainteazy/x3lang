# Legacy examples

These files are **not programs the current compiler accepts**, and they are kept rather
than deleted because each one documents a subject the project still cares about. Nothing
builds them: the conformance sweep globs `examples/*.x3`, so a file in this directory is
out of the gate by construction, and the gate over `examples/*.x3` (in
`crates/x3-tools/tests/cli.rs`) asserts that every file in the *parent* directory checks
and builds.

## What they are

All five are written in an older `contract <Name> { … fn … }` dialect — Solidity-shaped
top-level declarations, `const NAME: address = 0x…` constants, `fn` bodies — that the
parser no longer accepts. The first error is always:

```
x3c: parsing failed: Parser error: expected top-level item
```

| file | subject | why it is not rewritten |
|---|---|---|
| `arb.x3` | cross-DEX arbitrage across Uniswap V3 and SushiSwap | superseded: `examples/arb_scope.x3` is the current form of the same subject, in the `arb` + `venue` declarations PHASE 37 defines |
| `flash.x3` | flash-loan liquidation with collateral seizure | **cannot be**: flash capital is PHASE 20, whose own text forbids shipping before a formal safety proof, so there is no current form to write |
| `jit_lp.x3` | just-in-time concentrated liquidity provision | no phase covers JIT liquidity provision; a rewrite would be inventing a feature rather than translating one |
| `mev_smooth.x3` | MEV aggregation and validator distribution | the language has MEV notions as *costs* (`CostKind::MevLeakage`, the `mev_risk` category) and not as a distribution mechanism |
| `x3_coin_layer.x3` | a sketch of deposit/mint and burn/exit with BLS mirror proofs | its own title says `(Pseudo)`, and it is not a program in any dialect — it is pseudocode that was placed in a directory the tooling treats as programs |

## If you want to bring one back

Translate the *subject*, not the syntax. `examples/arb_scope.x3` is the worked example of
what that means for `arb.x3`: the old file's point was "find a profitable cycle across two
venues", and the current language expresses exactly that with `venue` declarations and an
`arb` scope, so the rewrite is shorter than the original and the compiler can decide more
about it. The other four need a language feature that does not exist yet before a
translation would mean anything, which is the reason each is listed above rather than
silently rewritten into something that parses and says less.
