# X3 Shield — Judge Guide

**Status: November 6, 2026 (draft; verify exact HEAD before judging).** Distinguish tested code, signed quotes, observed
chain facts, and proven causal settlement. This repository has no linked
public mainnet deployment/payment evidence at this point. Do not invent any.

**Track focus:** AI Security & Defense, Agent Economy, Unstoppable DeFi.

## Why X3 Shield is different from the NEAR Agent API

**This is not an entry about inventing spending limits.** NEAR's Agent API
already offers policy-bound wallets, and NEAR function-call keys scope an
account key's contract/method permissions. Those are baseline controls, not
original X3 technology.

X3 Shield's proposed differentiation is a *separate, auditable transaction
policy*: an owner expresses constraints in `.x3`; the NEAR adapter derives
candidate intent/quote rules; the `x3-guard` contract verifies the exact signed
1Click quote against a pinned manager key **before** applying the stored
mandate. This detects a **validly signed but policy-prohibited quote**, rather
than merely an invalid signature or a wallet allowance violation.

| Mechanism | Security question | Scope and limitation |
| --- | --- | --- |
| Agent API policy-bound wallet | Can this agent use the wallet within its limits? | Provider/wallet authorization; not X3's invention |
| NEAR function-call access key | Can this key call a scoped method with its allowance? | On-chain account-key capability, not signed quote semantics |
| X3 intent / mandate policy | Are these recipient, asset, refund, fee, amount, deadline, nonce and approval terms allowed? | Local adapter and Guard checks; Rust compiler currently preserves adapter metadata but does **not** run the mandate guard inside general X3VM |
| X3 Guard signed-quote gate | Is the exact 1Click manager-signed quote allowed under the stored policy? | Implemented in code/tests; no independently verified on-chain testnet deployment/refusal yet |
| X3 NEAR proof work | Does real receipt evidence match block and Merkle commitments? | Genuine archived evidence and read-only VM refusal in experimental draft #72; still lacks an independently trusted checkpoint |

**What judges should demand:** a testnet transaction whose manager signature
checks out but whose signed quote terms violate a mandate, showing an actual
Guard-level refusal under the deployed code hash. That evidence is **not yet
available** and is tracked in [issue #73](https://github.com/Cyptopimpinainteazy/x3lang/issues/73).
Do not replace it with an unsigned dry quote, invalid signature or screenshot.

**The desired eventual flow** is agent request → `.x3` policy → pinned-key
signed 1Click quote → deployed Guard's permit/refuse decision → authentic NEAR
consensus proof of that decision. The current repo proves separate local
components; the complete, independently authenticated end-to-end flow remains
blocked. Signing or deploying to testnet requires explicit owner approval.

### References to the baseline

- [NEAR Hackathon — AI Security & Defense](https://near.army/) explicitly includes spending limits and policy engines.
- [Track 06: AI Security & Defense starter](https://github.com/nearai/near-hackathon-prompt-examples/blob/main/tracks/06-security.md) calls out agent security and function-call keys.
- [NEAR Intents 1Click quote reference](https://github.com/near/agent-skills/blob/main/skills/near-intents/rules/api-quote.md) distinguishes dry quote previews from real deposit-address quotes.
- [X3 Guard contract](../../x3-lang/near/x3-guard/src/lib.rs) implements pinned-key quote-signature checking followed by policy guards.

## Deterministic Guard-level signed-quote refusal (local mock chain)

[Reproduce the scoped contract test and see it in the browser](X3_SHIELD_SIGNED_QUOTE_REFUSAL_DEMO.md): a correctly signed quote using the repo's deliberately **test-only** Ed25519 signer tries to pay `attacker.near`. `agent_accept_quote` rejects it after verification, records no accepted quote and no spend, while an allowed signed quote succeeds. The judge console's evidence panel runs and labels this test separately with `--with-contract`. This is **not** a genuine 1Click production-manager-signed live quote or a testnet transaction. Issue #73 still tracks that milestone.

## Testnet Guard deployment readiness (no keys or transactions)

A [read-only deployment feasibility check](GUARD_TESTNET_DEPLOYMENT_READINESS.md)
queries the real `x3lang.testnet` account and current NEAR Testnet protocol
storage-staking price from two RPC endpoints. The initial 545,640-byte local
WASM alone needs about 5.46 NEAR storage stake, more than the public
account's observed 2 NEAR balance. **The contract has not been deployed.**
The official 1Click manager signature also must not be confused with a
community-run testnet quote signature. See Issue #73 for the live Guard
refusal evidence still needed; no owner deployment approval has been given.

## Genuine NEAR validator-signature evidence (separate unmerged draft)

Draft [PR #60](https://github.com/Cyptopimpinainteazy/x3lang/pull/60), commit
`33d0583c7b7789c5d0c50dcd92e84c4074f87ee3`, has a Rust read-only
light-client candidate checker. On x3star1 it reproduced two archived genuine
NEAR testnet epoch transitions: 12 correctly verified Ed25519 approvals in the
HbM → p27 transition (candidate height 276392452), then 19 approvals in the
p27 → GbZ transition (candidate height 276420402). The verifier checks the
signed stake's strict >2/3 threshold against the preceding recorded validator
set and validates their Borsh commitments. All three archived normalized JSON
RPC responses are SHA-256 pinned: `acf673ba...`, `dca4ca9c...`, `c47f0c38...`.

**This is real cryptography, not independently authenticated finality.** The
first HbM candidate and its validator set were obtained through public RPC,
not an independently reviewed trust-anchor ceremony. Valid signatures under
an untrusted initial key set do **not** prove the canonical NEAR chain. The
review packet in draft [PR #61](https://github.com/Cyptopimpinainteazy/x3lang/pull/61)
and the two-provider consistency report in draft
[PR #62](https://github.com/Cyptopimpinainteazy/x3lang/pull/62)
remain explicitly unsigned/unapproved. No signing authorization follows.

To reproduce from PR #60's source checkout (this guide's own branch does not
contain that unmerged code):

```bash
cargo run --locked --quiet \
  --manifest-path x3-lang/bridge/lightclient-core/Cargo.toml \
  --bin check_candidate_epochs -- \
  x3-lang/bridge/lightclient-core/tests/fixtures/hbm-announces-p27.json \
  x3-lang/bridge/lightclient-core/tests/fixtures/p27-announces-gbz.json \
  evidence/near-proof/testnet/sha256/c4/c47f0c381dbd9a37ac432aa4f77cf9d47ea8ccc452bfe4ff2a67b868953caed7.json
```

This separate evidence cannot be substituted for the still-missing **Guard's
own testnet refusal receipt**, which must be authenticated against a trusted
chain head before claiming independently proven enforcement.

## Scope and historical disclosure

Read [PRIOR_WORK.md](../../PRIOR_WORK.md) and
[baseline...main](https://github.com/Cyptopimpinainteazy/x3lang/compare/baseline...main).
The X3Lang compiler/trading VM predate the hackathon. NEAR Intents, X3 Guard,
agent mandates and the chain-evidence integration are newly reviewed features.
Do not attribute historical code to the contest window.

## Reproduce with one command — do X, see Y

For the best first-run experience on an Ubuntu machine with Rust/Cargo and
Node.js installed, run from the repo root:

```bash
bash scripts/hackathon/judge_one_command.sh /tmp/x3-shield-judge
```

The wrapper selects Python 3.11+ because the current NEAR evidence parser uses
`datetime.UTC`. If the required Python packages are absent, it installs them
in an isolated user-cache virtual environment (internet needed for this
one-time dependency installation). It never accesses wallet keys or sends a
transaction; it invokes the existing `judge_evidence.py --mode quick` gate.

Run from this repository's root, using Python 3.11+ / pytest, Node 22+ and
Rust/Cargo. Full mode also requires wasm32-unknown-unknown.

| Do X | Expected Y | What it proves |
| --- | --- | --- |
| python3 scripts/judge_evidence.py --mode quick --report-dir /tmp/x3-judge | Exit 0; evidence.json and evidence.md | Local Python, browser, Guard and allocator checks |
| python3 scripts/judge_evidence.py --mode full --report-dir /tmp/x3-judge-full | Exit 0; Rust VM/fmt/Clippy/WASM also tested | Expanded local reproducibility |
| cat x3-lang/examples/near_mandate_agent.x3 | Owner, limits, recipient allowlists and approvals | Guard policy source, not wallet custody |
| cat x3-lang/examples/near_intents_agent_payment.x3 | Agent intent under mandate | Example program, not live execution |
| Open https://x3-guard.pages.dev | Signed dry quote policy checks | No actual payment was sent |
| bash x3-lang/near/first-swap.sh | Prints the supported commands | Manual swap workflow; no funds moved by this command |

Set up a Python virtual environment rather than using global/root pip:

    python3 -m venv /tmp/x3-judge-venv
    /tmp/x3-judge-venv/bin/python -m pip install -r x3-lang/requirements.txt pytest
    /tmp/x3-judge-venv/bin/python scripts/judge_evidence.py --mode quick --report-dir /tmp/x3-judge

The evidence gate does NOT sign, deploy, send funds or read wallet keys.
It records the exact commit, whether the worktree was modified, test status,
and SHA256 digests of command outputs. It deliberately does NOT save the
raw log or any secret-filled receipt to published artifacts. Report files
have private 0600 permissions; review before publication.

## Real blockchain evidence — only after owner-authorized live transfers

Keep the original raw receipt **private** and out of the public repo.
A receipt's local verified_outcome field is user-editable JSON and not proof.
When a live X3 swap has actually occurred:

    python3 scripts/judge_evidence.py --skip-tests --receipt /private/receipt.json --refresh-chain --require-live --report-dir /tmp/x3-live

Current safe behavior: the judge gate **does not** promote imported receipts to
independently verified Solana deposits, NEAR deliveries, or refunds. With
`--refresh-chain`, it rechecks quote signatures and signed-request/receipt
bindings but **does not query NEAR or Solana RPC endpoints**. It returns
`unverified_receipt_chain_binding` for valid live quotes because the receipt's
leg, token mapping, and historical balance baseline are not cryptographically
bound to the signed request. A true independent chain proof requires a new,
reviewed verifier that reconstructs those facts from authenticated data.
`--require-live` currently **always fails with exit 5** even for an authentic
live quote. It is a mandatory hard stop, not a live-payment success badge.

For a purported refund, the `--refund-tx` option currently **fails with exit
2** because the judge does not have an independently authenticated
quote-to-refund chain-binding path. The lower-level adapter's
`verify-settlement --refund-tx` is a separate diagnostic and must not be
confused with independently verified hackathon submission evidence.

Once a reviewed independent RPC verifier is implemented, configure
`SOLANA_RPC_URLS` and `NEAR_RPC_URLS` to comma-separated trusted endpoints;
one RPC endpoint is not a verification quorum. Missing/error/disagreement
must produce unverified, never an optimistic guess.

Exit 0 means all tests **that were actually executed** passed and no live-proof requirement failed; `--skip-tests` runs no tests and may still exit 0. Receipt classification alone does not change exit 0 without `--require-live`. Exit 1 = test failure. Exit 2 = invalid invocation, unsupported `--refund-tx`, missing/unreadable/oversize receipt, or receipt read/parsing error; parseable but structurally untrusted JSON is classified as unverified.
Exit 5 = the requested live-evidence gate is not implemented and cannot promote imported local receipts.

**Evidence limit:** A finalized Solana SPL transfer and NEAR NEP-141 balance
rise DO NOT by themselves prove that one particular solver caused both.
This gate always reports cross_chain_causality_proven=false. Signed 1Click
quotes prove signed terms, not payment.

X3 Guard holds no customer funds. Its mandates restrict Guard methods, not
a third party who has direct access to an originating wallet's private key.
Do not claim that it can stop all wallet overspending unless wallet/contract
enforcement is independently proven.

## Public evidence matrix — pending actual chain events

| Claim | Required real artifact | Current status |
| --- | --- | --- |
| Mainnet Guard deployment | Contract address and deployment transaction | PENDING |
| Owner mandate and revocation | NEAR contract transaction hashes | PENDING |
| Allowed agent acceptance | NEAR tx + nonce + quote guard result | PENDING |
| Forbidden payee/overspend refused | NEAR failure tx + error | PENDING |
| Solana USDC deposit | Finalized source transfer tx + mint and amount | PENDING |
| NEAR USDC receipt | Destination tx / balance observation, with attribution caveat | PENDING |
| Real model generates .x3 policy | Model/provider, actual output, compiled artifact | PENDING |
| Cryptographic solver causality | Independent cross-chain attribution proof | NOT PROVEN |

Never treat mock signatures or bundled test fixtures as mainnet evidence.

## Three-minute live demo script

**00:00–00:25** Explain the unsafe-agent problem and show the owner mandate.

**00:25–01:05** A real AI model proposes an X3Lang intent. Show model
call, compiler and output. If it is only a handwritten example, say so.

**01:05–01:45** Verify a real signed quote, accept one authorized action,
then try a malicious recipient / over-limit amount and show the Guard refusal.

**01:45–02:35** Show real NEAR transaction URLs and Solana deposit evidence
if authorized transfers exist; label every observed proof level accurately.

**02:35–03:00** Show the judge gate's test results, baseline comparison,
source code and what has **not** yet been proven.

## Submission checklist

The organizers require a public repository and public post on X explicitly
saying the project was built for NEAR HACKATHON, tagging @NEARProtocol
with #NEARHackathon, including video or screenshots. Both links go in
the final form before **November 14, 2026, 23:59 UTC**. Submissions
cannot be edited after sending; check everything first.
See [official rules](https://nearhackathon.com/) and
[organizer HACKATHON.md](https://nearhackathon.com/HACKATHON.md).
