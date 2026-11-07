# Private cross-repository X3Lang proof mirror

**Scope:** CI credentials for the read-only `Cyptopimpinainteazy/xxxstar` peer
checkout from private `Cyptopimpinainteazy/x3lang` pull-request workflows.
This does not approve a feature, admit a branch, authorize production
settlement, or weaken the byte-for-byte mirror policy.

## Why a dedicated credential is mandatory

GitHub's standard `GITHUB_TOKEN` is scoped to the repository running the
workflow. It cannot read a second private repository by default. A checkout
failure must **fail closed**, not turn an inaccessible peer into a pass. The
job still checks the matching feature branch and runs the existing
`check_x3lang_mirror.py` with `origin/master` bases after checkout.

## Owner setup: least privilege

1. While signed into the GitHub owner account, create a **fine-grained personal
   access token** in Settings → Developer settings → Personal access tokens →
   Fine-grained tokens. Select **only**
   `Cyptopimpinainteazy/xxxstar` under repository access.
2. Grant only **Contents: Read-only** (Metadata read is automatic). Do not
   grant write, admin, workflows, Actions, or organization-wide access. Prefer
   a short expiration with an explicit renewal plan. A GitHub App installation
   token with the same restricted read scope is also acceptable.
3. In the **private X3Lang repository**, open Settings → Secrets and variables
   → Actions → New repository secret. Name it exactly
   `X3LANG_PEER_READ_TOKEN` and paste the credential **there**, not in code,
   local logs, an issue, an environment file, or a chat message.
4. Check that each feature branch needing this gate has a **matching peer
   feature branch** in `xxxstar`, intentionally published and reviewed under
   the separate mirror-branch policy. A secret alone cannot fix a missing
   branch. `xxxstar` remains read-only to this workflow.
5. Re-run the **same exact-head** X3Lang proof workflow. The
   `Verify identical proof code in both GitHub repos` job must show a
   successful checkout and `PROOF MIRROR MATCHED ...` on the intended files.
   Check review and other gates separately before merging.
6. Rotate or revoke the credential when it is no longer required. When the
   repo becomes public, remove the cross-repo credential only via a **reviewed
   workflow change**; do not silently fall back to anonymous checkout.

## Fail-closed behavior

- Missing secret: the earlier `Require private peer read credential` step
  exits nonzero before the peer checkout.
- Expired or improperly scoped token: the real private checkout fails.
- Missing peer branch: checkout fails. No automatic fallback to `master`.
- Mismatched mirrored bytes: the existing checker fails.
- Pull requests from external forks: repository secrets may be unavailable;
  these jobs intentionally fail rather than using `pull_request_target`.
- The token is scoped to the credential gate and checkout step, is not
  persisted in Git config, and is not passed to the compiler/VM proof job.
- `xxxstar` is never modified by this workflow.

To test the preflight without credentials or network calls:

```bash
python3 -m unittest discover -s tests/ci -p test_private_peer_checkout.py -v
```

The tests validate **presence checks and workflow wiring**, not a real GitHub
authentication handshake. Green local unit tests do not establish successful
private checkout. Never replace the real checkout with a mock or assume
the token is valid merely because it is nonempty.
