# Local scoped-agent storage upgrade verification

All execution here is local: unit tests or an ephemeral NEAR Sandbox. The old
Guard WASM was obtained through a mainnet read-only RPC and checked against
`GRy6z84ucCR9WMVXyepibVDC34TyKwjWtEi9rteKJtny`; that is not a mainnet upgrade.
No public transaction, token deposit, or cross-chain swap was performed.

`verification.json` records runtime source hashes, toolchain, proposed binary
hash, commands, results and limits. Logs retain command output. An independent
read-only reviewer found no blocking code findings and separately ran seven
storage tests. CI results belong to the exact draft-PR head and must be checked
on GitHub; these local logs do not establish CI success.

The final normalized WASM passed an actual old-binary → new-binary Sandbox
upgrade, preserving an existing intent and mandate without reinitialization.
The scoped signer registered at zero deposit. Unfunded/exhausted writes rolled
back, policy refusal preserved credit, and withdrawal completed via callback.
Failed-transfer callback recovery is covered by a mocked promise result, and
accepted-quote storage-exhaustion/replay rollback remains untested in Sandbox.

See [the API and reproducible build command](../../docs/security/GUARD_SCOPED_STORAGE.md).
Mainnet deployment remains a separate exact-transaction approval gate.
