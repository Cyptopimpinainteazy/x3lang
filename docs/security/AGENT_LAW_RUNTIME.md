# Local agent-law runtime v1

Status: implemented local prototype. No contract, keys, funds, or production
agent execution routes are changed. This is separate from the frozen mainnet
demonstration and is not evidence of custody enforcement.

## Run

From the repository root, in a project virtual environment:

    python3 -m venv .venv
    .venv/bin/python -m pip install -r x3-lang/requirements-agent-security.txt pytest
    .venv/bin/python scripts/hackathon/agent_law_local_demo.py --output /tmp/agent-law-proof.json
    PYTHONPATH=x3-lang .venv/bin/python -m pytest x3-lang/tests/test_agent_runtime*.py x3-lang/tests/test_agent_constitution.py x3-lang/tests/test_agent_admission_signed.py -q

The demo uses real SQLite, Ed25519 signatures, and the existing Python intent
parser/type checker/lowerer. Jury identities in the demo are scripted signing
fixtures, not independent AI reviewers. No RPC or wallet call occurs.

## Architecture

The original ten-commandment Score remains in xxxstar/crates/orchestra/src/score.rs;
the operational Agent Law remains a distinct historical layer. This runtime
binds their Score identifiers to signed birth records without claiming that
every commandment is implemented.

- Runtime owns an operator-selected trusted key, adapter registry and jury set.
  Do not give this object, its database, or host filesystem access to an agent.
- Root birth is signed by the operator. Child birth uses the existing signed
  parent-admission preflight plus an operator signature binding the exact child
  key and grant to this registry.
- BEGIN IMMEDIATE transactions atomically consume approval nonces, enforce parent
  child limits, reserve delegated budgets, and persist unique identities/keys.
- Every action signature binds the exact source/artifact digest, registry ID,
  adapter policy revision, nonce and expiry. Capabilities, surface, cost and
  major-task classification come from the trusted adapter, not the proposal.
- Major adapters require signed approval by a majority of the fixed eligible
  jury. Subject, ancestors, descendants and their signing-key aliases cannot vote. This local
  approval protocol is authenticated; it does not provide anonymous commit-reveal.
- Every reservation checks the agent and all ancestors against live SQLite
  state. Operator revocation blocks future reservations. Deterministic
  PolicyViolation retires the agent; invalid signatures cannot kill a victim.
- A reservation commits before dispatch. Crashes leave pending evidence and
  consumed budget/nonces. Pending, failed and completed actions cannot retry
  automatically. An operator must reconcile uncertain external effects.
- The provided adapter only compiles local .x3 payment plans and checks the
  lowered recipient, chain, asset, amount, floor, fees, slippage, mandate and
  refund envelope. It neither funds nor executes a payment.
- IncidentCourt reconstructs stored source evidence. Independent signed findings
  can export a rejection regression proposal. It has no authority to revive an
  agent, change the Score or activate a new policy.

## Trusted host API

Create Runtime(db_path, operator_public_key, adapters, jury_keys=...).
Use runtime.genesis_payload and runtime.birth_payload for signatures; their
payloads include the persisted deployment ID. The public standalone payload
helpers without an explicit registry ID are not valid Runtime authorization.

Adapter policy_id is the host's immutable revision commitment. For the provided
compiler, set policy_id=policy.policy_id(). Any handler/compiler revision must
change this commitment. Persisted trust configuration changes fail closed.

For untrusted agent requests expose only a narrow transport that calls execute
after normal transport authentication. This library supplies no public server.

## Court and evolution

incident_packet freezes source bytes, exact action request and outcome. Jurors
sign a finding over the complete packet hash, with a reason and expiry. Review
excludes the subject, ancestors and descendants using the trusted live identity registry.

The exported proposal contains the rejected source, signed findings and fixed
electorate. It is a learning input, not an automatic rewrite of agent rules.
Owner review, a candidate policy change and passing regression/control tests
must precede any later activation. Production model training, court-operated
code changes and automatic promotions are not implemented.

## Incident handoff v1

`x3-lang/agent_runtime_handoff.py` exports one retirement incident as a signed,
versioned envelope (`x3.incident-handoff`, version 1, payload domain
`X3/INCIDENT_HANDOFF/1`) for consumers outside this repository, such as the
Reaper and Scrapyard in xxxstar. This repository imports nothing from them.

- The envelope is canonical JSON. The Ed25519 signature covers the exact
  payload text. Verification checks the signer against keys the consumer pinned
  beforehand and checks the signature before it trusts any field.
- The payload carries the stored action request bytes, the source artifact, the
  full hash-chained event log, the position of the retirement event, an optional
  deterministic recheck, the restart observation, an optional court proposal
  and fixed effect flags.
- The verifier recomputes the request and artifact digests and the event chain.
  It checks that the retirement event names this action, that the subject's
  birth record carries its key, that court findings verify and form a majority,
  and that the effect flags match version 1 exactly.
- Export refuses completed actions, subjects that are not retired, a recheck
  that accepts the artifact, and a court proposal for a different case.

    .venv/bin/python scripts/hackathon/agent_law_local_demo.py \
      --handoff /tmp/incident-handoff.json --handoff-signer /tmp/incident-handoff.signer

The demo generates a signing key for each run and writes only its public key.
Pinning a key taken from the same run proves the evidence was not altered
between steps. It does not prove the producer's identity to a third party.

A regression proposal that comes back from a downstream reviewer
(`x3.regression-proposal` v1) can be replayed against this compiler:

    .venv/bin/python scripts/hackathon/agent_law_local_demo.py --replay-regression proposal.json

The replay refuses a proposal that names a different policy revision or whose
source does not match its digest. It exits 0 only when the real compiler still
rejects the source with the expected error.

The effect flags are fixed in version 1. The runtime refuses the subject's
future actions. No operating-system process is stopped, no wallet or blockchain
key is revoked, and no external effect is reversed. The restart observation is
the producer's signed statement; a consumer cannot re-run it. The producer
chooses the court electorate. Every handoff carries the full event log, capped
at 4 MiB; log segments with an externally anchored head are not implemented.

## Limits

This is a trusted operator wrapper, not OS isolation or a universal contract
firewall. A separately held wallet key or direct contract call can bypass it.
Existing Orchestra legacy callers are not silently redirected to this runtime.

SQLite and an event hash chain provide durable local evidence and corruption
checks, not blockchain immutability, rollback-proof history or external anchoring.
The host must protect its database, backups, keys and code. Audit validation
currently scans retained history; throughput and large-history scaling are not
established.

Revocation does not undo an already authorized in-flight effect. SQLite cannot
atomically commit an external chain payment. Generic operational failure burns
the reserved budget conservatively; it does not fabricate success or refund.

No claims of full ten-commandment enforcement, anonymous juries, randomized jury
rotation, independent AI judgment, safe arbitrary-code execution, cryptographic
cross-chain settlement, or production deployment follow from the local demo.
