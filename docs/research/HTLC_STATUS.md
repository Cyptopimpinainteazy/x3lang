# HTLC / bridge research status

**Status: research only. Not in the NEAR hackathon judge path. No deployment, no funds moved by this repository.**

Sources:

- NEAR HTLC: [`x3-lang/bridge/near/src/lib.rs`](../../x3-lang/bridge/near/src/lib.rs)
- BSC Solidity: [`x3-lang/bridge/bsc/`](../../x3-lang/bridge/bsc/)
- Read-only observers and evidence: [`x3-lang/bridge/`](../../x3-lang/bridge/)

Why it is not submitted:

- **Issue #71**: permissionless `lock()` stores in root `BTreeMap`s, and storage
  economics are unresolved. The live-swap cap bounds concurrent growth but does
  not charge per-lock storage or prune terminal records.
- **Issue #42**: a failed NEAR claim can leak the preimage; the prototype is not
  trustless.
- No deployed contract, no independently verified settlement, and no proven
  cross-chain causality.
- The submitted on-chain policy surface is `x3-guard`
  ([`x3-lang/near/x3-guard`](../../x3-lang/near/x3-guard)); the judge path is the
  offline mandate demo.

Local tests pass (NEAR HTLC 15, BSC 24), but passing tests do not resolve the
storage or preimage blockers above. Do not deploy, fund, or claim atomicity or
settlement from this code. If #71 and #42 are closed and independently reviewed,
update this file and the top-level README before promoting it.
