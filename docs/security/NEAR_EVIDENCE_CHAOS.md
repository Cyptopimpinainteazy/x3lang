# X3Lang NEAR evidence: RPC disagreement and interrupted-write red team

This test-only security work evaluates the **read-only NEAR evidence collector** (`x3-lang/bridge/near_evidence.py`) and its local content-addressed storage. It never deploys a contract, signs or sends transactions, touches user wallets, or restarts a production validator.

## Attacks and required invariants

| Attack or fault | Required behavior |
| --- | --- |
| Three purported RPC hosts, one equivocates on next epoch while the others agree | Store `disagreement`, explicitly false `provider_consensus`, never claim authenticated finality or signing authorization. |
| One provider responds to `status`/starting `block` then times out during `next_light_client_block` | Store `incomplete`; a single valid respondent cannot set `provider_consensus=true`. Store partial responses unchanged. |
| Receipt or transaction proof is available from only one of two configured RPCs | Store `incomplete`, `candidate=null`, `authenticated=false`, `authorized_to_sign=false`. |
| Collector interrupted after linking the durable manifest, before appending its index | Verification refuses the unindexed manifest. Re-running the same request in a new collector instance repairs the index exactly once, preserving original retrieval metadata and content hashes. |
| Actual isolated collector subprocess terminated with `os._exit(73)` at the same boundary | Parent sees an orphan, restarts collection in a separate process context, gets one valid index entry. No live node is killed. |
| Four collector subprocesses concurrently preserve the same evidence | Use an exclusive local file lock and durable fsync: one manifest, exactly one index record, offline verification succeeds. |
| Truncated JSONL index | Collection and offline verification fail closed; never silently replace or extend damaged index. |
| Orphaned manifest subject field modified while the original `manifest_id` is left intact | Refuse with `manifest subject digest mismatch` before reindexing. |

The existing RPC transport takes **injected deterministic local responses in these tests**; three distinct hostnames do not prove independent real operators or consensus. The isolated subprocess kill is real OS process termination, but it is only **collector-process recovery**, **not NEAR validator-restart recovery**. A production validator restart/DB recovery and a network-level quorum test require a separately scheduled, isolated validator with genuine chain-state persistence.

## Implementation and replay

`EvidenceStore.commit_manifest` now reconciles index entries even when an apparently duplicate manifest path exists. It validates the existing manifest's *subject digest* before indexing; a modified security-relevant subject cannot be accepted merely from a matching filename or self-reported ID. This does not independently authenticate excluded metadata such as the original collection timestamp after an orphaned write. `fcntl.flock` serializes index reconciliation among local processes. The index check validates a complete JSONL record and its exact manifest checksum. A damaged index **is not automatically repaired**; an operator must restore from independent evidence. The tests cover **process crashes, not host power loss or storage-hardware durability**, and do not assert fsync of all parent directories.

The collector still labels agreement as `complete_candidate`, **never authenticated finality**. An interrupted/partial proof remains incomplete. No consumer may turn `provider_consensus`, a transaction status, or a `candidate` into permission to sign or settle.

Run from X3Lang repository root on Linux with the pinned Python dependencies:

```bash
PYTHONPATH=x3-lang/bridge python3 -W error::ResourceWarning \
    -m unittest test_near_evidence -v

# Broader bridge suite (includes unrelated database tests):
PYTHONPATH=x3-lang/bridge python3 -m unittest discover \
    -s x3-lang/bridge -p 'test_*.py' -q
```

The focused suite checks real code paths but injects synthetic RPC data at the **external RPC boundary**; it is not a replacement for a public testnet probe or an authenticated NEAR light-client verifier. On the current local implementation, the crash-index, quorum-outage, and corruption regressions were each observed **red before fixing the production code**.

## Release gates and constraints

- CI workflow `x3lang-near-evidence-chaos.yml` runs the focused suite on Ubuntu 24.04 / Python 3.12 with existing hash-pinned dependency freeze and exact proposed head.
- Fail a security-sensitive release on missing, skipped, red, or unexpectedly unreviewed checks; do not bypass private-peer mirror or deployed-WASM proof gates.
- Existing manifests and artifacts stay immutable and network-separated. This is a storage-consistency repair, not an authenticated backfill, chain finality oracle, or wallet signer.
- The evidence index lock uses `fcntl` and is therefore currently **Linux/Unix-only**. It is designed for local cooperating collector processes on the same filesystem; no distributed storage or network-filesystem consensus is claimed.
- If an index ends with a partial record, the verifier refuses it; the fix will not silently guess missing bytes after a crash.
