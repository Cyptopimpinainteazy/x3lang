# X3-PIR-05 — Agent Law exact-tool enforcement pilot

This opt-in integration connects the real Rust capability compiler to the existing Python Agent Law Runtime. No production migration, host isolation, general contract/cross-chain execution, or external rollback guarantee is claimed.

## Provisioning contract
Build the offline example `pir_tool_grant` from the exact reviewed revision. It accepts independently selected scope and Unix-second time as two arguments and reads the capability source from stdin (bounded to 65,536 bytes). It returns hex canonical grant bytes and exact source SHA-256 as JSON. No signing or execution. The supported requirement set is empty intentionally: runtime proof of any nonempty requirement is not implemented. Only Tool + NoAsset is supported.

The trusted owner must independently review source, compiler identity, emitted fields, resource, and the actual host adapter's absence of asset effects before signing. The owner signs Ed25519 over ASCII X3/PIR/OWNER-GRANT/1 followed by NUL, 32-byte source digest, then the full canonical grant bytes. Runtime configuration accepts a list of these records (encoded, source_sha256, signature, all lowercase hex), with an independently chosen scope. Use a unique scope per deployment; the signature does not include a generated runtime ID, so reusing a scope and owner across fresh databases can reuse grants. A source digest alone does not establish source-to-grant correctness. No automatic signing or authorization from agent-supplied source.

## Enforcement boundary
Pass pir_scope and pir_grants to Runtime at creation. Both omitted preserve legacy behavior; an explicit empty list denies all PIR actions. Pinned SQLite configuration refuses changed or removed grants on restart, including downgrade. Every stamp issuance and execute reservation independently checks signed grant records, exact scope, pinned adapter policy metadata, native authenticated identity/ancestry, SHA-256 of actual requested artifact bytes, grant expiry, and action expiry. Every ancestor needs an exact matching resource grant, with decreasing delegation depth and nonincreasing expiration. Existing native signature, admission, budget, nonce, revocation, and configured stamp/jury checks remain in place. The reservation event records the leaf grant digest; this is local audit evidence, not an independently signed execution receipt.

Trusted host, database, clock, compiler, callback and operator must remain isolated. NoAsset is a restriction on the grant, not a proof of callback side effects. Adapter code identity is not verified. This pilot does not enable independent PIR-only revocation or configuration rotation: existing identity/ancestor and stamp revocation block future reservations, not in-flight execution. It does not migrate existing databases. A malicious host or direct access to underlying tools remains outside this boundary.

## Verification
From x3-lang:
```sh
cargo build -p x3-lang-compiler --example pir_tool_grant --locked --offline
X3_PIR_COMPILER="$PWD/target/debug/examples/pir_tool_grant" python3 -m unittest discover -s tests -p test_pir_agent_runtime.py -v
python3 -m unittest discover -s tests -p 'test_agent_runtime*.py' -q
python3 -m unittest discover -s tests -p 'test_agent_tool_gateway*.py' -q
```
The dedicated integration test must be run with the executable path; default Python discovery skips it when absent. Tests use ephemeral keys, SQLite and harmless callbacks; no network calls. Dedicated CI must be added before promotion. Initial constructor-only stub produced two expected failures (artifact/expiry and grant configuration), two existing safety tests passed; implementation passed those and two additional integration tests.

## Release hold
Stacked on #284. No production service changes, real credentials, funds, deployment or merge. Next: dedicated exact-head CI; real service isolation and direct-tool bypass tests; authenticated artifact/adapter identity, policy lifecycle and receipt proofs; EVM/settlement integration separately.