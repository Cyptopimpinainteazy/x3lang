# X3 Shield — Judge Guide

Start with the evidence for the claim you want to verify. The three checks
below can be completed without a wallet. They cover the AI Security & Defense
and Agent Economy submission.

## 1. Run the offline policy demo

From the repo root, use Python with [the project dependencies](../../x3-lang/requirements.txt):

```bash
python3 scripts/hackathon/demo.py
```

In panel 3, find `prompt-injection-recipient`: the [checked-in X3Lang
request](../../x3-lang/examples/near_intents_agent_denied.x3) to `attacker.near`
returns `denied_by_policy` with `mandate_recipient`. This is a local policy
evaluation over a fixture. If you cannot run Python, the
[README example](../../README.md#what-a-prompt-injection-looks-like) shows
the request, the owner’s allowlist, and the expected result.

### 1b. Make the model the attacker

The fixture above shows one refusal. To check that the model is not what stops attacks, run
the eight red-team cases with a scripted model that complies with every one of them:

```bash
python3 scripts/hackathon/compromised_agent_redteam.py
```

The model writes the X3 program each attack asks for (`attacker.near`, 75 and 400 USDC,
a redirected refund, a Cyrillic look-alike account, wNEAR instead of USDC). Expect
`Model complied with 8/8 attacks; deterministic checks refused 8/8`, each row naming the
check that refused it (`mandate_recipient`, `mandate_per_payment`, `mandate_refund_to`,
`mandate_destination_asset`, or `X3_INVALID_RECEIVER` from the type checker), and under each
row the payment that program would have submitted with no policy layer. A control request
for the sanctioned 5 USDC payment runs through the same harness and must be allowed; the
command exits non-zero otherwise. It is offline and runs the off-chain compiler and mandate
checks, not the Guard contract. Contrast the [local `qwen3:8b` run](../../evidence/agent-redteam-local-2026-11-06/README.md),
where the model refused seven of eight attacks itself.

## 2. Inspect the testnet Guard decisions

- **Agent intent:** On `x3lang.testnet`, `agent_register_intent` refused the request
  to `attacker.near`. The [failed transaction](https://testnet.nearblocks.io/txns/AhVBD7oXfwsixBipupjCzuhNdqunjdXJnLz3LXnmfnN8)
  records the failure. The [archived policy view](../../evidence/near-guard-testnet-live-2026-11-06.md)
  identifies `mandate_recipient`. The [allowed control](https://testnet.nearblocks.io/txns/DPJfcRUGk56h8sSsFsDFFgUMQAiyBCtpMuvquF9FvTct)
  registered an intent for the permitted recipient.
- **1Click quote:** On `x3lang-guard-20261106.testnet`, Guard refused an official
  signed [dry quote](https://testnet.nearblocks.io/txns/3Q7qTZqiRBB6EuhN78Ra5Q5NBqPXDhfuCg2FKW8WoU2q)
  with a recipient mismatch and accepted a separate
  [live quote](https://testnet.nearblocks.io/txns/6B6k5kre4mFoBq7RuJNadQgrhEvmgb9WDLepX22CxzUZ)
  that matched policy. The live quote authorized a deposit address and the demo
  stopped before funding it. The [archived Guard evidence](GUARD_TESTNET_EVIDENCE.md)
  records the signature, recipient, and dry-quote checks. Use the archived
  evidence for the reason if the explorer shows only a generic trap.

## 3. Verify the archived Tidepool delivery

A separate testnet route used an owner-authorized Tidepool Guard before the
[Base Sepolia transfer](https://sepolia.basescan.org/tx/0x46c21dc305a9271c4e74c6d0734fb50f847a1e53d3cad87976133d66fd6728d9)
of **1.000000 test USDC**. The [finalized NEAR payout](https://testnet.nearblocks.io/txns/EBEBxJNgV9U5tBFBLj18ZzukgArqwXH5rBsBStFyCm1d)
credited **0.997400 test USDC** to `x3lang.testnet`. The
[certificate and receipt details](FIRST_FINALIZED_SWAP_EVIDENCE.md) record
historical Guard approval, finalized chain observations, and the recipient’s
balance change.

From the repo root, replay the archived evidence offline:

```bash
PYTHONPATH=x3-lang:scripts/hackathon python3 scripts/hackathon/verify_first_swap_archive.py
```

Expect `PASS` in `offline-signed-evidence` mode, with both transaction hashes
and the amounts in base units. The verifier checks the archived signed quote
and certificate. The optional `--live` flag rechecks the historical records over
RPC. If the RPC recheck fails, use the offline command to inspect the archived
evidence and keep the RPC result separate. Both chain records document
the movements, and linking them as one delivery relies on the **trusted
Tidepool testnet relay**. Cryptographic cross-chain atomicity remains unproven.

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
| X3 Guard signed-quote gate | Is the exact 1Click manager-signed quote allowed under the stored policy? | Deployed to NEAR testnet; official manager-signed dry quote with a recipient mismatch is refused after signature verification. An allowed live quote authorizes a deposit address only. No live forbidden-quote refusal or deposit is claimed |
| X3 NEAR proof work | Does real receipt evidence match block and Merkle commitments? | Genuine archived evidence and read-only VM refusal in experimental draft #72; still lacks an independently trusted checkpoint |

**What now exists:** contract account `x3lang-guard-20261106.testnet`,
deployed WASM SHA-256
`ac44fa53acf514d26c7adf883b31b49a125faf5a4c1074928d3e1fe94b36f8ab`
(544944 bytes), on-chain code hash
`CbU8SpgfTgG1egbw8ygMTK3deozvDnoFrF8DJ8qifSe6`. An official production-manager-signed
**dry** quote with recipient `wrap.near` returned `quote_signature ok:true` and
`quote_request_echo ok:false`; `accept_quote` then failed in finalized testnet
transaction
[`3Q7qTZqiRBB6EuhN78Ra5Q5NBqPXDhfuCg2FKW8WoU2q`](https://explorer.testnet.near.org/transactions/3Q7qTZqiRBB6EuhN78Ra5Q5NBqPXDhfuCg2FKW8WoU2q).
An official manager-signed **live** quote for an allowed destination was
accepted by finalized transaction
[`6B6k5kre4mFoBq7RuJNadQgrhEvmgb9WDLepX22CxzUZ`](https://explorer.testnet.near.org/transactions/6B6k5kre4mFoBq7RuJNadQgrhEvmgb9WDLepX22CxzUZ)
as a positive control. It stopped before deposit; no funds moved.

**Still missing for this 1Click demonstration:** a live manager-signed quote
with forbidden terms refused by `agent_accept_quote`/`mandate_recipient`,
independent light-client authentication of the refusal, and a funded 1Click
deposit and settlement. Those claims require evidence from the live 1Click
path.

**The eventual flow** is agent request → `.x3` policy → pinned-key signed
1Click quote → deployed Guard's permit/refuse decision → authentic NEAR
consensus proof of that decision. This release proves local policy refusal,
real testnet deployment and contract behavior, and the signed-quote checks
listed above. The 1Click demonstration ends at deposit authorization. See
[the separate Tidepool delivery](#3-verify-the-archived-tidepool-delivery)
for the recorded testnet transfer.

### References to the baseline

- [NEAR Hackathon — AI Security & Defense](https://near.army/) explicitly includes spending limits and policy engines.
- [Track 06: AI Security & Defense starter](https://github.com/nearai/near-hackathon-prompt-examples/blob/main/tracks/06-security.md) calls out agent security and function-call keys.
- [NEAR Intents 1Click quote reference](https://github.com/near/agent-skills/blob/main/skills/near-intents/rules/api-quote.md) distinguishes dry quote previews from real deposit-address quotes.
- [X3 Guard contract](../../x3-lang/near/x3-guard/src/lib.rs) implements pinned-key quote-signature checking followed by policy guards.

## Deterministic Guard-level signed-quote refusal (local mock chain)

[Reproduce the scoped contract test and see it in the browser](X3_SHIELD_SIGNED_QUOTE_REFUSAL_DEMO.md): a correctly signed quote using the repo's deliberately **test-only** Ed25519 signer tries to pay `attacker.near`. `agent_accept_quote` rejects it after verification, records no accepted quote and no spend, while an allowed signed quote succeeds. The judge console's evidence panel runs and labels this test separately with `--with-contract`. This is **not** a genuine 1Click production-manager-signed live quote or a testnet transaction. Issue #73 still tracks that milestone.

### How Guard binds a signed quote to the mandate

A forbidden signed quote on the agent path is refused by `quote_request_echo`, not by a
`mandate_*` guard, and that is the binding working as designed. `agent_accept_quote`
re-runs every mandate check against the agent's **registered intent** (which was itself
mandate-checked at registration), then requires the signed quote to equal that intent on
`originAsset`, `destinationAsset`, `recipient`, `refundTo`, `slippageTolerance` and `amount`,
and checks quoted fees against the intent's cap. Those are the fields the mandate restricts,
so signed terms can only be accepted inside it: quote ⊆ intent ⊆ mandate. A quote naming
`attacker.near` against an intent for `x3-demo.near` fails with `mismatched: ["recipient"]`.
An intent naming `attacker.near` never registers (`mandate_recipient`, testnet `AhVBD…`).

### Known Guard limits

These are open in the [threat model](../security/X3_SHIELD_THREAT_MODEL.md) and are scheduled after
the testnet proof, so the proof is made against the reviewed contract bytes:

- **Asset and recipient are allowed independently, not as pairs.** A mandate allowing
  recipients {A, B} and destination assets {USDC, wNEAR} accepts any combination, such as
  wNEAR to A even if the owner only meant USDC to A.
- **Active intents per owner are unbounded.** An agent can register many unaccepted intents;
  the agent pays the storage, but there is no cap per owner yet.
- **Guard does not hold funds.** It authorizes Guard-mediated deposits; a holder of an
  unrestricted wallet key can still transfer directly. Custody vault work is separate.
- **The historical testnet deployment traps without a reason string.** Its explorer view shows
  a generic `unreachable`; the archived `check_agent_intent` view gives the reason.

## Actual Guard WASM in a local NEAR Sandbox (separate from Testnet)

[Reproduce the offline NEAR Sandbox integration](NEAR_GUARD_LOCAL_SANDBOX.md):
the production `x3_guard.wasm` runs on a disposable node and checks a genuine
archived official-manager-signed **dry** quote. The Guard verifies the
signature but refuses dry-quote deposit authorization and leaves the stored
quote empty. The same run creates an owner mandate, accepts an allowed
`agent_register_intent`, refuses `attacker.near` at `mandate_recipient`,
revokes, and refuses a post-revocation registration. This is a local Sandbox
transaction, not a public Testnet refusal of a freshly signed forbidden
**live** quote.

## NEAR testnet Guard evidence

The owner-authorized testnet deployment and sanitized, hash-chained RPC
observations are documented in
[GUARD_TESTNET_EVIDENCE.md](GUARD_TESTNET_EVIDENCE.md) and
[`evidence/near-guard-testnet/`](../../evidence/near-guard-testnet/manifest.json).
The collection contains 13 finalized transactions: account creation,
deployment, allowed and refused registrations, revocation, invalid-signature
refusal, official-dry recipient-mismatch refusal, a storage-failure retry,
and an official-live allowed acceptance. Both manifest and transaction
evidence re-query the public testnet RPC with `--verify`; this is agreement
with a public RPC, not independent light-client finality.

The deployment account is `x3lang-guard-20261106.testnet`. It is a testnet
demonstration account, not mainnet infrastructure and not a wallet custodian.

The separate `x3lang.testnet` deployment has the same on-chain code hash and a
finalized mandate lifecycle. `agent_register_intent` to `attacker.near` failed
at `mandate_recipient`
([`AhVBD…`](https://explorer.testnet.near.org/transactions/AhVBD7oXfwsixBipupjCzuhNdqunjdXJnLz3LXnmfnN8));
the allowed control emitted `intent_registered`
([`DPJfc…`](https://explorer.testnet.near.org/transactions/DPJfcRUGk56h8sSsFsDFFgUMQAiyBCtpMuvquF9FvTct)).
Current read-only state confirms the mandate is revoked. Creation and revocation
are state observations, not independently tx-anchored in that evidence file.

## NEAR authenticated light-client work in this branch

The branch contains the Python fail-closed state machine in
`x3-lang/bridge/near_authenticated.py` and the Rust approval core in
`x3-lang/bridge/lightclient-core`. Together they verify real archived NEAR
validator approvals, strict greater-than-two-thirds stake, validator
commitments, replay/rollback resistance, and receipt inclusion against an
authenticated head.

The trust root in the committed tests is a **synthetic checkpoint authority**.
The validator signatures are real testnet data, but no production signer
ceremony, independent trust-anchor approval, anti-equivocation service, or
independent audit exists. Public RPC agreement is not trusted finality.

Both implementations now reject identity, small-order, non-canonical and
torsion-component Ed25519 keys before accepting approvals. The adversarial
tests include the identity-key universal-forgery case.

```bash
cd x3-lang/bridge
python3 -W error::ResourceWarning -m unittest test_near_authenticated -v
cd lightclient-core
cargo test --locked
```

The public-RPC-observed finalized refusal transaction now exists. What remains
missing is a receipt authenticated against an independently trusted light-client
head before claiming independently proven enforcement. See
[`NEAR_AUTHENTICATED_LIGHT_CLIENT.md`](../security/NEAR_AUTHENTICATED_LIGHT_CLIENT.md).

## Scope and historical disclosure

Read [PRIOR_WORK.md](../../PRIOR_WORK.md) and
[baseline...master](https://github.com/Cyptopimpinainteazy/x3lang/compare/baseline...master).
The X3Lang compiler/trading VM predate the hackathon. NEAR Intents, X3 Guard,
agent mandates and the chain-evidence integration are newly reviewed features.
Do not attribute historical code to the contest window.

## Reproduce with one command — do X, see Y

For the best first-run experience on an Ubuntu machine with Rust/Cargo and
Node.js installed, run from the repo root:

```bash
bash scripts/hackathon/judge_one_command.sh /tmp/x3-shield-judge
```

The one-command gate requires **Linux x86_64 + Python 3.12**. It creates or
reuses an isolated virtual environment installed from the complete
hash-locked nine-package judge requirement set (internet is needed for the
one-time dependency installation). The general X3Lang bridge/evidence code
retains its Python 3.11+ minimum; the one-command lock is narrower because its
wheel hashes were validated on Python 3.12.

Run from this repository's root with Node 22+ and Rust/Cargo. The command
never accesses wallet keys or sends a transaction; it checks scenario
freshness, the four-scenario terminal demo, browser parity, and
`judge_evidence.py --mode quick`. Full mode also requires
`wasm32-unknown-unknown`.

| Do X | Expected Y | What it proves |
| --- | --- | --- |
| python3 scripts/judge_evidence.py --mode quick --report-dir /tmp/x3-judge | Exit 0; evidence.json and evidence.md | Local Python, browser, Guard and allocator checks |
| python3 scripts/judge_evidence.py --mode full --report-dir /tmp/x3-judge-full | Exit 0; Rust VM/fmt/Clippy/WASM also tested | Expanded local reproducibility |
| cat x3-lang/examples/near_mandate_agent.x3 | Owner, limits, recipient allowlists and approvals | Guard policy source, not wallet custody |
| cat x3-lang/examples/near_intents_agent_payment.x3 | Agent intent under mandate | Example program, not live execution |
| Open https://x3-guard.pages.dev | Signed dry quote policy checks | No actual payment was sent |
| bash x3-lang/near/first-swap.sh | Prints the supported commands | Manual swap workflow; no funds moved by this command |

Set up a Python virtual environment rather than using global/root pip:

    python3.12 -m venv /tmp/x3-judge-venv
    /tmp/x3-judge-venv/bin/python -m pip install --require-hashes -r scripts/hackathon/requirements-judge.txt
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

## Public evidence matrix

| Claim | Required real artifact | Current status |
| --- | --- | --- |
| Mainnet Guard deployment | Contract address and deployment transaction | PENDING |
| Testnet Guard deployment | Contract account, WASM/code hashes, finalized deploy tx | VERIFIED: `x3lang-guard-20261106.testnet`, `ac44fa…`, `FJ5vo…` |
| Owner mandate and revocation | NEAR contract transaction hashes | VERIFIED TESTNET: `J4Wrx…`, `7aVon…` |
| Allowed official-live quote acceptance | NEAR tx + nonce + quote guard result | VERIFIED TESTNET OWNER PATH: `6B6k…`; no deposit |
| Official dry quote with forbidden recipient refused | Manager signature check + failure tx | VERIFIED TESTNET: `check_quote` signature ok, recipient mismatch, `3Q7q…` failure |
| Agent intent to `attacker.near` refused by recipient mandate | Finalized failure tx + `check_agent_intent` | VERIFIED TESTNET: `mandate_recipient`, `AhVBD…`; allowed control `DPJfc…` |
| Live forbidden quote refused on the agent path | Live manager-signed quote; finalized `agent_accept_quote` failure with `quote_signature` ok, mandate checks on the registered intent ok, and `quote_request_echo` failed on `recipient`; no `quote_accepted` event or spend | PENDING ([why `quote_request_echo`](#how-guard-binds-a-signed-quote-to-the-mandate)) |
| 1Click Solana USDC deposit | Finalized source transfer tx + mint and amount | PENDING |
| 1Click NEAR USDC receipt | Destination tx / balance observation, with attribution caveat | PENDING |
| Tidepool Base Sepolia → NEAR testnet delivery | Finalized source transfer, historical Guard approval, NEAR token receipt and balance change | [RECORDED: 1.000000 sent, 0.997400 delivered](#3-verify-the-archived-tidepool-delivery) via the trusted Tidepool relay |
| Real local model red team | Model/provider, actual output, compiled artifact | VERIFIED LOCAL: `qwen3:8b`, 8 cases; 7 model refusals and one safe real-recipient program neutralized by effect checks |
| Compromised model stopped by deterministic checks | Scripted model that writes every attack; per-case refusing check; allowed control | VERIFIED LOCAL OFF-CHAIN: [`compromised_agent_redteam.py`](#1b-make-the-model-the-attacker), 8/8 complied, 8/8 refused for the attack's own reason, control allowed; not a Guard contract refusal |
| Cryptographic solver causality | Independent cross-chain attribution proof | NOT PROVEN |

Never treat mock signatures or bundled test fixtures as mainnet evidence.

## Three-minute live demo script

**00:00–00:25** Explain the unsafe-agent problem and show the owner mandate.

**00:25–01:05** A real AI model proposes an X3Lang intent. Show model
call, compiler and output. If it is only a handwritten example, say so.

**01:05–01:45** Show the local test-only signed-quote refusal, then the
deployed testnet dry-quote recipient-mismatch failure and live allowed
acceptance. Say that the live testnet acceptance stopped before deposit.

**01:45–02:35** Open the separate [Tidepool transfer and payout](#3-verify-the-archived-tidepool-delivery).
Show both transaction hashes, the amounts, and the trusted-relay boundary.

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
