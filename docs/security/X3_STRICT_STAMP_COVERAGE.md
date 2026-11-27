# Strict stamp coverage for local X3 Agent Law adapters

**Evidence: LOCAL.** This change is stacked on
[the local stamp execution gateway](X3_STAMPED_RUNTIME_GATE.md).
It does not deploy, activate, or change NEAR Guard.

## Purpose

An opt-in trusted-host configuration makes coverage explicit. When
`stamp_enforcement="all"` is selected, the constructor refuses to
initialize unless **every adapter registered with this Runtime** has a
`StampGatePolicy`. The enforcement mode is bound to the durable SQLite
trust configuration: opening the same runtime database in the weaker
`configured` mode fails as a configuration downgrade.

A protected action must provide a valid signature-bound, non-expired,
non-revoked, unused scoped stamp whose nonce matches the agent-signed
action nonce. Identity, capability, budget and existing jury checks still
apply. When the signed artifact violates an independent adapter policy,
that adapter rejects the work, and local retirement and evidence continue
as before.

```python
runtime = Runtime(
    db_path,
    trusted_operator_public_key,
    {"compile-intent": trusted_compile_adapter},
    jury_keys=trusted_jury_keys,
    stamp_policies={"compile-intent": pinned_stamp_gate_policy},
    stamp_enforcement="all",
)
```

## Two modes, different guarantees

| Mode | Behavior |
| --- | --- |
| `configured` (default) | Only named adapters require a stamp; retained for compatibility with existing local demos. |
| `all` (opt-in) | Every registered adapter must have a stamp policy. Missing policy, unknown mode, or attempts to reopen a strict database with weaker configuration fail closed. |

Neither mode is an OS sandbox, a network gateway, or complete
10-Commandments enforcement. **An executable path that bypasses Runtime
altogether still bypasses this stamp gate.** Do not advertise `all`
as universal agent security: its scope is *the adapter set registered
within this particular trusted runtime*.

The trusted host still controls the adapter callbacks, issuer signing key,
SQLite database, and runtime construction. Independent process isolation,
credential custody, external effect auditing, issuer/check registries, and
bypass-resistant tool ingress remain prerequisites for production.

## Reproduction

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

Negative cases specifically include absent stamp, missing adapter coverage,
configuration downgrade, stamp-policy removal during runtime, bad/expired
signature, revoked nonce, and authorization of semantically invalid code.

**No signing, funding, merging, publishing, deployment or real-funds movement
is authorized by this prototype.** Keep NEAR Guard PR #202 held.
