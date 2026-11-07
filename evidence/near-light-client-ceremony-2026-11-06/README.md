# NEAR light client — real offline checkpoint ceremony (2026-11-06)

The light client was previously initialized in tests with a **synthetic** checkpoint
signer. This directory records the real ceremony: a freshly generated (non-deterministic)
Ed25519 signer key, the real NEAR testnet genesis hash and protocol version, the real
20-validator set for the checkpoint epoch, and the real p27 → Gb light-client transition.

## What was verified

- Checkpoint built from the real testnet light-client block at height `276392452`
  (epoch `p27Pbpijm…`, which announces the Gb validator set).
- Current validator set = the 20 real validators returned by the testnet RPC for that
  block, with their real stakes.
- Checkpoint sealed and approved with a real Ed25519 signature from a freshly generated
  signer key (public key `ed25519:7kfGRoRmj9BK3RrDC88TWexyFEeHdtf3grwB7fExXisW`).
- Advanced to the Gb head (`4gugmCPdbFQPhfo6YS9PVQaQAfkxpgeaAMwCNPa1akj2`, height
  `276420402`) and verified the real validator approvals:

```text
approved stake  588424816164625374350075972122329
total stake     592425651545485319272531395151441
approval ratio  99.3247%
```

This is the objective's "verify a real block's validator approvals" step, done against
real chain data rather than a synthetic test fixture.

## Files

- `trust-policy.json` — the pinned policy (signer key, genesis, protocol 88, threshold).
- `checkpoint.json` — the sealed checkpoint (height, validators, commitments, provenance).
- `checkpoint-approval.json` — the Ed25519 approval signature over the checkpoint checksum.
- `authenticated-head.json` — the initialized-and-advanced authenticated state.
- `ceremony-report.json` — the condensed verification report.

## Boundary

This proves light-client header authentication and real validator approvals. It does
**not** prove wallet execution, durable contract storage, settlement authorization, or
cross-chain causality. The signer key is freshly generated for this ceremony; key custody
of the offline review key remains an operational control, not a cryptographic gap.
