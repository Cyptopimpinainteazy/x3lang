# X3-PIR v1: native policy binding layer (experimental)

This is a first interoperability primitive, NOT a semantic policy IR, compiler integration, authorization engine, or proof of settlement. It preserves original protocol identifiers, does not modify native formats, and introduces no new source language.

## Interface

`bind(profile, source, scope, native_id) -> Record(encoded, digest)` takes exact policy/artifact bytes (1–65,536), an independently selected ASCII scope (1–128 bytes), and the exact 32-byte native identity. Scope is an opaque deployment/context label, not authenticated chain identity. Profile is a closed enum. Unknown profiles refuse. Digest is lowercase SHA-256 hex of encoded.

Wire layout: ASCII domain `X3/PIR/BINDING/1` followed by NUL, u8 length of profile and ASCII profile, u8 length of scope and ASCII scope, 32-byte SHA-256 of source, 32-byte native ID. Lengths are byte counts. No trailing bytes. Verification rebuilds and compares the entire record from independently trusted inputs; never trust caller-selected inputs or a self-recomputed digest. A source hash does not prove that the source semantics match the native artifact.

| Profile | Native ID | Boundary |
|---|---|---|
| evm.action.v1 | 32-byte action ID from PR #279 executor/compiler | Opaque external identifier; no compiler linkage or EVM validation yet |
| agent.admission.v1 | SHA-256 of existing approval_payload(child,parent,nonce,expires_at) | Reuses agent_admission_signed.py; no signature verification, budget reservation, revocation or admission |
| near-bsc.window.v1 | Existing bridge.settlement.window_id(**terms) | No finality, time-window validity, funding or settlement authorization |

Do not treat a binding as a proof, signature, receipt of execution or grant of authority. Existing checks remain mandatory. A caller must independently select canonical source bytes, scope, native inputs and profile; there is no durable registry, trusted policy selection, or execution integration here.

## Verification

From x3-lang:
```sh
python3 -m unittest discover -s tests -p 'test_x3_policy_ir.py' -v
python3 -m unittest discover -s tests -p 'test_agent_constitution.py' -q
python3 -m unittest discover -s tests -p 'test_agent_admission_signed.py' -q
python3 -m unittest discover -s bridge -p 'test_settlement.py' -q
```

Requires Python with cryptography for the existing Agent Law adapter. Initial red: four assertion failures with empty stub. Green on isolated x3star1 checkout: 4 new + 35 existing focused tests. Default brew Python lacked cryptography; system Python had it. No production services, chain transactions, permissions or funds modified.

## Next gates

1. Add independent Rust encoding vector and typed capability/policy semantics within the canonical compiler (not another DSL).
2. Verify source-to-native lowering equivalence, typed asset identity, units, delegation, expiry and explicit atomicity/finality requirements; refuse unsupported constraints.
3. Integrate exact EVM action encoder after PR #279 has passed its own security gates; test compiler-generated and on-chain IDs with matching context.
4. Connect actual Agent Law admission/runtime checks and settlement validators, preserving every native security invariant.
5. Add exact-head CI and independent review before production promotion.
