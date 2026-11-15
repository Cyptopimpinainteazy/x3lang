# X3 Secure Worker — distinct-UID deployment contract (NOT DEPLOYED)

**Evidence: LOCAL Linux subprocess tests.** This is a configuration/runbook
for independent operators, NOT a claim of real two-account end-to-end proof,
GPU isolation, wallet safety, or hardened production sandbox.

## Trust separation

Two independent Linux identities must be configured by an authorized system
administrator, with distinct UIDs:

| Component | Illustrative identity | Permissions |
| --- | --- | --- |
| Trusted gateway/worker | `x3worker` (dedicated UID) | Owns runtime, stamp signing key, callbacks and SQLite database |
| Untrusted agent client | `x3agent` (different UID) | May access **only** the Unix IPC endpoint and its own work files |
| Dedicated socket group | `x3clients` | Allows pathname traversal and socket connection; **not** secret/data access |

These names are examples, **not** existing verified host accounts. Do not
create users/groups or change live service permissions as part of this PR.

The worker's Unix socket parent directory must be worker-owned, group
`x3clients`, mode **`2710`** (setgid, owner rwx, group x, others none).
The bound socket must inherit that group and is set to **`0660`**.
The listener additionally authenticates Linux kernel `SO_PEERCRED` UID
and refuses any caller not in the explicit allowlist. Worker UID MUST NOT
be listed as an agent client. Allowing a UID is an authorization choice:
a compromised process running as that UID can connect, but it must still
present the agent-signed action to the X3 gateway.

Runtime SQLite and signing secrets must be **outside the shared socket
directory**, under a worker-only parent directory mode **`0700`**.
The SQLite database must be a worker-owned regular file with mode **`0600`**.
The new worker preflight verifies those properties when `client_group` is
enabled. The test harness uses a private SQLite subdirectory and ephemeral
in-memory issuer keys. It does **not** test stored signing-key ACLs.

An authorized operator would provision an isolated service user, group,
runtime directories, key source, and supervisor. Such operations have not
been executed in this milestone. A production deployment must also
verify ACLs (e.g. `getfacl`), Linux capabilities, sudo privileges, supplementary
groups, `/proc` exposure, namespaces and process ptrace restrictions. File
mode checks by themselves cannot defeat root or a compromised service UID.

## IPC and availability hardening

The listener bounds each complete request with an **absolute monotonic**
deadline, independent of per-`recv` idle timeouts. A drip-feeding client can
no longer monopolize a sequential worker forever using one framed request.
A timeout or malformed frame gets only the constant `request denied`.

Clients may specify a bounded `execution_timeout` separately from the
short framing timeout (default **30 seconds**, maximum **600**). A timeout
while a valid adapter is running is an **unknown outcome**, not proof of
task cancellation: the runtime may already have reserved the nonce/budget.
Never blindly retry a timed-out signed action. Production needs queryable
idempotent result retrieval and a task supervisor. A callback that never
returns can still monopolize this sequential worker despite the framing
deadline; this module does not implement cancellation or per-task isolation.

Startup refuses unsafe/ambiguous filesystem paths and does not overwrite
existing sockets. Post-bind errors, including failure to capture the socket
inode, clean up only an owned socket in the prevalidated non-writable directory.

## Verification

```bash
PYTHONPATH=x3-lang python3 -m pytest -q x3-lang/tests/test_agent_secure_worker.py
PYTHONPATH=x3-lang python3 -m pytest -q x3-lang/tests/test_agent_*.py
```

Relevant tests cover:
- Protected task actually executed in a different process (PID).
- Forged signature, replay, rejected UID, truncated/oversize frame.
- Drip-feed request hard deadline and authorized task over 2 seconds.
- Distinct-UID mode socket permissions, group/setgid directory, rejection
  of worker UID, and refusal of exposed SQLite directory/file.
- Failure during socket startup does not strand the socket.

**What is not proven:** a real `x3agent` UID connecting to a real `x3worker`
UID, file access denial by the kernel across those UIDs, secret provenance,
GPU/driver or wallet key isolation, forced termination of hung adapters,
security of any public API, or production-grade audit and 10 Commandments.

## Required pre-production gate

An authorized test environment with two actual separate user accounts,
no shared credential file permissions, restricted agent filesystem/network
access, and negative attempts to read the worker secret/SQLite files and
directly invoke tool callbacks. Record UID/GID/mode evidence, exact commit,
test traces and supervisor restart/replay behavior; then obtain independent
security review. Until those tests pass, **do not assign real keys, GPU
device access or mainnet tool privileges to this worker**.

The earlier [gateway PR #262](https://github.com/Cyptopimpinainteazy/x3lang/pull/262)
and [worker PR #267](https://github.com/Cyptopimpinainteazy/x3lang/pull/267)
are held for the NEAR judging freeze. This branch does not deploy, merge,
fund, publish, or modify NEAR Guard.
