# NEAR authenticated light-client state

`x3-lang/bridge/near_authenticated.py` composes the merged NEAR Merkle,
approval, validator-commitment and execution-outcome primitives into a
fail-closed authenticated head store.

The verifier is initialized only by an offline-approved checkpoint. A public
RPC response cannot create authenticated state, even when multiple providers
agree.

## Trust chain

```text
externally pinned trust policy
        +
offline threshold signature over checkpoint checksum
        |
        v
authenticated checkpoint header and current validators
        |
        v
committed next-epoch validator set
        |
        v
real Ed25519 approvals from the correct epoch
        |
        v
authenticated successor head
        |
        v
receipt outcome and block inclusion against that head's Merkle root
```

No API accepts `trusted=true`, `verified=true`, or a caller-provided
authenticated root. The verifier's `authenticated=True` result is produced only
after those cryptographic checks succeed.

## Offline checkpoint ceremony

The trust policy lists the accepted NEAR network, genesis hash, protocol
versions, checkpoint signer public keys and signature threshold. Its canonical
SHA-256 digest must be recorded outside the repository and supplied to
`initialize`.

Checkpoint creation and approval are intentionally separate:

```bash
# Build an unsigned candidate from one untrusted light-client response.
python3 x3-lang/bridge/near_authenticated.py build-checkpoint \
  --candidate checkpoint-response.json \
  --current-validators independently-reviewed-validators.json \
  --network testnet \
  --genesis-hash FWJ9kR6KFWoyMoNjpLXXGHeuiy7tEY6GmoFeCA5yuc6b \
  --protocol-version 88 \
  --source "offline NEAR testnet review" \
  --verified-by "reviewer identity" \
  --observed-at "2026-11-06T00:00:00Z" \
  --out checkpoint.candidate.json

# Compute the canonical integrity checksum.
python3 x3-lang/bridge/near_authenticated.py seal-checkpoint \
  --input checkpoint.candidate.json --out checkpoint.json

# Print the exact domain-separated bytes/checksum for an external HSM or
# reviewer-controlled key. This program never imports a private key.
python3 x3-lang/bridge/near_authenticated.py checkpoint-message \
  --checkpoint checkpoint.json

# Pin the reviewed policy hash.
python3 x3-lang/bridge/near_authenticated.py trust-policy-digest \
  --trust-policy trust-policy.json

# The approval file contains threshold signatures from the policy's signers.
python3 x3-lang/bridge/near_authenticated.py initialize \
  --checkpoint checkpoint.json \
  --approval checkpoint.approval.json \
  --trust-policy trust-policy.json \
  --trust-policy-sha256 <externally-recorded-policy-sha256> \
  --checkpoint-sha256 <externally-reviewed-checkpoint-sha256> \
  --state ~/.x3/near-authenticated-head.json
```

`build-checkpoint` does not approve anything. `seal-checkpoint` adds only an
integrity checksum. Initialization fails unless the signed policy and
checkpoint checksums match the separately supplied values.

### Trust-policy shape

```json
{
  "schema": "x3.near-light-client-trust-policy.v1",
  "network": "testnet",
  "genesis_hash": "FWJ9kR6KFWoyMoNjpLXXGHeuiy7tEY6GmoFeCA5yuc6b",
  "allowed_protocol_versions": [88],
  "checkpoint_signers": [
    {
      "signer_id": "offline-review-1",
      "public_key": "ed25519:<base58 public key>"
    }
  ],
  "threshold": 1
}
```

### Checkpoint approval shape

```json
{
  "schema": "x3.near-light-client-checkpoint-approval.v1",
  "checkpoint_sha256": "<64 lowercase hex>",
  "approved_at": "2026-11-06T00:00:00Z",
  "signatures": [
    {
      "signer_id": "offline-review-1",
      "signature": "ed25519:<NEAR base58 signature>"
    }
  ]
}
```

The signed message is:

```text
"X3_NEAR_LIGHT_CLIENT_CHECKPOINT_V1\0" || raw_32_byte_checkpoint_checksum
```

## Advancement

```bash
python3 x3-lang/bridge/near_authenticated.py advance \
  --state ~/.x3/near-authenticated-head.json \
  --candidate next-light-client-response.json

python3 x3-lang/bridge/near_authenticated.py head \
  --state ~/.x3/near-authenticated-head.json
```

For each candidate the verifier:

1. Reconstructs the candidate block hash from the lite header.
2. Requires a strictly greater height.
3. Accepts only the authenticated current epoch or its committed next epoch.
4. Selects the validator set for the candidate's epoch; a newly announced set
   is never used to authenticate itself.
5. Verifies `next_bps` using the V1 Borsh commitment supported by protocol 88.
6. Reconstructs `ApprovalInner::Endorsement(next_block_hash)` and the signed
   height.
7. Verifies every present Ed25519 signature in validator order.
8. Counts absent approvals as zero, counts the full validator stake, and
   requires signed stake strictly greater than two-thirds.
9. Rejects duplicate identities, malformed keys, unsupported versions,
   overflow, replay, conflicting epochs, and incomplete persistence.
10. Atomically replaces state only after every check succeeds.

State writes use a lock, same-directory temporary file, `fsync`, atomic rename
and directory `fsync`. A failed comparison or interrupted write leaves the
previously authenticated state unchanged.

## Genuine testnet evidence

Two real responses from PR #57 are retained:

| Evidence | SHA-256 | Result |
|---|---|---|
| p27 light-client update | `466f916b65037b2f3d51e0958d3cb8ae1e989c4e18f8d64e07b9961939f1ffb6` | Announces Gb validator set |
| Gb light-client update | `c47f0c381dbd9a37ac432aa4f77cf9d47ea8ccc452bfe4ff2a67b868953caed7` | Real approvals and next commitment |

FastNear and NEAR Foundation returned byte-identical copies of the p27
response. Provider agreement remains evidence metadata only; it is not the
checkpoint trust root.

The p27 response's 20-validator `next_bps` commitment is:

```text
CL9uW6xEEJGhSstXRHZMACfNac2eVpK64A4Kyu3Cc85D
```

Using exactly those announced Gb validator keys and stakes, the Gb response has:

```text
approved stake  588424816164625374350075972122329
total stake     592425651545485319272531395151441
approval ratio  99.3246687798844%
```

The Gb response also commits its next validator set:

```text
4nzeJKGgHtgtESYKxfNh7Qq38WuYxCZ4zbVhjQHUYvez
```

The test suite verifies this real signature transition after initialization
with a **synthetic test checkpoint signature**. The real validator approvals are
not synthetic; the checkpoint trust in that test is explicitly test-only.

## Authenticated inclusion

```bash
python3 x3-lang/bridge/near_authenticated.py verify-inclusion \
  --state ~/.x3/near-authenticated-head.json \
  --proof receipt-light-client-proof.json \
  --receipt-id 3f9FCzqC8k75AGob2UrCxbKVqADRyv5M5vGfoLEXbZyu \
  --receiver-id league-v7.near-post.testnet
```

This runs `near_outcome.verify_receipt_outcome` against the authenticated
head's block-Merkle root. A successful result proves successful receipt
inclusion under that head. It does **not** prove X3 escrow storage, deployed
code identity, `Claimed`, `Refunded`, `NotFunded`, settlement causality or
permission to sign.

## Verified properties

- Ed25519 approvals are checked against the validator set for the candidate
  epoch.
- Validator and checkpoint keys are decoded canonically and must be non-identity
  points in the Ed25519 prime-order subgroup. Small-order and torsion-component
  keys are rejected, including keys belonging to validators that did not sign.
- The strict `> 2/3` total-stake threshold is enforced.
- Current and next validator identities cannot be substituted without detection.
- V1 validator Borsh commitments are recomputed.
- Checkpoint network, genesis, protocol, checksum, provenance and offline
  signatures are checked before state exists.
- Head rollback and replay are rejected.
- State corruption and interrupted updates fail closed.
- Receipt outcome and block paths are checked against an authenticated head
  root, not an RPC-selected root.

## Unverified properties

- No independently trusted NEAR testnet checkpoint approval has been provided.
  Therefore the committed real transition is a cryptographic test under a
  synthetic checkpoint authority, not a completed live trust-anchor ceremony.
- State authority is only as strong as the offline checkpoint signer policy.
  Compromise or operator error in that ceremony compromises every later head.
- The selected protocol-88 V1 encoding is supported. V2/future validator
  encodings are rejected until protocol-specific vectors and review are added.
- Raw light-client responses do not cryptographically carry a protocol version.
  Optional candidate protocol metadata is checked when present, but protocol
  transitions require an authenticated external release policy.
- The current code does not authenticate NEAR account storage or deployed
  contract code. A receipt remains insufficient for terminal escrow state.
- The local state checksum detects partial/corrupt writes; it is not a MAC
  against an attacker who already controls the state-owner filesystem account.
- No independent security audit, distributed signer anti-equivocation,
  slashing, deployment or funded settlement exists.

**No wallet keys are loaded, no settlement certificate is signed, no contract is
deployed, and no funds move.**

## Verification

```bash
cd x3-lang/bridge
python3 -W error::ResourceWarning -m unittest test_near_authenticated -v
python3 -W error::ResourceWarning -m unittest discover -s . -p 'test_*.py' -v

cd lightclient-core
cargo test --locked
cargo fmt -- --check
cargo clippy --all-targets -- -D warnings
```
