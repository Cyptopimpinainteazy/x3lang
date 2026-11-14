# X3 Secure Worker — Linux Unix-socket subprocess prototype

**Classification: LOCAL / DEVELOPMENT ONLY.** Builds on the preserved
`feat/agent-tool-ingress-gateway-20261114` branch (PR #262, held for the
NEAR judging window). This is **not** a deployed GPU worker, wallet signer,
internet-facing API, OS sandbox, or complete X3 agent isolation solution.

## What the tests actually exercise

`x3-lang/agent_secure_worker.py` is a minimal Unix-domain socket server
running in its **own process**. A *trusted process* initializes a strictly
stamped `Runtime`, owns its SQLite file and trusted Ed25519 issuer key,
registers a single allowlisted adapter, and invokes `serve_unix_worker`.

- Socket path must be absolute; parent directory must exist, be owned by
  the worker effective UID, and have no group/other permissions.
- Existing socket paths are **never overwritten**, and a socket is cleaned
  up only if its inode still matches the one this server created.
- The socket is mode `0600`; Linux `SO_PEERCRED` obtains authenticated
  kernel-supplied peer UID for an explicit allowlist.
- Requests and responses use four-byte network-order length framing, bounded
  by `MAX_WIRE_BYTES` and `MAX_REPLY_BYTES`.
- One request is handled per connection, sequentially, with socket timeouts.
- Errors return the constant `request denied`, not private keys, exception
  strings or internal runtime state.
- The existing `LocalToolGateway` authenticates the agent signature, checks
  exact allowlisted adapter names, enforces scoped stamps, and lets the
  **trusted Runtime** verify authorization, budgets, jury approval and nonce
  consumption before invoking its host-owned callback.

## Local reproducibility

```bash
PYTHONPATH=x3-lang python3 -m pytest -q x3-lang/tests/test_agent_secure_worker.py
PYTHONPATH=x3-lang python3 -m pytest -q x3-lang/tests/test_agent_*.py
```

The subprocess tests use `multiprocessing.get_context("fork")` and an
ephemeral SQLite database; they compare worker and test-process PIDs and
exercise valid execution, bad signatures, reuse, wrong peer UID, oversized
frames, truncated bodies, invalid socket directory permissions and cleanup.
No live GPU call or wallet transaction occurs.

## Mandatory production hardening BEFORE real credentials

**Separate process is not the same as isolation.** The tests intentionally
launch the trusted worker under the same UID as the test process so they can
run without privileged setup. An untrusted process running as the same UID
may inspect other process state or access files depending on host security
configuration. Neither `SO_PEERCRED` nor a `0600` socket protects a service
from a malicious process with equivalent OS privileges.

Production deployment must introduce an independently audited service
account or comparable sandbox with an explicit ACL/IPC design, least-privilege
filesystem access, seccomp/AppArmor/SELinux or container isolation as
appropriate, separate secret storage, no unrestricted network/GPU device
access in the agent process, resource limits, logging, service supervision,
graceful shutdown under crashes, and authenticated rollback-resistant
audit storage. A production socket cannot simply use this demo's `0700`
directory and `0600` mode when a distinct client UID needs to connect;
the access model must be designed and tested for that separation.

The current code does **not** enforce that all agents route through this
socket. The trusted host must forbid direct access to adapters, model
tool handlers, GPU runtime APIs, wallets, and signer material. Those
capabilities are not safe to enable based only on these tests.

The existing PR #262 preflight audit discrepancy is unresolved:
capability/budget preflight denials do not always trigger the same persisted
retirement event as an execution-time denial. Authentication failures must
remain fail-closed; designing a reliable denial log requires careful
distinction between unauthenticated and authenticated requests.

**No Guard contract changes, PR #202 changes, keys, funds, production
deployment, merge, or public publishing were performed by this milestone.**
