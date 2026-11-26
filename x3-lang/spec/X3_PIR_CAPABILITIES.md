# X3-PIR-02 — typed capability model (experimental)

Extends the existing Rust AST crate; reuses the existing Trading Core AssetId, with conservative full structural equality (including symbol). No new DSL, dependency, bytecode opcode, or wire-format change. This is a structural model, not compiler lowering or authorization. See X3_PIR_V1.md for the separate offline provenance binding format; no equivalence between these formats is established yet.

## Types and rules
- Grant: subject, optional immediate parent, opaque scope, exact resource, amount limit, exclusive Unix-second expiry, remaining delegation depth, explicit set of requirements.
- Resource: exact EVM chain ID (u64), executor, target, selector and calldata hash; exact tool adapter plus artifact hash; exact NEAR–BSC window plus lock/claim/refund operation. No wildcards. Scope must identify the independently selected execution context, not a caller assertion.
- AmountLimit: NoAsset (no asset authority, never unlimited), or Asset with existing AssetId and u128 per-action cap in base units. Parent and child must have the same amount mode and exact asset; child cap cannot increase. This is not aggregate or lifetime budget accounting and does not prove which assets a contract actually affects. Asset-to-resource mapping must be authenticated by the adapter.
- Delegation: same scope/resource, different subject, exact immediate parent link, live deadlines, child deadline no later than parent, strictly decreasing depth, child retains every parent requirement. Structural checks do not authenticate identity, detect global cycles, reserve sibling budgets, prove signatures or enforce revocation.
- Requirements: exact code identity, atomic rollback, authenticated finality. Closed enum and no implication between requirements. check_support rejects any requirement not in the independently trusted adapter's support set. Support is NOT proof that an operation actually satisfies the requirement. No automatic cross-chain atomicity.
- Validation: bounded ASCII identity/scope/adapter names, nonzero EVM chain and addresses with distinct executor/target, nonempty asset fields, nonexpired grant. No claimed network, code hash, asset semantics or actual chain state is authenticated by these functions.

## Verification
From x3-lang:
```sh
cargo test -p x3-lang-ast --test policy_ir --locked --offline
```
Six tests cover bounded delegation, exact resources, asset substitution and caps, expiry/depth/identity/requirement changes, malformed inputs and unsupported requirements. Permissive stub: five expected assertion failures, one success; implementation: six passed. Existing Python binding/Agent Law/settlement tests are independent regression gates.

## Release hold
No runtime integration, source grammar, canonical serialization, adapter mapping, signature verification, shared-budget accounting, finality proof, live transactions or production permission changes. Existing native Agent Law and settlement checks remain mandatory. Next: canonical encoding and compiler lowering with strict unsupported-policy rejection, then trusted adapter enforcement with end-to-end tests. PR stacked on the binding draft so it can be reviewed independently; retarget/rebase after dependency review. No merge/deploy authorization.
