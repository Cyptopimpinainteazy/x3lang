# X3 Force Field — NEAR Hackathon submission checklist

## Competition framing

**.x3 is an owner-controlled, typed intent language for AI-agent payments, combining compiler checks with Guard enforcement of signed quote terms.**

An intent states the desired outcome. The owner’s `.x3` mandate sets the limits.
The compiler rejects invalid programs; deterministic mandate checks and NEAR Guard
refuse prohibited terms. NEAR Intents / 1Click supplies the supported execution route.
Read [the stage-by-stage explanation](X3_LANGUAGE.md#x3-intents-with-compiler-checks-and-guard-enforcement)
for compile-time versus runtime checks and the evidence boundaries.


Official instructions were checked against
[`https://nearhackathon.com/HACKATHON.md`](https://nearhackathon.com/HACKATHON.md) on
2026-11-04. Re-open that live file immediately before submitting because rules may change.

## Organizer facts

- Development and submission close: **2026-11-14T23:59:00Z**.
- The repository must be public by the deadline.
- The human must publish a public demo post on X saying the project was built at NEAR HACKATHON,
  tagging `@NEARProtocol` and `#NEARHackathon`, with video or screenshots attached.
- Submit once through [the application form](https://nearhackathon.com/#apply) or the documented
  `POST https://nearhackathon.com/api/applications` API.
- Errors are changed through
  [`/request-changes`](https://nearhackathon.com/request-changes), not by creating a second
  submission.

## Project submission scope

**Public-facing name:** X3 Force Field (formerly X3 Shield). Preserve historic X3 Shield evidence names, deployed `x3-guard` contract and `.x3` hash identities. This rename makes no protocol or on-chain change.

**Differentiator to show judges:** typed owner-written `.x3` mandate → compile/type-check and reject unsafe edits → deterministic quote/policy checks → NEAR Guard signed-quote enforcement → archived `invoice_06` authorization, settlement and recorded outcome. Do not imply native X3VM executed on NEAR.


Owner-selected tracks: AI Security & Defense (`security`, primary), Agent Economy (`agents`), Autonomous Business (`business`) and Unstoppable DeFi (`defi`). Selection is confirmed by the owner; this checklist does not certify organizer acceptance or a saved submission receipt.

Historical Tidepool testnet delivery and the official mainnet invoice_06 settlement remain separate evidence. See [DeFi evidence and gaps](DEFI_TRACK_EVIDENCE_AND_GAPS.md).

## Repository

- [x] Release consolidation PR #83 was merged to the canonical `master` branch.
- [ ] Confirm the **exact submission commit SHA** is still reachable from `master` (do not submit a stale release branch).
- [ ] Confirm no unreviewed changes are required for the submitted demo; keep non-production settlement gates closed.
- [ ] `PRIOR_WORK.md` and the baseline comparison remain intact.
- [ ] No API keys, key files, seed phrases, `.env` files or private receipts are tracked.
- [ ] No fabricated deployment, quote, transaction or settlement evidence is present.
- [ ] Root and hackathon documentation describe the same verified capability.
- [ ] The repository is public before the organizer deadline.

## Reproduce

- [ ] Create and activate a project-local Python virtual environment.
- [ ] Use Linux x86_64, Python 3.12, Node 22+ and the Rust toolchain; the judge
      wrapper provisions its isolated nine-package lock. If a local Python 3.12
      was installed using uv but is not on PATH, the launcher discovers it
      offline; use uv python install 3.12 before running on an unprepared host.
      Native macOS execution has not been validated; do not claim it works.
- [ ] Run `bash scripts/hackathon/judge_one_command.sh /tmp/x3-shield-judge`.
- [ ] Run `python3 scripts/judge_evidence.py --mode full`.
- [ ] Confirm the readiness report names the current clean commit.
- [ ] Confirm the demo prints authorized, approval-required, wrong-recipient and
      prompt-injection refusal decisions.
- [ ] Confirm the local demo reports `local_policy_preview_only`; keep the
      separately labeled NEAR testnet evidence at its own limited level.
- [ ] Re-verify both commands in `GUARD_TESTNET_EVIDENCE.md`; distinguish
      official dry-quote refusal and live allowed acceptance from an unproven live
      forbidden-quote refusal and funded official 1Click settlement.
- [ ] Reproduce the separately labeled historical Tidepool archive using
      `verify_first_swap_archive.py`; retain the exact commit and result. Keep
      archived observations, any fresh RPC recheck and trusted-relay assumptions
      distinct; neither local policy nor quote acceptance proves delivery.
- [ ] Run GitHub CI on the exact pushed SHA.
- [ ] Record every check result without treating a neutral or skipped check as a pass.

## Browser demo

- [ ] Run `python3 -m http.server 8000 --directory x3-lang/near/demo`.
- [ ] Open `http://localhost:8000/judge.html` on localhost so WebCrypto is available.
- [ ] Confirm all twelve scenarios render and Python/browser decisions agree.
- [ ] Confirm the local judge preview says its scenarios do not fetch a fresh
      quote, authorize a wallet, send a transaction or settle funds. Keep any
      linked archived quote, Guard transaction and Tidepool delivery evidence
      separately labeled.
- [ ] If a hosted page is used, verify it serves this exact commit and keep it available through
      judging.

## Video and X post

- [ ] Use [`VIDEO_WALKTHROUGH.md`](VIDEO_WALKTHROUGH.md) and keep the recording under three
      minutes.
- [ ] Show the local one-command judge path after its documented dependency
      provisioning and show the exact decision labels.
- [ ] Use the exact transactions and code hashes for each evidence path. State
      that the official 1Click positive control in `GUARD_TESTNET_EVIDENCE.md`
      stopped before funding. If showing the separate historical Tidepool trade,
      link `FIRST_SWAP_JUDGE_WALKTHROUGH.md`, show the funded source and finalized
      destination, and disclose the trusted relay. Do not claim official 1Click
      settlement or cryptographic cross-chain atomicity from that trade.
- [ ] State the evidence boundary plainly.
- [ ] The human publishes the X post from the project/team account.
- [ ] The post names NEAR HACKATHON, `@NEARProtocol` and `#NEARHackathon`.
- [ ] The post contains the video or screenshots and public repository link.

## Owner-only submission fields

Collect these from the human; do not infer or invent them:

- project name;
- lead name;
- contact email;
- team size;
- tracks;
- mainnet NEAR payout account;
- public repository URL;
- public X demo-post URL;
- 20–2,000 character description;
- explicit consent to the organizer contact notice.

Generate one lowercase UUID as `requestId` and reuse it for retries. Print the complete JSON
payload for the human and wait for explicit approval before sending anything.

## Submission gate

- [ ] Official rules re-opened immediately before submission.
- [ ] `PYTHONPATH=x3-lang python3 -m pytest x3-lang/tests/test_submission_copy.py -q` passes (description LF/UTF-16/CRLF, all three tracks, X post lengths and tags).
- [ ] Final exact-commit tag `hackathon-submission` is created once and never moved; preserve `pre-hackathon`/`baseline` provenance.
- [ ] Hosted demo remains available through judging (at least November 18, ideally November 25 per organizer guidance).
- [ ] Human confirmed the exact JSON payload.
- [ ] Human confirmed the public repository and X post links.
- [ ] No secret or personal browser state appears in screenshots.
- [ ] Submission sent only once and the response recorded.

If the submission already exists, do not submit again. Use `/request-changes`.
