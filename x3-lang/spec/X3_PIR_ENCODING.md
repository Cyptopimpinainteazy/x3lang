# X3-PIR-03 — Canonical policy encodings (experimental)

Adds offline encoders to the existing Rust compiler library, using existing sha2 and AST types. No dependency, source grammar, bytecode, signature or runtime changes. Commitments are not authority, signatures, proof of execution, or proof of finality.

## Binding v1 compatibility
`encode_binding(profile, source, scope, native_id)` returns encoded bytes and SHA-256 digest. Wire format unchanged from X3_PIR_V1.md: ASCII domain X3/PIR/BINDING/1 + NUL; u8 profile byte length and ASCII profile; u8 scope length and ASCII scope; SHA-256 of exact source bytes; 32-byte native ID. Profiles: evm.action.v1, agent.admission.v1, near-bsc.window.v1. Source 1..65536 bytes, scope 1..128 ASCII bytes with initial alphanumeric and remaining alphanumeric or :._/-. Native IDs are opaque, not reinterpreted as typed grants.

## Typed grant v1
`encode_grant(&Grant)` returns bytes and SHA-256 digest. All integers big-endian. LP = u16 byte length and exact UTF-8 bytes, 1..256 bytes; existing grant validation further limits subject, parent, scope and tool adapter to 128 ASCII bytes. No normalization or truncation. Asset identity includes every existing AssetId field, even display symbol (conservative structural equality). Order and tags:

1. ASCII X3/PIR/GRANT/1 followed by NUL.
2. LP subject; parent presence u8 0 (none) or 1 followed by LP parent; LP scope.
3. Resource u8 tag: 1 EVM (u64 chain id, executor 20, target 20, selector 4, calldata hash 32); 2 tool (LP adapter, artifact hash 32); 3 NEAR–BSC (window ID 32, operation u8 lock=1, claim=2, refund=3).
4. Amount u8 tag: 0 NoAsset; 1 asset (LP VM family, LP chain, LP canonical ID, LP symbol, decimals u8, cap u128 in base units).
5. Exclusive expiry u64 Unix seconds, remaining delegation depth u8.
6. Requirement count u8 then ascending explicit one-byte IDs (exact code identity=1, atomic rollback=2, authenticated finality=3).

No optional defaults, extra fields or implicit ordering of guarantees. Encoder validates structural shape with epoch zero (expiry zero invalid; previously expired grants can be encoded for historical audit), not actual deadline, adapter support, identity, signatures, asset meaning or permission. New text bound is a wire-format constraint; no silent normalization. No decoder or general Python typed-grant implementation yet. A typed-grant digest is not an EVM action ID or settlement ID; never substitute it for a native protocol digest.

## Reproducible checks
From x3-lang:
```sh
cargo test -p x3-lang-compiler --test policy_encoding --locked --offline
python3 -m unittest discover -s tests -p 'test_x3_policy_encoding_vectors.py' -v
```
Rust tests pin three Python binding digests, full typed-grant bytes and SHA-256 against an independently assembled Python fixture, field mutations, disjoint resource/operation variants and malformed input refusal. Python reference fixture is intentionally not an alternative production encoder. Stub RED: five assertion failures. Implementation GREEN: five Rust tests; two Python reference tests. Prior native policy formats unchanged.

## Release hold
Stacked on PR #282 (and #281); review dependencies first. No parser-to-grant lowering, end-to-end runtime enforcement, decoder, signatures, shared budget, cross-chain finality, actual Solidity parity or live operations. Next: strict canonical compiler parsing/lowering, verify trusted context and adapter support before execution. No merge, deployment or production funds authorized.
