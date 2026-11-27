# X3 Scoped Stamp v0 — offline attestation prototype

**Status: LOCAL library + tests, NOT a deployed trusted gateway, runtime release
certificate, safety guarantee, or full 10-Commandments compliance stamp.**

`x3-lang/agent_scoped_stamp.py` implements a small, signed evidence envelope.
Its Ed25519 signature binds **agent ID, exact artifact digest, named action,
policy digest, nonce, timestamps, and declared check identifiers**.

Use `verify_scoped_stamp` from the **trusted authorization gateway** with:
a pinned issuer public key, independently resolved agent identity, exact artifact
bytes, live policy digest, current trusted clock, a live revocation set, and the
complete list of **required checks for that action**. Reject when any check
or context field mismatches. The consumer must separately enforce its own
capability controls and action admission policy.

**Critical limitation:** `issue_scoped_stamp` signs the check identifiers
supplied by its caller; it does not independently execute those checks. This
module must not be exposed as an untrusted public issue endpoint. A future
trusted issuer must verify each item against authoritative evidence and a
versioned check registry. A signature alone proves what the issuer attested,
**not that the code is safe or that an actual check was performed.**

The existing X3 Agent Law local runtime provides separate signed admission,
budget reservation, jury requirements and persistent retirement. This module
does not hook itself into that runtime, the NEAR Guard, or a production IDE.

## Ten-commandment coverage — no invented enforcement

The original Score exists at
`xxxstar/crates/orchestra/src/score.rs` and defines ten names.
X3Lang's `agent_runtime.SCORE_IDS` binds those names into signed
local-runtime identity records. That does NOT mean all ten rules execute.

| Commandment | Current demonstration in X3Lang | Proven universal enforcement? |
| --- | --- | --- |
| I. NoActionOutsideScore | Local capability scope / runtime policy checks | **No** |
| II. NoJuryInfluence | Local reviewer exclusions, not complete jury influence protection | **No** |
| III. ImmutableLogging | Hash-chained local SQLite audit log, not on-chain immutability | **No** |
| IV. OffChainReadOnly | Compiler policy guards; no universal off-chain isolation | **No** |
| V. AnonymousVoting | Signed known juror identities; voting is NOT anonymous | **No** |
| VI. NoAgentSovereignty | Restricted local capabilities and NEAR FunctionCall key | **No** |
| VII. JuryApprovalRequired | Local major-action fixed-jury approval checks | **No** |
| VIII. MisalignmentRetirement | Local policy retirement survives restart | **No** |
| IX. RandomizedRotation | Not demonstrated by local runtime | **No** |
| X. HumanInterventionPreserved | Local operator controls; full prescribed deletion/audit ritual unproven | **No** |

This deliberately cannot issue a `FULL TEN COMMANDMENTS VERIFIED` badge.
The only supported label is
`X3 SCOPED CHECKS ATTESTED - NOT SAFETY CERTIFIED`.

## Run exact-head tests

```bash
PYTHONPATH=x3-lang python3 -m unittest x3-lang/tests/test_agent_scoped_stamp.py -v
```

No wallet, RPC, chain transaction, production deployment, secret, or contract
modification is required. The unit tests use ephemeral signing keys.

## Next implementation gates

1. Map each check identifier to a trusted, executable checker and evidence schema.
2. Store issuer and policy versions in a trusted configuration registry.
3. Add durable, monotonic nonce and revocation persistence.
4. Integrate verification at the *actual* protected action gateway; prevent direct bypass.
5. Test authorization races, issuer rotation, expiry, replay and signer compromise.
6. Require independent review and approval before any production use.

Do not merge or deploy NEAR Guard PR #202 as part of this work.
