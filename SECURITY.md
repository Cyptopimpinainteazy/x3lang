# Security Policy

X3Lang is experimental research and hackathon-stage software. **There is no production release, mainnet approval, custody guarantee, or public bug bounty promised by this repository.**

## Report a vulnerability

Do **not** publish working exploits, private keys, seed phrases, credentials, wallet signatures, or private chain evidence in public issues. If GitHub's private vulnerability-reporting feature is available on the repository's **Security** tab, use it. Otherwise, contact the maintainer privately through the contact methods listed on the [GitHub profile](https://github.com/Cyptopimpinainteazy). Do not assume a public issue is confidential.

A useful report includes: affected commit SHA, impacted files, the trust boundary crossed, a minimal non-destructive reproducer, expected versus actual behavior, and any safety-critical consequences. Redact secrets and personal details. There is no guaranteed response time.

## Security-sensitive boundaries

- **Agent instructions are untrusted.** A model's generated `.x3` program must pass deterministic parsing, type checks, financial guards and human-approved policy.
- **Local preflight is not wallet authority.** Neither `track_control.py` nor a successful contract `check_quote` view independently prevents origin-chain wallet spending.
- **Origin-chain keys are outside X3 Guard.** An owner-controlled wallet, contract or signing policy must independently enforce spending restrictions. A Guard acceptance by itself is not permission to send funds.
- **Signed 1Click quotes are not settlement.** A valid signature authenticates provider terms, not the outcome of a transfer.
- **Chain observations are not necessarily causality.** Independently read both sides before attributing a cross-chain payment.
- **Upgradeable contracts remain subject to deployment-key authority.** No source-only test establishes immutable deployed code.
- **Approval and replay boundaries matter.** Test agent identity, owner binding, version changes, revocation, nonce uniqueness, expiry, fee limits and refund destinations.

Do not fund any address from example files; placeholder addresses and test keys are not treasury destinations.

## Minimum bar for changes

For security-relevant changes, provide targeted negative-case tests, the exact commands and commit SHA tested, and a clear distinction between local, CI, signed-quote and on-chain evidence. Never relax a release gate, skip a failing test, or present a mock response as live transaction proof.

See the [main README](README.md) and [contributor guide](CONTRIBUTING.md) for reproducible checks. The parent X3 chain has a separate [security policy](https://github.com/Cyptopimpinainteazy/xxxstar/blob/master/SECURITY.md).
