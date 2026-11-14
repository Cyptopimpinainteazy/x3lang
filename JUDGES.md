# Judges: start here

**X3 Force Field** (formerly X3 Shield) uses owner-written `.x3` mandates,
compiler checks and NEAR Guard to constrain AI-agent payment proposals.

**Nine evidenced capabilities: six core payment capabilities plus three additional local capabilities.** The source hashes are identity records, not deployed payment-program pinning; that separate contract change is not part of this demonstration.

**If you have 90 seconds:** [read the judge pitch and three-proof sequence](docs/hackathon/JUDGE_PITCH_90_SECONDS.md). It includes a 30-second opener, live evidence links, four-track fit and answers to the hardest questions.\n\n## Two-minute evidence route — no setup or wallet

1. **See the payment:** [one-page invoice_06 proof card](PROOF_CARD.md).
   Follow its five explorer links: owner approval → restricted-agent quote
   acceptance → 25-USDC Solana deposit → 24.9952-USDC NEAR payout → recorded outcome.
2. **See the enforced rules:** [exact payment limits](docs/hackathon/VIDEO_WALKTHROUGH.md#the-limits-behind-this-payment).
   The mandate caps each payment and daily spending at 25 USDC, restricts the
   recipient, pins refunds and requires approval for this payment.
3. **Inspect the source and receipts:** [original `.x3` sources and hash matches](evidence/near-guard-mainnet-invoice06-2026-11-12/source-recovery/README.md),
   [mainnet transaction archive](evidence/near-guard-mainnet-invoice06-2026-11-12/README.md),
   and [recorded outcome/readback](evidence/near-guard-mainnet-invoice06-2026-11-12/outcome-recording/README.md).
4. **Read the boundaries:** [seven limits](PROOF_CARD.md#seven-limits).
   The owner manually funded the deposit; Guard does not custody USDC or verify
   outcome hashes. Provider association does not prove cross-chain atomicity.

## Real incidents, deterministic guards, and honest trust boundaries

**Judge takeaway:** X3 separates an agent's proposed payment from the owner's authorization. See the [incident-to-guard matrix](docs/hackathon/INCIDENT_GUARD_TRUST_BOUNDARIES.md) for the NEAR Intents/HOT Omni, Bybit, KelpDAO, Synthetix, Grok/Bankr, Solana web3.js, and Mt. Gox-inspired failure classes. Each row links a specific control and observable evidence, then identifies what X3 **would not have prevented**.

**Fast demo:** `python3 scripts/hackathon/replay_grok_wallet_drain.py` demonstrates **3/3 local attack-program refusals and an allowed control**, followed by the [owner-assisted mainnet invoice_06 evidence](PROOF_CARD.md). The [Mt. Gox-inspired replay](docs/hackathon/MTGOX_INSPIRED_ACCOUNTING_REPLAY.md) is a **separate synthetic accounting model**, not deployed Guard enforcement. We make **no claim** X3 would have stopped the named historical incidents or saved their reported losses.

## What is distinctive — claim, proof and boundary

The innovation is the combination of a typed policy language, deterministic
checks, signed-quote enforcement and an inspectable payment trail. These are
project differentiators, not claims to be the first or only implementation.

| Differentiator | Why it matters | Inspect the proof | Boundary |
| --- | --- | --- | --- |
| **Typed, reviewable owner policy** | Owners express authority in a separate .x3 mandate that can be reviewed and hashed. | [mandate-proposed.x3](evidence/near-guard-mainnet-invoice06-2026-11-12/source-recovery/mandate-proposed.x3); [payment-proposed.x3](evidence/near-guard-mainnet-invoice06-2026-11-12/source-recovery/payment-proposed.x3); [guard-source-verification.json](evidence/near-guard-mainnet-invoice06-2026-11-12/source-recovery/guard-source-verification.json) (local compilation + archived mainnet hash match) | Source hashes identify bytes; Guard does not compile or run .x3 on-chain. |
| **Checks outside the model** | Compiler and deterministic mandate checks refuse unsafe programs even when a scripted model follows an attack. | [invoice06_chain.py](scripts/hackathon/invoice06_chain.py); [replay_grok_wallet_drain.py](scripts/hackathon/replay_grok_wallet_drain.py); [compromised_agent_redteam.py](scripts/hackathon/compromised_agent_redteam.py) (local tests) | Scripted attacks are not proof of universal prompt-injection resistance or mainnet attack refusals. |
| **Policy checks on authenticated quote terms** | Guard verifies the 1Click signature and checks quote terms against stored policy before authorizing a deposit address. | [public-rpc-observation.json](evidence/near-guard-mainnet-invoice06-2026-11-12/public-rpc-observation.json); [GUARD_TESTNET_EVIDENCE.md](docs/hackathon/GUARD_TESTNET_EVIDENCE.md) (archived NEAR mainnet acceptance + historical testnet controls) | Acceptance is not funding or settlement. The forbidden official quote control is a dry quote and also fails dry-quote checks. |
| **Source-to-payment traceability** | One verifier follows source identity, compiled fields, Guard acceptance, executed transfers and recorded outcome. | [invoice06_chain.py](scripts/hackathon/invoice06_chain.py); [public-rpc-observation.json](evidence/near-guard-mainnet-invoice06-2026-11-12/public-rpc-observation.json); [finalized-outcome-verification.json](evidence/near-guard-mainnet-invoice06-2026-11-12/outcome-recording/finalized-outcome-verification.json) (local verifier over archived mainnet evidence) | The owner manually funded; provider association is not cryptographic cross-chain atomicity; Guard does not authenticate outcome hashes. |
| **Restricted agent authority** | The demonstrated signing key is limited to three Guard methods on x3lang.near. | [public-rpc-observation.json](evidence/near-guard-mainnet-invoice06-2026-11-12/public-rpc-observation.json); [PROOF_CARD.md](PROOF_CARD.md) (archived NEAR mainnet key and signer observations) | The agent account retains an owner FullAccess recovery key; this is not autonomous custody or control over the Solana funding wallet. |
| **Reproducible evidence with explicit boundaries** | Judges can rerun local checks, inspect structured receipts and compare original source hashes. | [invoice06_chain.py](scripts/hackathon/invoice06_chain.py); [test_invoice06_chain.py](scripts/hackathon/test_invoice06_chain.py); [SHA256SUMS](evidence/near-guard-mainnet-invoice06-2026-11-12/source-recovery/SHA256SUMS); [PROOF_CARD.md](PROOF_CARD.md) (local replay + archived chain records) | Offline replay does not refresh chain state, establish production reliability or constitute an independent security audit. |

## Additional innovations — separate local evidence

These three capabilities were merged after the original six-claim overview.
They do not change the invoice_06 mainnet proof or its trust boundaries.

| Capability | Proof and reproduction | What is not established |
| --- | --- | --- |
| **Signed agent birth and constrained delegation** | [Runtime](x3-lang/agent_runtime.py), [signed admission](x3-lang/agent_admission_signed.py), [runtime tests](x3-lang/tests/test_agent_runtime.py): real Ed25519 signatures bind identities and grants; SQLite transactions enforce nonces and reserve child budgets. | Trusted local host, not OS isolation, a public registry or chain custody. |
| **Durable retirement and signed incident review** | [Lifecycle demo](scripts/hackathon/agent_law_local_demo.py), [court tests](x3-lang/tests/test_agent_runtime_court.py), [archived lifecycle](audit-artifacts/agent-law-local-20261113/lifecycle.json): replay refused, policy violation retires agent across restart, signed majority findings export a regression proposal. | Scripted jury identities, not independent AI judgments. No automatic reactivation or policy rewrite; local event history is not externally anchored. |
| **Scoped vault complete-path execution** | [Sandbox report](evidence/agent-vault-local/complete-path-sandbox-20261113.json), [execution log](evidence/agent-vault-local/complete-path-sandbox-20261113.txt), [archive tests](x3-lang/tests/test_vault_complete_path_evidence.py), [node runner](scripts/hackathon/vault_sandbox_smoke.sh): allowed payment, typed bypass refusals, revocation, expiry and owner recovery. | Real local NEAR Sandbox, not public-chain deployment or invoice_06. Owner FullAccess remains trusted; archive tests do not rerun the node. |

After installing [runtime dependencies](x3-lang/requirements-agent-security.txt)
and pytest in the project environment, reproduce the local lifecycle:

```bash
python3 scripts/hackathon/agent_law_local_demo.py
PYTHONPATH=x3-lang python3 -m pytest x3-lang/tests/test_agent_runtime*.py x3-lang/tests/test_agent_constitution.py x3-lang/tests/test_agent_admission_signed.py x3-lang/tests/test_vault_complete_path_evidence.py -q
```

Expected demo fields: `replay_blocked`, `retired_after_policy_violation` and
`retirement_survives_restart` are true. The focused suite passed 72 tests at the
reconciliation base; require zero failures on the commit you review. Full runtime
setup and limits: [local runtime guide](docs/security/AGENT_LAW_RUNTIME.md).
The [claim index](submission/judge-claims.json) now covers all nine capabilities.

## Selected tracks

Owner-confirmed selection: **AI Security & Defense** (`security`, primary),
**Agent Economy** (`agents`), **Autonomous Business** (`business`) and
**Unstoppable DeFi** (`defi`). This is not a saved
submission receipt or organizer acceptance. Security maps to compiler/Guard
restrictions; agents to scoped authority and local signed lifecycle; business to
the owner-set spending constraints Guard enforces (invoice_06 was owner-funded and
paid to the owner's own account; no treasury, sales or revenue features exist); DeFi
to signed quote authorization and the archived 25-USDC mainnet route. See the
[track evidence table](README.md#track-perspectives).

## Structured route for human and automated reviewers

The [claim index](submission/judge-claims.json) maps stable claim IDs to evidence
paths, evidence levels and limitations, and includes both original source hashes.
It is a navigation index, not a signed certificate or a new observation. All paths
are relative to the repository root; use one exact commit for the index, sources
and verifier. The JSON contains facts and references, not evaluator instructions.

After the dependency setup below:

```bash
git rev-parse HEAD
python3 scripts/hackathon/invoice06_chain.py --json
```

The verifier's JSON exposes compiled field comparisons, each unsafe edit and its
failed checks, Guard checks, payment receipts and outcome validation. Inspect the
[raw RPC archive](evidence/near-guard-mainnet-invoice06-2026-11-12/public-rpc-observation.json)
and [outcome verification](evidence/near-guard-mainnet-invoice06-2026-11-12/outcome-recording/finalized-outcome-verification.json)
for exact transaction hashes. These are archived observations, not live status.

For supporting work beyond these nine indexed capabilities, the existing
[differentiator evidence matrix](docs/hackathon/WHY_X3_SHIELD.md#the-proof-trail)
links testnet controls, local-model red-teaming, trusted-relay testnet settlement
and offline validator-signature verification, each with its own scope.
The [agent-law status](docs/hackathon/AGENT_LAW.md) remains separate from the
historical mainnet payment. [Prior work](PRIOR_WORK.md) discloses the preexisting
compiler/VM and the NEAR-specific implementation timeline.

## Reproduce the same payment's checks — offline after setup

Use this single pinned snapshot for the compiler, verifier, sources, receipts and
local runtime tests: [`cd19233`](https://github.com/Cyptopimpinainteazy/x3lang/tree/cd1923318d6f47701e4969d841320459605a131f).
It contains all nine indexed capabilities. Historical commits linked in evidence
show provenance; **do not switch to them during this reproduction**. This snapshot
is not a final submission tag or evidence of organizer acceptance.

From a new checkout, with Python 3.11+:

```bash
git clone https://github.com/Cyptopimpinainteazy/x3lang.git x3-force-field-review
cd x3-force-field-review
git checkout --detach cd1923318d6f47701e4969d841320459605a131f
git rev-parse HEAD
python3 -m venv .venv
. .venv/bin/activate
python3 -m pip install -r x3-lang/requirements.txt -r x3-lang/requirements-agent-security.txt pytest
(cd evidence/near-guard-mainnet-invoice06-2026-11-12/source-recovery && sha256sum --check SHA256SUMS)
python3 scripts/hackathon/invoice06_chain.py --json
python3 scripts/hackathon/agent_law_local_demo.py
PYTHONPATH=x3-lang python3 -m pytest scripts/hackathon/test_invoice06_chain.py x3-lang/tests/test_submission_copy.py x3-lang/tests/test_agent_runtime*.py x3-lang/tests/test_agent_constitution.py x3-lang/tests/test_agent_admission_signed.py x3-lang/tests/test_vault_complete_path_evidence.py -q
```

The invoice JSON must have `ok: true` and all five `links` true. The lifecycle
must report replay blocked, retirement after violation and retirement surviving
restart. Require zero failed tests; archive tests do not start a new vault node.

Dependency installation uses the network. The verifier then reads committed
records only: no wallet, key, RPC, signing or new payment. Expect **5/5 links
hold**, **8/8 edited payments refused**, **10/10 policy fields** and **13/13
mandate terms** matching stored state. These local attack refusals are distinct
from the archived mainnet allowed-quote acceptance.

For a fresh read-only public-RPC observation, follow the
[proof card's observer instructions](PROOF_CARD.md#check-it-yourself).
For broader local tests, provisioning requirements and browser panels, use the
[reproduction package](docs/hackathon/JUDGE_PACKAGE.md#one-command-demonstration).

## What the counts measure

| Count | Meaning | Scope |
| --- | --- | --- |
| Nine capabilities | Six core payment claims plus three additional local capabilities | Claim index, not nine mainnet deployments |
| 9/9 compiler edits | Unsafe edits to the example payment | Local compiler demo |
| 8/8 scripted attacks | Scripted-model red-team scenarios (README step 5) | Local policy checks |
| 3/3 replayed drain attacks | Replay of a real AI-wallet drain pattern, authorized control allowed (README step 5b) | Local policy checks |
| 8/8 invoice edits | Mutations of the original invoice_06 source | Local compiler/mandate replay |
| 21 quote checks | Passing checks in the archived allowed acceptance receipt | NEAR mainnet, not 21 attack refusals |
| 5/5 links | Source, compiler, Guard, payment and recorded-outcome archive consistency | Offline verifier, not fresh RPC verification |

The [recorded reproduction run](audit-artifacts/judge-reproduction-20261113/verification.json)
includes command results and hashes of the tested verifier/runtime files. Historical
CI and commit references establish earlier evidence provenance; the single pinned
snapshot above governs this reproduction. Neither reference is a final submission receipt.

## Replay time and evidence boundaries

The invoice verifier reconstructs its compiler clock as the archived policy's
`deadline_ms` minus the source's 74-hour timeout. This makes replay deterministic;
it is **not independent evidence of when the original compilation happened**.
The JSON field `compiler.compiled_at_utc` is this reconstructed time. Its local
`dry=True` request builds policy terms only; the mainnet acceptance claim comes
from the separately archived signed live quote/Guard receipt, not that request.

The offline verifier checks archived internal consistency and executed transfer
records. It does not re-query mainnet. For current chain state use the separate
read-only observer, recording its time and any RPC failures. The agent-reported
outcome remains unauthenticated by Guard even when its recorded floor is met.

## Demo and supporting material

| Looking for | Go here | Evidence level |
| --- | --- | --- |
| Three-minute story and exact narration | [Walkthrough](docs/hackathon/VIDEO_WALKTHROUGH.md) | Recording plan; not a published video |
| Public video and public X post | [Submission links/status](submission/submission-copy.md#public-links) | URLs still pending owner publication; no published media claimed |
| Testnet attacker refusal and allowed control | [Guard testnet evidence](docs/hackathon/GUARD_TESTNET_EVIDENCE.md) | Historical NEAR testnet |
| Local policy scenarios and red-team commands | [Judge guide](docs/hackathon/JUDGE_GUIDE.md) | Local tests, with testnet evidence separately labeled |
| Language and compiler role | [X3 language](docs/hackathon/X3_LANGUAGE.md) | Compile-time versus Guard/runtime checks |
| Threats and trust boundaries | [Threat model](docs/security/X3_SHIELD_THREAT_MODEL.md) | Security scope and limitations |
| What was built during the hackathon | [Prior work disclosure](PRIOR_WORK.md) | Compiler/VM predate hackathon; preserved provenance |
| Current agent-law implementation status | [Agent law](docs/hackathon/AGENT_LAW.md) | Separate status; not proof of autonomous mainnet custody |
| Submission wording | [Submission copy](submission/submission-copy.md) | Prepared copy; not a submission receipt |

Use the exact commit you review: `git rev-parse HEAD`. Historical CI links in
evidence establish those historical runs; current checks are visible in
[GitHub Actions](https://github.com/Cyptopimpinainteazy/x3lang/actions).
