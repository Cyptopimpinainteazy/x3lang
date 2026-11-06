# NEAR proof-evidence harness

`x3-lang/bridge/near_evidence.py` is a read-only collector for NEAR testnet
light-client evidence. It preserves raw RPC responses in a content-addressed
store, records exact requests and provider identities, detects disagreement,
and verifies the stored bytes offline.

This harness is **not a light client**. A saved RPC response is untrusted input
until an independent verifier authenticates checkpoint ancestry, validator
signatures and stake, epoch transitions, Borsh commitments, outcome
serialization, Merkle branches, and durable account storage. No command signs,
deploys, transfers funds, or authorizes a BSC settlement.

The verifier integration and remaining authenticated-light-client acceptance
criteria are tracked in [issue #56](https://github.com/Cyptopimpinainteazy/x3lang/issues/56).

Upstream references:

- FastNear's current `light_client_proof` RPC schema:
  <https://docs.fastnear.com/rpc/protocol/light-client-proof>
- NEAR light-client protocol notes:
  <https://nomicon.io/ChainSpec/LightClient.html>
- Nearcore's current source documentation, which warns that the Nomicon page is
  out of date and that the archived reference implementation must not be used
  as the sole production specification:
  <https://github.com/near/nearcore/blob/master/docs/ChainSpec/LightClient.md>

## Reproduce the two evidence classes

### Candidate checkpoint

```bash
python3 x3-lang/bridge/near_evidence.py collect-checkpoint \
  --rpc https://test.rpc.fastnear.com \
  --rpc https://rpc.testnet.near.org \
  --network testnet \
  --head-hash EXLQRQpbx44GJjDRgA27u4D2DRZkCUddt9WYZGNgkUp9 \
  --out evidence/near-proof
```

The collector requests, from every provider:

1. `status`, to record `chain_id`, genesis hash and protocol version.
2. `block`, to pin the requested starting head.
3. `next_light_client_block`, to preserve the candidate light-client view,
   validator set, declared `next_bp_hash`, and approvals.
4. `block` at the candidate height, to record the block hash and compare the
   returned inner-lite fields with the light-client view.

The result is labelled `candidate_checkpoint_untrusted`,
`authenticated=false`, and `authorized_to_sign=false`. A caller cannot promote
it through a boolean option. Promotion requires an independent verifier.

The committed collection demonstrates an expected operational limitation:
FastNear and NEAR Foundation finalized heads were at different heights during
collection. Both candidates are preserved and the manifest reports
`status=disagreement`; no common checkpoint or consensus claim is invented.
Raw validator sets remain available to the verifier, but the collector does
not compute or validate the Borsh `next_bps` commitment.

### Receipt proof

```bash
python3 x3-lang/bridge/near_evidence.py collect-receipt \
  --rpc https://test.rpc.fastnear.com \
  --rpc https://rpc.testnet.near.org \
  --network testnet \
  --receipt-id 3f9FCzqC8k75AGob2UrCxbKVqADRyv5M5vGfoLEXbZyu \
  --receiver-id league-v7.near-post.testnet \
  --transaction-hash 6RAnrVJY5P6XK8qhzBfYtWvpLn1ofxWyi6goXA9N3MgY \
  --sender-id ai-48.near-post.testnet \
  --head-hash V7KcHcg16BVWWxhVbw2vQj87D3dJQwXchnU7DCVzo6V \
  --out evidence/near-proof
```

The two providers returned identical successful execution proofs for the
receipt:

- Network: NEAR testnet
- Genesis: `FWJ9kR6KFWoyMoNjpLXXGHeuiy7tEY6GmoFeCA5yuc6b`
- Execution block: `DSu44uX9GmwyGXzUhAVK1DWJYGRaUBbfT2tNRGQjEd5i`
- Proof head height: `276418680`
- Proof head epoch: `GbZqwYHuVsdzj91APrve7tykRtgZbrp8MrfEJYq9DFLU`
- Outcome branch: 1 node
- Outcome-root branch: 2 nodes
- Block branch: 20 nodes

The committed response is real chain-derived evidence. It remains
unauthenticated because the harness does not run the independent verifier.

The same CLI also provides `collect-transaction` for a top-level transaction
light-client proof. The committed transaction proof also matches across both
providers and records the same transaction’s `SuccessReceiptId`.

Committed manifests:

| Evidence | Manifest ID | Status |
|---|---|---|
| Checkpoint candidates | `9f20750427bf33d12750ad713707e472f86d46c45016c2580f70ab8f43386745` | `disagreement` |
| Receipt proof | `26ca2546a31f8b9ba267aa3fadc154cfd2ea6f64ee81c0190f848f1b02148493` | `complete_candidate` |
| Transaction proof | `9fd3bc8241df327644f0a2ffadb9649506be0c5dd1fcf0f31c8fed92abf20236` | `complete_candidate` |

## Persistent evidence layout

The evidence root is separated first by network:

```text
evidence/near-proof/testnet/
  index.jsonl
  manifests/<evidence-digest>.json
  sha256/<first-two>/<sha256>.json
  mutations/manifest.json
  mutations/<case>.json
```

Every raw JSON-RPC response is stored byte-for-byte under its SHA-256 digest.
An identical response is stored once. A conflicting object at the same
content address is rejected. Manifests contain:

- schema and evidence kind;
- network, retrieval time and exact request parameters;
- provider origin and status identity;
- per-call method, parameters, request ID and artifact checksum;
- agreement or disagreement status;
- unaudhenticated candidate interpretation;
- explicit trust limitations.

The manifest ID is a SHA-256 digest of its evidence subject. A repeated
collection of the same evidence returns the existing manifest instead of
appending a duplicate index record.

## Offline verification

```bash
python3 x3-lang/bridge/near_evidence.py verify \
  --root evidence/near-proof --require-negative-corpus
```

This checks:

- append-only index and manifest correspondence;
- manifest subject digests;
- network separation;
- every artifact’s exact path, SHA-256, byte length, and JSON syntax;
- duplicate index entries and unindexed manifests;
- all 12 negative fixtures and their labels/checksums.

It does not check blockchain consensus, validator signatures, stake, epochs,
Merkle inclusion, or storage state. Those are verifier responsibilities.

## Escrow terminal states

The native NEAR escrow is `x3-lang/bridge/near/src/lib.rs`.

| State | What execution evidence can show | What authenticated durable state still requires |
|---|---|---|
| `Claimed` | The recipient successfully executed `claim(id, preimage)` and the receipt emitted the expected success | Finalized storage proof of the exact swap, recipient, amount, hashlock, and `SwapState::Claimed`; deployed code identity |
| `Refunded` | The initiator successfully executed `refund(id)` after expiry | Finalized storage proof of the exact swap, initiator, amount, expiry, and `SwapState::Refunded`; deployed code identity |
| `NotFunded` | Anyone successfully executed `close_unfunded(id)` after the immutable deadline | Finalized storage proof of the registered window, exact terms, deadline, absence of `swaps[id]`, and `FundingWindowState::NotFunded` |

A successful receipt proves that one contract call executed successfully. It
does not alone prove the resulting durable storage state, the deployed code
hash, or the exact swap terms. Ordinary `query` view-call output is similarly
not an authenticated storage proof.

No suitable deployed X3 escrow was found for this evidence pass. The real
receipt above therefore validates generic proof acquisition only. `Claimed`,
`Refunded`, and `NotFunded` settlement verification remain outstanding.

## Negative corpus

```bash
python3 x3-lang/bridge/near_evidence.py generate-negative-corpus \
  --root evidence/near-proof \
  --network testnet
```

The corpus derives labelled mutations from the genuine receipt proof and
candidate light-client response:

- incorrect block hash;
- wrong receipt ID;
- wrong receiving contract;
- modified outcome status;
- modified Merkle branch;
- incorrect Merkle direction;
- wrong light-client head;
- wrong network;
- incorrect validator information;
- truncated or malformed proof;
- cross-swap evidence substitution;
- evidence from the wrong epoch.

Each file declares `label=synthetic_negative_fixture`,
`authenticated=false`, its source artifact checksum, and the exact mutation.
These fixtures do not claim that the production light client rejected them.
That assertion is allowed only after the independent verifier is exercised.

## Security review

| Boundary | Control |
|---|---|
| RPC transport | HTTPS-only origin, no credentials/query/path, bounded response, JSON content type, exact request-ID match |
| JSON parsing | Duplicate keys, NaN/Infinity, malformed JSON and oversized responses rejected |
| Provider diversity | At least two distinct hostnames; disagreement preserved rather than averaged |
| Network substitution | RPC `chain_id` must equal the requested network; storage is network-separated |
| Proof substitution | Receipt/transaction ID, executor, successful status, branch shape and requested head are checked structurally |
| Checkpoint promotion | No boolean override; output is permanently classified as a candidate |
| Integrity | Content-addressed raw bytes, SHA-256 manifests/index, duplicate detection, offline verification |
| Financial authority | No signing key, deployment, transfer or settlement API exists |

Distinct hostnames are not proof of independent operators or consensus. A
provider can return a validly shaped but false response. Only the separately
developed authenticated verifier can establish finality and inclusion.

## Tests

```bash
cd x3-lang/bridge
python3 -W error::ResourceWarning -m unittest test_near_evidence -v
python3 -W error::ResourceWarning -m unittest discover -s . -p 'test_*.py' -v
```

The tests cover multi-provider agreement and disagreement, chain/network
mismatch, bounded endpoint validation, content-addressed deduplication,
append-only indexing, malformed JSON, artifact tampering, negative-corpus
generation, and offline verification. CI does not require live RPC access.

**Release status: READ-ONLY EVIDENCE COLLECTION. NO AUTHENTICATED LIGHT CLIENT.
NO ESCROW-STATE PROOF. NO BSC AUTHORIZATION. NO MAINNET FUNDS.**
