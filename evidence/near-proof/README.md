# NEAR testnet proof evidence

This directory was collected read-only from NEAR testnet public RPC on
2026-11-06. It contains genuine RPC responses, not fabricated chain data.

## Committed evidence

- Checkpoint candidate:
  [`testnet/manifests/9f20750427bf33d12750ad713707e472f86d46c45016c2580f70ab8f43386745.json`](testnet/manifests/9f20750427bf33d12750ad713707e472f86d46c45016c2580f70ab8f43386745.json)
  — status `disagreement` because the two providers sampled different finalized
  heads; both candidates remain preserved.
- Receipt light-client proof:
  [`testnet/manifests/26ca2546a31f8b9ba267aa3fadc154cfd2ea6f64ee81c0190f848f1b02148493.json`](testnet/manifests/26ca2546a31f8b9ba267aa3fadc154cfd2ea6f64ee81c0190f848f1b02148493.json)
  — status `complete_candidate`; identical across
  `test.rpc.fastnear.com` and `rpc.testnet.near.org`.
- Transaction light-client proof:
  [`testnet/manifests/9fd3bc8241df327644f0a2ffadb9649506be0c5dd1fcf0f31c8fed92abf20236.json`](testnet/manifests/9fd3bc8241df327644f0a2ffadb9649506be0c5dd1fcf0f31c8fed92abf20236.json)
  — status `complete_candidate`; identical across both providers.
- Negative corpus:
  [`testnet/mutations/manifest.json`](testnet/mutations/manifest.json) — 12
  labelled synthetic mutations derived from the real evidence.

The receipt proof targets:

- transaction `6RAnrVJY5P6XK8qhzBfYtWvpLn1ofxWyi6goXA9N3MgY`
- receipt `3f9FCzqC8k75AGob2UrCxbKVqADRyv5M5vGfoLEXbZyu`
- executor `league-v7.near-post.testnet`
- light-client head `V7KcHcg16BVWWxhVbw2vQj87D3dJQwXchnU7DCVzo6V`

## Verify offline

```bash
python3 x3-lang/bridge/near_evidence.py verify \
  --root evidence/near-proof --require-negative-corpus
```

## What this proves

The collector obtained and preserved genuine proof-shaped NEAR testnet
responses from two RPC providers. It did not authenticate NEAR consensus,
validator signatures, stake, epochs, Merkle inclusion, the deployed escrow code
identity, or terminal escrow storage state.

`authenticated` and `authorized_to_sign` are false throughout. No keys were
used, no transaction was signed, no contract was deployed, and no funds moved.
The independent verifier in issue #56 remains responsible for deciding whether
these artifacts establish a finalized chain fact.
