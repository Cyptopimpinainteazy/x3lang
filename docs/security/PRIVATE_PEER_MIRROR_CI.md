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

1. The GitHub owner generates a **dedicated Ed25519 SSH deploy keypair** for the
   private `Cyptopimpinainteazy/xxxstar` repository. The private key remains
   owner-only outside both repositories and is used only as an Actions secret.
2. Add the **public** key to `xxxstar` Settings → Deploy keys → Add deploy key,
   with **Allow write access OFF**. This key has access to `xxxstar` only.
3. In the private X3Lang repository, Settings → Secrets and variables → Actions,
   create `X3LANG_PEER_READ_SSH_KEY` with the complete private deploy key.
   Never print, commit, or paste the private key into chat. The matching
   checkout step references it as `ssh-key:` and never persists credentials.
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
- Expired or improperly scoped deploy key: the real private checkout fails.
- Missing peer branch: checkout fails. No automatic fallback to `master`.
- Mismatched mirrored bytes: the existing checker fails.
- Pull requests from external forks: repository secrets may be unavailable;
  these jobs intentionally fail rather than using `pull_request_target`.
- The deploy key is scoped to the credential gate and checkout step, is not
  persisted in Git config, and is not passed to the compiler/VM proof job.
- `xxxstar` is never modified by this workflow.

To test the preflight without credentials or network calls:

```bash
python3 -m unittest discover -s tests/ci -p test_private_peer_checkout.py -v
```

The tests validate **presence checks and workflow wiring**, not a real GitHub
authentication handshake. Green local unit tests do not establish successful
private checkout. Never replace the real checkout with a mock or assume
the deploy key is valid merely because it is nonempty.
