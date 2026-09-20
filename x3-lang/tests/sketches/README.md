# Sketches

These files are **not programs**, and they are kept rather than deleted because each one
records a subject the project may still want. Nothing builds or runs them: the conformance
gates walk `examples/*.x3` and `tests/**/*.x3`, and both skip this directory by name, so a
file here is out of every gate by construction rather than by accident.

They sat in `tests/` until TICKET-014, where `x3c check` fails on them and nothing was
looking — the corpus gate covers `examples/` only, and the harness reads named fixtures
rather than globbing. A `.x3` file in a directory the tooling treats as programs is a claim
that it *is* one; this directory makes the claim honestly.

## What they are

Both are written in the same older dialect as `examples/legacy/` — top-level `fn` bodies,
`vec![…]` literals, a stdlib-shaped `atomic_swap(pair, …)` call — which the parser does not
accept. The first error is always:

```
x3c: parsing error
```

| file | subject | what it needs |
|---|---|---|
| `vector_ops.x3` | element-wise vector arithmetic, with a comment saying it "implies SIMD add" | the language has `Operation::VectorMath` and a `VECTOR_MATH` opcode but **no surface syntax** for a vector literal, so this is a sketch of a construct rather than a translation of one |
| `e2e_atomic_swap.x3` | an atomic swap called as a stdlib function | the current form is the `atomic swap` declaration (`examples/atomic_swap.x3`), which is a block with a route, guards and a refund path rather than a call |

## The three that passed the gate for the wrong reason (TICKET-109)

`arithmetic.x3` (was `tests/test_arithmetic.x3`), `simple_transfer.x3` and `bridge_step.x3`
(both were `tests/e2e/`) use the same older dialect — a `fn main() -> i64`, statement `let`
bindings, `return`, calls to a stdlib that does not exist here. They did **not** fail the
corpus gate the way the two above did, and that is the point: the compiler accepted every one
of those statements and lowered each to `Operation::Nop`, which is emitted as four zero bytes
and is therefore invisible to every reader. `arithmetic.x3` — whose entire subject is
arithmetic — reported `3 ops, no semantic errors`, and its artifact was byte-identical to an
empty program's.

So they were not programs the compiler could run; they were programs the compiler could
*ignore*, and the gate's claim ("a `.x3` file in a directory the tooling walks is a claim that
it is a program") was satisfied by the artifact of the dropping. `lowering` now refuses every
statement it cannot lower, naming the construct — `let`, `return`, `break`, `continue`, `for`
— which is what put these three here. `tests/test_arithmetic.x3b` went with them: a committed
one-byte file named like an artifact, `0x06`, which no reader accepts.

## If you want to bring one back

Write the current form of the *subject*, not this syntax. `examples/atomic_swap.x3` is the
worked example for the second one; the first needs a surface for vector values first, which
is a language decision rather than a rewrite.
