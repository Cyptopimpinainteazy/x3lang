# Agent vault on NEAR testnet: runbook

This puts the [scoped-agent vault](X3_SHIELD_SCOPED_AGENT_VAULT.md) on public NEAR testnet and
records an observed testnet scenario using the agent's own key. RPC records are checked
separately from local Sandbox or CLI reports; this is not an audited custody guarantee. It answers the review finding that the deployed Guard holds no funds and a full
wallet key can go around it.

**Status:** not yet run. Nothing here is evidence until `evidence/agent-vault-testnet-*/` exists
and `vault_testnet_evidence.py verify` passes on it.

## What the observed-scenario verifier checks

| Claim | How it is shown | Checked by |
|---|---|---|
| The deployed code matches the supplied WASM | code hash on chain = SHA-256 of the supplied WASM bytes; source/build provenance is separate | `collect`, `verify` |
| The observed vault has no access keys | the vault account has **no access keys** at the pinned final block | `collect`, `verify` |
| The observed agent key is scoped at the pinned final block | its only key is FunctionCall, `<vault>::request_payment` | `collect`, `verify` |
| Allowed payments work | 0.1 and 0.2 NEAR paid to the vendor; the vault returns `true` | per-transaction |
| Over the cap is refused | 0.3 NEAR: `X3_VAULT_DENIED: per-payment cap exceeded` | per-transaction |
| Unapproved recipients are refused | stranger: `X3_VAULT_DENIED: recipient not allowlisted` | per-transaction |
| Replays are refused | reused nonce: `X3_VAULT_DENIED: nonce already used` | per-transaction |
| The total budget holds | 0.2 more after 0.3: `X3_VAULT_DENIED: cumulative budget exceeded` | per-transaction |
| Local reports of transfer and owner-method refusal | CLI reports are retained as local evidence; attempted execution or protocol refusal is not independently verified | CLI output, with separately queried key list and stranger balance |
| Even the owner can't drain a live mandate | recovery before revoke: `X3_VAULT_DENIED: revoke or expire mandate before recovery` | per-transaction |
| Revocation works | after revoke: `X3_VAULT_DENIED: owner revoked mandate` | per-transaction |
| The ledger matches the money | vault `spent` = 0.3, nothing reserved, refused nonces left no record; the vendor received exactly 0.3 and the stranger nothing | `collect`, `verify` |

Version 2 accepts only the complete 21-step bounded scenario with locking enabled and
19 distinct included transaction records. An unlocked run cannot receive this verified-scenario result. Missing, duplicated, reordered or substituted
steps/records, wrong actions or deposits, empty or different deployed code, and an unlocked
vault refuse collection or verification. Pre-attack balances and final queries must match
their recorded blocks. Recomputing an evidence hash does not replace these checks.

A refusal for the wrong reason, a key with wider scope, a stray yoctoNEAR at the stranger or any
code-hash mismatch fails the whole collection. The result describes verified RPC observations
for this testnet scenario; it does not authenticate consensus finality, establish general
custody/security guarantees, authorize execution, or supply an independent audit.

## What it does not prove

* **Testnet only.** The contract refuses to initialize on an account that doesn't end in `.testnet`.
* **Native NEAR, one recipient.** It does not pay NEAR Intents deposit addresses (they change per
  quote) and is not part of the mainnet swap.
* **The owner is trusted.** The owner account's full-access key is outside this vault.
* **Two reports have no included transaction hash.** Their CLI text is not independent chain proof
  of an attempt or refusal. The final key list and unchanged balance are separate observations.
* **Not audited.** The open items in [X3_SHIELD_SCOPED_AGENT_VAULT.md](X3_SHIELD_SCOPED_AGENT_VAULT.md)
  still apply: a fixed 0.5 NEAR callback buffer, a 64-payment lifetime cap that a compromised agent
  can use up, and no recovery if a callback never arrives. Here they can only cost test NEAR.

## Before you start

The [independent testing/review gates](X3_SHIELD_SCOPED_AGENT_VAULT.md) remain open.
Complete those gates and obtain explicit owner approval of the exact deployment,
signing and funding plan before executing these commands. Merging this runbook or
verifying evidence is not execution approval.

* near-cli-rs 0.30 or later (`near --version`), cargo-near, Rust from `rust-toolchain.toml`, Python 3.
* A testnet owner account (default `x3lang.testnet`) whose key is in `~/.near-credentials/testnet/`.
* About **6 testnet NEAR** on it; about 0.7 comes back at the end. The storage NEAR in a locked
  vault stays there.

## Run it

```bash
S=x3-lang/near/vault-testnet.sh
$S setup      # owner account, key file, lock yes/no (default yes)
$S check      # tools, the owner key is on chain, balance
$S build      # cargo near build; prints the WASM SHA-256
$S accounts   # vault-, agent-, vendor-, stranger-<tag>.<owner>        4 txs
$S deploy     # deploy + new() in ONE transaction, then a code-hash check  1 tx
$S lock       # code-hash check again, then delete the vault's only key  1 tx, irreversible
$S fund       # owner -> vault, 1 NEAR of escrow                         1 tx
$S scope      # agent gets a request_payment-only key, loses its full key 2 txs
$S attack     # 6 vault calls + 2 RPC refusals                           6 txs
$S revoke     # recover while live (refused), revoke, agent refused, recover  4 txs
$S evidence   # read-only: collect + verify -> evidence/agent-vault-testnet-<tag>/
$S show       # every transaction with its explorer link
```

Every signing step prints what it sends and waits for `yes`. If a step stops halfway, re-run the
same command: steps already recorded are skipped. A request the vault should refuse but accepts
stops the run as a security finding. So does an RPC refusal that comes back with a transaction id.

Keys stay in `~/.near-credentials`. The script hands key-file paths to near-cli. The helper reads
only each file's `public_key`, to match it to the key on chain.

## Re-verify later

```bash
python3 scripts/hackathon/vault_testnet_evidence.py verify \
  --evidence evidence/agent-vault-testnet-<tag>/evidence.json \
  --wasm x3-lang/near/x3-agent-vault/target/near/x3_agent_vault_testnet.wasm
```

It re-reads every transaction, the key lists, the vault's views and both balances at the recorded
block. A non-archival RPC forgets old blocks after a few days, so pass `--rpc-url` with an archival
testnet RPC after that.
