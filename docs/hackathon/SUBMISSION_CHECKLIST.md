# NEAR Hackathon submission checklist

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
- Track ids for this project are `security` and `agents`. DeFi routing remains
  roadmap work until a separately evidenced deposit and settlement exist.

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
      wrapper provisions its isolated nine-package lock.
- [ ] Run `bash scripts/hackathon/judge_one_command.sh /tmp/x3-shield-judge`.
- [ ] Run `python3 scripts/judge_evidence.py --mode full`.
- [ ] Confirm the readiness report names the current clean commit.
- [ ] Confirm the demo prints authorized, approval-required, wrong-recipient and
      prompt-injection refusal decisions.
- [ ] Confirm the local demo reports `local_policy_preview_only`; keep the
      separately labeled NEAR testnet evidence at its own limited level.
- [ ] Re-verify both commands in `GUARD_TESTNET_EVIDENCE.md`; distinguish
      official dry-quote refusal and live allowed acceptance from a live
      forbidden-quote refusal, deposit, settlement and causality.
- [ ] Run GitHub CI on the exact pushed SHA.
- [ ] Record every check result without treating a neutral or skipped check as a pass.

## Browser demo

- [ ] Run `python3 -m http.server 8000 --directory x3-lang/near/demo`.
- [ ] Open `http://localhost:8000/judge.html` on localhost so WebCrypto is available.
- [ ] Confirm all twelve scenarios render and Python/browser decisions agree.
- [ ] Confirm the page says no quote, wallet authorization, transaction or settlement occurred.
- [ ] If a hosted page is used, verify it serves this exact commit and keep it available through
      judging.

## Video and X post

- [ ] Use [`VIDEO_WALKTHROUGH.md`](VIDEO_WALKTHROUGH.md) and keep the recording under three
      minutes.
- [ ] Show the local one-command judge path after its documented dependency
      provisioning and show the exact decision labels.
- [ ] Show only the real testnet transactions and code hashes in
      `GUARD_TESTNET_EVIDENCE.md`; state that no deposit or settlement occurred.
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
- [ ] Human confirmed the exact JSON payload.
- [ ] Human confirmed the public repository and X post links.
- [ ] No secret or personal browser state appears in screenshots.
- [ ] Submission sent only once and the response recorded.

If the submission already exists, do not submit again. Use `/request-changes`.
