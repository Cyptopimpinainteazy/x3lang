# X3 Local Tool Ingress — bounded proof, not a deployed sandbox

**State: local-only Python facade.** This work extends the merged signed-runtime
boundary. It is neither an HTTP service nor an operating-system isolation layer,
GPU worker integration, wallet protection, or proof of universal 10 Commandments
enforcement.

## Intended boundary

A **trusted host process** constructs an Agent Law `Runtime` in
`stamp_enforcement="all"` mode. It registers exact adapters, scopes and
`StampGatePolicy` objects, holds the scoped stamp issuer key and database,
and exposes only `LocalToolGateway.submit(bytes)` to a caller.

The request envelope contains only an agent ID, adapter ID, request nonce,
expiry, base64-encoded exact artifact, and the agent's Ed25519 signature over
`Runtime.action_payload(action)`. No callback, URL, key, dynamic policy,
caller-issued stamp or arbitrary tool name is accepted.

The gateway validates input bounds and exact schema, obtains a stamp through
trusted local preflight, and invokes the existing signed-action
`Runtime.execute` method. That runtime rechecks the stamp, capability,
live status, budget, jury requirements, source binding and nonce inside its
trusted SQLite transaction before dispatching the registered handler.

## Reproduce locally

```bash
PYTHONPATH=x3-lang python3 -m pytest -q \
  x3-lang/tests/test_agent_tool_gateway.py \
  x3-lang/tests/test_agent_runtime_stamp_gate.py \
  x3-lang/tests/test_agent_scoped_stamp.py \
  x3-lang/tests/test_agent_runtime.py
```

The negative tests reject forged signatures, callback injection, arbitrary
tool names, caller-provided stamps, unknown adapters, replay, revoked agents,
policy violations, malformed input and ambiguous duplicate JSON fields.
The test handler is a **local in-memory callback**, not GPU execution or any
real wallet/network operation.

## Critical limitations

**A Python facade cannot prevent direct access to its own runtime objects or
registered callbacks by untrusted code executing in the same process.** To
make this the actual mandatory entry point, keep the trusted runtime and
adapter callbacks in a separately isolated process with file-system, network
and credential restrictions. Never hand untrusted agents the callback
references, the issuer key, the SQLite file, a raw GPU driver/session, or
privileged process execution.

**External integrations are NOT secured here.** Before granting GPU, wallet,
IDE, or API privileges, build and review:

1. A separate authenticated IPC ingress (e.g., properly permissioned Unix
   socket), with request size limits, peer identity checks and denial tests.
2. An operator-owned worker process without agent-controlled imports, direct
   callback references, or unrestricted filesystem and network access.
3. Credentials stored only within appropriately isolated worker processes;
   third-party API keys and wallet keys are never delivered to agent code.
4. Allowlisted tool operations with bounded side effects, resource quotas,
   cancellation, audit trails, and safe crash recovery.
5. Independent end-to-end adversarial tests demonstrating that direct
   callback, socket, GPU, wallet and raw-credential bypass paths fail.
6. A reviewed production rollout and emergency revocation plan.

No deployment, merger, wallet transaction, on-chain policy upgrade, or Guard
change is part of this PR. Keep NEAR Guard PR #202 out of this scope.
