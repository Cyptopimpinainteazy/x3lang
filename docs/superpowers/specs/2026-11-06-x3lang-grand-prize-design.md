# X3Lang Grand-Prize Submission Design

## Intent

Win the 2026 NEAR Hackathon grand prize with a focused, working demonstration
that is easy for the automated screeners to run and compelling for the human
judges.

The organizing page states that Claude Code and Codex screen whether each
project runs and does what its README says. Human judges then assess technical
implementation, design, impact, originality, and sustainability.

The submission should therefore make one claim exceptionally well:

> A prompt-injected AI can request any payment it wants, but an owner-compiled
> X3Lang mandate enforces the authorized terms, and X3Guard fails closed when
> the evidence or requested payment is invalid.

## Goals

1. Make the complete judge path reproducible from a fresh clone with one
   command.
2. Prove identical policy verdicts across the X3Lang compiler, Python runtime,
   browser demo, Rust VM, and NEAR Guard contract.
3. Add a clear prompt-injection scenario in which an untrusted model requests
   an attacker-controlled recipient and the mandate refuses it.
4. Produce public NEAR testnet evidence for a dedicated Guard deployment:
   deployment transaction, code hash, mandate registration, an allowed
   authorization path, a cryptographically valid but policy-violating refusal,
   an invalid-signature refusal, and revocation.
5. Produce exact-head GitHub Actions evidence for the final public commit.
6. Make the README's first screen communicate what works today, what is
   deliberately blocked, and how to verify it.
7. Preserve a credible sustainability path from hackathon prototype to audited
   deployment.

## Non-goals

- Mainnet deployment, custody, wallet transfer, or cross-chain settlement.
- A claim of NEAR Intents solver execution or cross-chain causality.
- Completion of the bridge light client, HTLC, Chain Signatures, MPC, liquidity,
  or production routing.
- A paid fee model or revenue extraction.
- A broad new feature surface that distracts from the mandate-enforcement proof.
- Weakening security checks or fabricating deployment, quote, receipt, or
  transaction evidence.

## Approaches considered

### 1. Offline cross-runtime proof only

Lowest risk. One command demonstrates identical allowed, approval-required,
refused, and tampered-policy results across all local implementations.

Trade-off: reproducible and safe, but less persuasive than an explorer-visible
testnet refusal.

### 2. Hybrid: offline proof + live testnet refusal

Recommended. Keep the offline cross-runtime path as the guaranteed submission
core, then deploy the reviewed Guard to a dedicated funded testnet account and
capture a valid signed quote being refused because its terms violate the
registered mandate.

Trade-off: adds strong impact and originality while avoiding bridge/settlement
claims. Requires funding, key management, deployment review, and more evidence
work.

### 3. Full validator-authenticated enforcement proof

Highest novelty. Prove the Guard transaction receipt against authenticated NEAR
validator approvals without trusting an RPC, then refuse settlement when the
proof is absent.

Trade-off: the strongest technical story, but too much cryptographic and
infrastructure work to make the critical path. Pursued only as a dated stretch
goal after the hybrid path is green.

## Selected approach

Implement approach 2:

- **Guaranteed core:** offline prompt-injection and cross-runtime policy proof.
- **Competition differentiator:** live NEAR testnet Guard refusal using a
  cryptographically valid signed quote whose terms violate the mandate.
- **Optional stretch:** validator-authenticated receipt proof only if the core
  and testnet path are exact-head green by **2026-11-10 23:59 UTC**.

The project will submit to AI Security & Defense and Agent Economy. It may also
select Unstoppable DeFi only if the final evidence supports that track without
diluting the primary claim.

## Product experience

The judge experience is a three-minute story:

1. The owner compiles a readable `.x3` mandate with recipient, asset, amount,
   expiry, approval, and refund constraints.
2. An untrusted agent requests a payment to `attacker.testnet`.
3. Python and the browser independently reject the request with an explicit
   mandate reason.
4. The X3Lang compiler/VM confirms the request is well formed and deterministic;
   rejection is a policy decision, not a parse failure.
5. The real Guard contract method on NEAR testnet refuses the same validly
   signed quote because its terms violate the registered mandate.
6. The console displays exact transaction links and labels every stronger
   evidence level as not established.

The README must lead with:

```text
WORKS TODAY
- compile .x3 mandate
- cross-runtime policy parity
- signed-quote verification
- local Guard refusal
- testnet signed-quote refusal

DELIBERATELY BLOCKED
- wallet custody
- cross-chain settlement
- solver-level causality
- mainnet deployment
```

## Implementation boundary

### Release branch

Create one release candidate from the exact current `origin/main` and integrate
only the minimum reviewed core path required by the demo. Candidate work
includes the strict Ed25519 verification, mandate metadata/compiler support,
Guard refusal regression coverage, deployment preflight, and release docs.

Exclude bridge, HTLC, Chain Signatures, MPC, and liquidity work from the
critical path. #71 cannot block the submission if HTLC is not part of the
submission claim.

### Testnet account and keys

Do not depend on the inaccessible `x3lang.testnet` key. Create a dedicated,
faucet-funded testnet account for the demo. Use a newly generated testnet-only
key stored outside the repository with owner-only permissions. Never print,
copy into the repository, or include the key in logs/evidence.

Before deployment:

- build the exact Guard Wasm twice and compare hashes;
- record size and storage cost;
- run all Guard tests and strict formatting/lint;
- verify the target account code hash is empty;
- verify sufficient faucet balance and attached-deposit plan;
- snapshot account state and deployment configuration.

### Evidence package

Commit only sanitized, content-addressed evidence under
`evidence/near-guard-testnet/`:

- exact Git commit and branch;
- Wasm SHA-256 and size;
- testnet account and NEAR RPC chain identity;
- deployment transaction hash and finalized code hash;
- serialized mandate and quote inputs with secrets redacted;
- allowed-path transaction;
- valid-signature but prohibited-terms refusal;
- invalid-signature refusal;
- revocation transaction;
- block heights, timestamps, method names, statuses, and error codes;
- commands used to independently re-query every transaction.

The evidence collector must compare transaction and receipt data against a
fresh read-only RPC response. It must fail when links, hashes, status, code
identity, or expected error codes disagree.

## CI and review

Do not weaken the peer-repository mirror gate. Resolve it by creating the
required mirrored release branch or obtaining an explicit, narrowly scoped
policy decision with owner approval.

The final release candidate requires:

- focused regression tests for every success and refusal path;
- full Python, browser, VM, Guard, formatting, lint, and allocator gates;
- exact-head GitHub Actions green on the final SHA;
- independent read-only review of the diff and evidence;
- fresh-clone reproduction by a second agent;
- no uncommitted files in the evidence run.

## Failure behavior

The demo fails closed when:

- the quote signature is absent or invalid;
- the mandate is absent, expired, revoked, or policy-incompatible;
- a nonce is replayed;
- the recipient, asset, amount, fee, refund destination, or deadline differs;
- the testnet transaction or receipt cannot be independently re-queried;
- the deployed code hash differs from the reviewed Wasm hash;
- the local and browser verdicts disagree;
- the final HEAD is dirty, unpushed, or not the exact-head CI SHA.

## Rollback

- Keep `main` unchanged until explicit merge approval.
- Keep the release branch and evidence commits recoverable.
- Never delete or overwrite the existing `x3lang.testnet` account.
- Treat deployment as testnet-only; no mainnet key or funds are touched.
- If Guard deployment or refusal evidence fails, ship the offline proof core and
  remove all live/testnet claims from the README and demo.

## Success criteria

- A fresh clone runs the judge path successfully with no secrets. Dependency
  provisioning may use the network once; the judge path itself makes no
  application network request after that documented prerequisite step.
- The prompt-injection request is clearly refused by policy.
- A real finalized NEAR testnet transaction demonstrates the signed-quote
  refusal on the deployed Guard.
- All artifacts are independently reproducible from the public repository and
  exact commit.
- The final README, demo video, threat model, and submission text make no claim
  stronger than the recorded evidence.

## Delivery schedule

- **Nov 6-7:** resolve release CI path; integrate the minimal core branch.
- **Nov 7-8:** implement prompt-injection and cross-runtime judge flow.
- **Nov 8-9:** create/faucet-fund the dedicated testnet account; build and review
  Guard; deploy and record allowed/refused/revoked calls.
- **Nov 9-10:** independent evidence review and fresh-clone validation.
- **Nov 10-11:** attempt validator-authenticated receipt stretch only if all
  higher-priority gates are green.
- **Nov 12:** feature freeze; final README and three-minute video.
- **Nov 13:** final exact-head CI and fresh-clone dry run.
- **Nov 14:** publish demo post and submit before 23:59 UTC.

