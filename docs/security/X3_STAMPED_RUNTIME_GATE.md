# Scoped X3 Stamp — local protected task execution (development prototype)

**Evidence level: LOCAL.** This builds on [Scoped Stamp v0](X3_SCOPED_STAMP_V0.md).
No public API, deployed NEAR Guard change, funded wallet, operating-system sandbox,
full 10 Commandments proof, or production authorization gateway is established.

## What actually changed

The existing `x3-lang/agent_runtime.py` `Runtime.execute` now accepts a
`stamp=` argument. **Only adapters registered by the trusted host in
`stamp_policies` require a stamp.** Other adapters retain their previous local
behavior. The stamp policy (pinned issuer public key, policy digest, required
check identifiers) becomes part of the persistent SQLite trust configuration,
and reopening the database with the policy missing or weakened is refused.

For a protected adapter, the execution method checks the signed agent
action, live identity/ancestor status, stamp signature, exact source digest,
agent ID, action, policy digest, time, required check IDs, revocation, and
the **stamp nonce equaling the action nonce**. These checks run inside
`BEGIN IMMEDIATE`. It then enforces normal capability, budget, and
major-action jury rules, and durably consumes the nonce **before dispatch**.

A trusted runtime operator can call `issue_stamp_for_action` with a local
agent-signed action and a **pinned issuer private key**. It independently checks
three local claims: signed agent identity, capability/surface scope, and source
digest binding. It also checks active status, budget availability, expiry,
revocation and used nonces. Issuance is not a budget reservation; the runtime
rechecks before execution. It cannot automatically attest other commandments,
jury decisions, immutable on-chain logging, or compiler soundness.

## Reproduce exactly

```bash
PYTHONPATH=x3-lang python3 -m pytest -q \
  x3-lang/tests/test_agent_runtime_stamp_gate.py \
  x3-lang/tests/test_agent_scoped_stamp.py \
  x3-lang/tests/test_agent_runtime.py \
  x3-lang/tests/test_agent_runtime_court.py \
  x3-lang/tests/test_agent_runtime_intents.py \
  x3-lang/tests/test_agent_constitution.py \
  x3-lang/tests/test_agent_admission_signed.py
```

The new gate tests cover correct issuance and execution, missing stamps,
forged signatures, wrong source/policy/identity, expiration, revocation across
restart, missing required checks, nonce misuse, and rejected policy downgrades.
Existing Agent Law tests check that older local adapters remain functional.

## Trust boundaries and remaining work

- The host that instantiates `Runtime` and controls issuer private keys is
  trusted; malicious host code or direct external access to adapter endpoints
  can bypass the Python runtime. The host must not expose raw adapter callbacks
  to untrusted agents.
- The standalone `issue_scoped_stamp` helper can sign caller-supplied claim
  labels. **Only the trusted host issuance path** verifies the three supported
  claims; treat manually signed evidence about other checks as unproven until
  separately independently verified.
- Revocation stops *future* reservations and cannot undo a task already
  dispatched. SQLite state can be rolled back by a compromised host; backups,
  externally anchored evidence and operating-system isolation remain separate.
- This proof covers a selected local adapter, **not universal enforcement across
  all agent tasks or deployed Guard**. A stamp is scoped authorization evidence,
  never a guarantee that the artifact is safe.
- Before production: key custody and rotation; independent executable
  commandment registry; ingress/API enforcement (no bypass); persistent
  revocation with operational controls; queue and crash recovery; authorization
  race testing; threat modeling; and independent security audit.

The existing NEAR Guard PR #202 must remain held for hackathon judging.
