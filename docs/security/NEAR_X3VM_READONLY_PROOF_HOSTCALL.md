# X3VM read-only NEAR receipt proof opcode (experimental, fail-closed)

**Status: no independently authenticated checkpoint; never authorizes signing, escrow release, atomic settlement, custody, deployment or any funds movement.** This development evidence is intentionally separate from the Python NEAR mandate adapter and the on-chain `x3-guard` contract. This does not make the five NEAR mandate examples runnable as generic VM-protected mandates.

## What is implemented

- Rust `.x3` frontend accepts `verify_near_receipt(proof_json, context_json, "UNTRUSTED_HEAD_NO_SIGNING")`. It lowers into the typed `ProofVerify` X3BC capability with dedicated `kind = 3` (not an arbitrary unverified `CALL_HOST`). Production bytecode is checked by X3VM's usual verifier before execution.
- Actual Rust X3VM dispatch (`vm/src/executor.rs`) independently reconstructs genuine NEAR lite-header SHA256/Borsh hash, `PartialExecutionOutcome` Borsh leaf, receipt→shard→block outcome Merkle paths and block→caller-supplied head root path. Only historical RPC provenance exists for that head root; no consensus trust anchor is supplied. Returns the fatal `X3_NEAR_HEAD_UNTRUSTED` error for a matching outcome; malformed/altered evidence returns `X3_NEAR_PROOF_INVALID`.
- **The VM itself validates `kind=3` and returns a fatal refusal regardless of what bridge adapter is wired.** Neither a custom adapter reply nor an ordinary `.x3` failure-handler branch may turn this untrusted claim into successful execution. Direct calls to the existing `ProductionBridgeAdapter` and default `DryRunBridge` refuse kind 3 as well.
- `vm/src/near_evidence.rs::inspect_receipt_untrusted` is a pure read-only API returning an observation with `authenticated=false` and `authorized_to_sign=false`, never a transferable settlement certificate.

## Reproduce with real archived data (no chain RPC or wallets)

From root, after dependencies are available:

```bash
(cd x3-lang && cargo test --locked -p x3-lang-vm --test near_evidence_real -- --nocapture)
(cd x3-lang && cargo test --locked -p x3-lang-compiler near_receipt_inspection_compiles_as_a_dedicated_proof_capability)
(cd x3-lang && cargo fmt --all -- --check)
(cd x3-lang && cargo clippy --all-targets -p x3-lang-vm --locked -- -D warnings)
```

The fixture is the **genuine** archived testnet RPC receipt at `evidence/near-proof/testnet/sha256/0f/0fb16971ff172bd30e6ada514a3c9ea688ac77815ea506f28be09b6b3512d5ab.json`. It belongs to `league-v7.near-post.testnet`, **not** an X3 smart contract or escrow.

The `near_evidence_real` test runs source-generated `.x3` through the real Rust compiler, X3BC emission and the actual VM opcode; valid Merkle evidence is rejected at the **untrusted head** boundary. Corrupting gas, receipt ID, executor, outcome/block branch, header root or caller root is rejected. An intentionally installed failure handler cannot catch-and-suppress the lack of trust. No synthetic positive blockchain proof is used. This is an *end-to-end refusal demonstration*, not a successful bridge or proof of mainnet finality.

## Explicit limitations and next gates

1. No independently authenticated genesis/checkpoint, approved current validator set or >2/3 signed real live head; caller's historical RPC root is untrusted. This test cannot assert consensus finality.
2. No authenticated NEAR storage state, contract code identity or X3-owned vault receipt. No `Claimed`/`Refunded`/`NotFunded` settlement or BSC causality evidence.
3. `.x3` mandate policy/NEAR Intents signed-quote enforcement remains in separate Python adapter/`x3-guard` components; compilation of a NEAR proof instruction is **not** a production mandate-to-bytecode enforcement integration.
4. This proof opcode intentionally **never** returns success to an executable `.x3` instruction while it accepts only an untrusted RPC root. After an independently approved checkpoint and reviewed validator history are in place, a separate signed-off design is required before permitting any authenticated success branch.
5. Contract deployment, keys, signing, and fund movement are explicitly out of scope.

The dedicated CI job `.github/workflows/x3lang-near-vm-untrusted-proof.yml` rebuilds and verifies the real archived fixture on each changed PR exact head; its success is **not** authorization to spend.

Independent review and CI exact-head gates are required before merging this experimental work. The supporting deployment/config hardening is stacked in draft PR #70 and is itself not production permission.
