# X3-PIR-04: bounded capability source compilation

This is an offline policy artifact entry point inside the existing Rust compiler and canonical lexer/parser. It is NOT a new general executable syntax, command-line feature, runtime authority or integration with the existing program AST/bytecode. Existing executable parse/check/build entry points deliberately refuse these declarations, including mixed source, until enforcement exists.

Example (fictional, expired fixture):

```x3
capability {
    subject "child";
    parent "parent";
    scope "sandbox:fixture";
    tool "reader" "0404040404040404040404040404040404040404040404040404040404040404";
    no_asset;
    expires 1000;
    depth 1;
    requirements [exact_code_identity, atomic_rollback];
}
```

Fixed order, exactly one declaration and EOF. Parent is a quoted identifier or literal none. Tool is an exact adapter and 64 lowercase hex digits (without 0x) of artifact SHA-256 as a caller-declared identifier; hashing the actual artifact is a runtime requirement. no_asset is mandatory, not unlimited. Expiry is exclusive Unix seconds (u64); depth is u8; both use the canonical lexer unsuffixed integer literal. Requirements are explicit, may be empty, and can only use the three known names (exact_code_identity, atomic_rollback, authenticated_finality). Duplicates, unknown fields, EVM or settlement syntax, asset authority, mixed source and trailing tokens refuse. Existing identifier validation and encoder bounds apply. Comments and whitespace follow canonical lexer behavior; source digest still commits exact bytes.

`compile_capability_source(source, trusted_scope, supported, now)` returns the typed Grant, canonical grant Commitment, and exact source SHA-256. Trusted scope must match, trusted supported requirements must cover every requested requirement, deadline must be live. Inputs are supplied by the trusted caller, not evidence of an actual adapter, trusted clock or authorization. The parent is not authenticated, delegation is not checked against a parent grant here, and NoAsset does not prove absence of side effects. Runtime integration must authenticate identity, artifact, delegation, revocation and resource, and ensure each requirement is actually satisfied.

Test from x3-lang:
```sh
cargo test -p x3-lang-compiler --test capability_source --locked --offline
```
Five tests cover positive lowering, scope/requirements/expiry refusal, malformed/unhandled source, source vs semantic commitment distinction and legacy executable-path refusal. Stub red: two expected positive-case assertion/unwrap failures, three refusal tests passed. Implementation green: five passed. Next: connect this artifact to real Agent Law host enforcement with independent signature/parent checks and deny bypasses; then support typed EVM and settlement resources explicitly. No deployment or permission changes.
