# X3Lang — reproducible compiler/VM + real NEAR cryptographic evidence

**Release classification: OFFLINE DEVELOPMENT DEMONSTRATION. No signing, custody, deployed X3 escrow, settlement authorization, or chain transfer.**

## One command

On an Ubuntu development host with Rust/Cargo, Python >=3.11 and a standard Python venv/pip installation:

```bash
bash scripts/hackathon/verify_cryptographic_demo.sh /tmp/x3lang-judge-crypto-demo
```

The default output directory is `/tmp/x3lang-judge-crypto-demo` if the argument is omitted. On the first invocation, the wrapper may download and install repository Python requirements into an isolated user-cache virtual environment or fetch lockfile-pinned Cargo crates if they are absent; those bootstrap operations may require internet. Once dependencies are provisioned the actual **demo itself makes no blockchain RPC/network calls**. The wrapper calls `cargo build --locked --offline --manifest-path x3-lang/Cargo.toml -p x3-tools --bin x3c` (with a one-time `cargo fetch` fallback), runs the actual Rust compiler/checker on `x3-lang/examples/timeout_refund_minimal.x3`, executes compiled `.x3b` in the real **local dry-run** VM, and verifies *archived* genuine NEAR testnet evidence. It requires no RPC/network connectivity for the cryptographic verification or local VM execution. Package provisioning can use network, with an explicit message. **There is no mock compiler fallback.**

Optional explicit preparation on a fresh machine (initial installation requires standard package network access and an appropriate Rust toolchain):

```bash
python3 -m venv .venv-demo
. .venv-demo/bin/activate
python -m pip install -r x3-lang/requirements.txt
cargo fetch --locked --manifest-path x3-lang/Cargo.toml
bash scripts/hackathon/verify_cryptographic_demo.sh /tmp/x3lang-judge-crypto-demo
```

The report is `/tmp/x3lang-judge-crypto-demo/report.json`, with the compiled artifact `/tmp/x3lang-judge-crypto-demo/timeout_refund_minimal.x3b`. This report contains exact Git HEAD, source and artifact SHA256, CLI build/execution results, genuine archived proof fixture SHA256, Merkle proof and validator commitment results, and tampering attempts. None of these are claimed as financial settlements.

## Five judge panels

1. **Actual X3Lang source**: show `x3-lang/examples/timeout_refund_minimal.x3`. It declares a route, nonce/finality requirements and a timeout/refund policy. This is a Rust `x3c`-compatible source in the current compiler. It is a **local demo policy**, not a NEAR settlement program.
2. **Real compiler + VM**: `x3c check`, `x3c build`, and `x3c run` are invoked as separate OS processes. The run uses a dry-run local VM. An emitted VM receipt is **not** a NEAR on-chain receipt or confirmation of a bridge transfer.
3. **Real NEAR testnet evidence**: the raw historical proof fixture `near_live_receipt_proof_20261105.json` is SHA-256 pinned (`0fb16971ff172bd30e6ada514a3c9ea688ac77815ea506f28be09b6b3512d5ab`) from X3Lang evidence PR #57. The genuine NEAR receipt ID `3f9FCzqC8k75AGob2UrCxbKVqADRyv5M5vGfoLEXbZyu` belongs to **`league-v7.near-post.testnet`**, NOT X3 escrow. The cryptographic verifier reconstructs its Borsh outcome leaf and checks receipt → shard → block outcome root `7mUR47bB4y7d7MbakknJGSdqmpCEDL5X47Y5LMq9BQnp`, plus a block-Merkle path against historical RPC head root `6F2mBDUkzB8tRpETPpRTtH5zo1E5yGGpftfyNYbRVDVw`. **That head root is not authenticated by a pinned independent checkpoint.**
4. **Validator commitment and provider cross-check**: the real 20-validator `next_bps` fixture (`c47f0c381dbd9a37ac432aa4f77cf9d47ea8ccc452bfe4ff2a67b868953caed7`) is re-encoded and compared with the light header's commitment. The archived two-provider report is integrity checked; provider agreement is NOT an independently trusted validator quorum.
5. **Attack demonstrations + release boundary**: modify real archived proof fields in memory (gas, executor, branch orientation), and real validator stake. Cryptographic verification rejects each corrupted sample. An intact Merkle path or matching list commitment does **not** authorize signing. The report always sets `trusted_checkpoint=false`, `escrow_storage_authenticated=false`, `cross_chain_causality_proven=false`, `authorized_to_sign=false`, and `funds_moved=false`.

### Expected local output

```text
X3Lang real compiler/VM dry-run: PASS | source=x3-lang/examples/timeout_refund_minimal.x3
Real archived NEAR outcome + block Merkle proof: MATCH (root NOT authenticated)
Real NEAR validator-list commitment: MATCH (validator set NOT authenticated)
Two provider metadata observations: CONSISTENT (NOT independently trusted)
Adversarial rejected: 4 / 4
TRUSTED CHECKPOINT: FALSE | AUTHORIZED TO SIGN: FALSE | FUNDS MOVED: FALSE
```

The actual artifact SHA256 is printed and recorded, rather than assumed constant across compiler builds.

## Limitations and release blockers

- **Not an integrated cross-chain hostcall**: a correct Python NEAR proof does not automatically become input to an authenticated X3VM runtime. The language program and cryptographic verifier execute as two distinct, auditable demonstration components.
- **Not a trusted light-client checkpoint**: no independent trust-anchor ceremony, authenticated genesis/checkpoint/validator history or third-party review has occurred.
- **Not X3 escrow settlement**: the genuine receipt is unrelated to any X3 contract; no deployed X3 escrow code identity, terminal `Claimed`/`Refunded`/`NotFunded` storage proof, bridge atomicity or BSC causality is established.
- **Not wallet authorization**: no keys, RPC transactions, contracts deployed, EIP-712 signatures, committee certificates, or funded testnet/mainnet transactions.
- **Known compiler-surface difference**: `near_intents_agent_payment.x3` contains an `under mandate` clause accepted by the Python preview adapter but not by the current Rust `x3c` parser. This demo deliberately uses the already accepted `timeout_refund_minimal.x3`; do not imply all examples compile in both frontends.
- **CI mirror gate**: the repo still requires corresponding changed sources in read-only `xxxstar`; a missing matching feature branch is a separate blocker, never waived by this demo.

For independent review see `docs/security/NEAR_CHECKPOINT_PROVIDER_AND_MIRROR_REVIEW.md`, draft PRs #59/#61/#62, and open security issue #56. The language and VM originated before the current hackathon; review `PRIOR_WORK.md` for authorship and timeline disclosure.

## Verify tests

```bash
PYTHONPATH=scripts/hackathon python -m unittest discover -s scripts/hackathon -p 'test_cryptographic_demo.py' -v
PYTHONPATH=x3-lang/bridge python -m unittest discover -s x3-lang/bridge -p 'test_*.py' -v
node --test x3-lang/near/demo/test/*.test.mjs
```

The test and default runner require the genuine Rust CLI; no fallback, mock receipt, stubbed provider, or synthetic positive proof is used.
