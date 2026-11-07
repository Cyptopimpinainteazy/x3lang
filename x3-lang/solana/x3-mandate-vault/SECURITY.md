# x3-mandate-vault — Dependency Security Notes

This crate is a standalone, owner-controlled Solana SPL vault. It intentionally
keeps its own `Cargo.lock` and its own Solana dependency pin so it can be built
and audited independently of the Rust VM workspace.

## On-chain surface vs. dev/test tooling

The deployed SBF program is built only from the normal (non-dev) dependency
graph:

- `borsh 1.5`
- `solana-program 2.3`
- `solana-system-interface 1.0`
- `solana-sdk-ids 2.2`
- `spl-token 7.0`

`solana-program-test 2.3` and `solana-sdk 2.3` are **dev-dependencies** used only
for local unit/integration tests. They pull in the banks server, runtime,
client, and logger crates, which are never compiled into the on-chain program.

## Open GitHub dependabot advisories

Ten advisories currently reference this crate's `Cargo.lock`. Nine of them are
**dev/test-only** — they are reachable exclusively through
`solana-program-test 2.3.13` / `solana-sdk 2.3.1` and therefore never ship in
the deployed program:

| GHSA | Package | Why it does not ship on-chain |
| --- | --- | --- |
| `GHSA-w44r-93j7-qv28` | rustls 0.21 | TLS client in test harness only |
| `GHSA-c8m5-c952-6cm4` | rustls 0.21 | TLS client in test harness only |
| `GHSA-h2v8-394m-p7vv` | rustls 0.21 | TLS client in test harness only |
| `GHSA-82j2-j2ch-gfr8` | rustls-webpki 0.101 | TLS cert validation in test harness only |
| `GHSA-xgp8-3hg3-c2mh` | rustls-webpki 0.101 | TLS cert validation in test harness only |
| `GHSA-965h-392x-2mh5` | rustls-webpki 0.101 | TLS cert validation in test harness only |
| `GHSA-x4gp-pqpj-f43q` | curve25519-dalek 3.2 | test-harness keypair code only |
| `GHSA-w5vr-6qhr-36cc` | ed25519-dalek 1.0 | test-harness keypair code only |
| `GHSA-g98v-hv3f-hcfr` | atty 0.2 | `env_logger 0.9` via `solana-logger`; test-only |

One advisory is in the normal graph but is not reachable:

| GHSA | Package | Assessment |
| --- | --- | --- |
| `GHSA-cq8v-f236-94qc` | rand 0.7 | Present only via `solana-program → solana-secp256k1-recover → libsecp256k1 0.6 → rand 0.7`. The advisory requires the `log` + `thread_rng` features, a custom logger that calls `rand::rng()`, and a specific reseed-under-logging condition. This vault defines no logger and never calls any `rand` API; the secp256k1 recovery path does not generate keys at runtime. |

## Why these are not fixed by dependency upgrades

- `solana-program` is pinned to `"2.3"`; the newest released 2.x is `2.3.0`.
- `solana-sdk` `"2.3"` resolves to `2.3.1`, the newest 2.x.
- `solana-program-test` `"2.3"` resolves to `2.3.13`, the newest 2.x (already
  locked).
- `cargo update --dry-run` in this crate resolves only six unrelated micro
  bumps (`brotli-decompressor`, `cc`, `flate2`, `smallvec`, `toml_datetime`,
  `toml_edit`); none of the six advisories' packages have a compatible update
  inside the supported Solana 2.3 line.
- The patched versions (rustls 0.23, rustls-webpki 0.103, ed25519-dalek 2.0,
  curve25519-dalek 4.1, rand 0.8.6) belong to the Solana 5.x ecosystem.
  Migrating this standalone owner-controlled vault to Solana 5.x is a breaking
  API/ABI change, not a patch.
- Forcing those versions with `[patch]` or by hand-editing the lockfile would
  silently change Solana signing/keypair/ABI behavior and is unsafe for a
  custody-adjacent vault. `ed25519-dalek 2.x` is a breaking API change,
  `rustls 0.23` replaces the entire TLS stack, and `atty` has no maintained
  replacement inside `env_logger 0.9`.

## Resolution

These advisories are dismissed in GitHub as `tolerable_risk` with the rationale
above. They should be reopened and re-evaluated if the vault migrates to
Solana 5.x or if a future Solana 2.3.x patch releases updated transitive
dependencies.
