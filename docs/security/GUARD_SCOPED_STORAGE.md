# Guard storage for restricted agent keys

This change is prepared for review. It is not a mainnet deployment or swap proof.

NEAR function-call keys cannot attach deposits. Previously every successful
agent write that grew storage required a deposit, so the deployed Guard could
not support the restricted-key mainnet workflow. The upgrade permits the same
three agent mutation methods to consume bounded storage credit prepaid by the
mandate owner. Quote, approval, policy, replay and spend checks are unchanged.

## Public API

| Method | Caller and arguments | Deposit and result |
| --- | --- | --- |
| `fund_agent_storage` | Owner; `{"agent":"agent.near"}`; requires that owner's active, unexpired mandate | Positive deposit, at most 1 NEAR. First funding pays the credit record's storage; the rest becomes credit. Returns decimal-string yoctoNEAR balance. Total unspent credit cannot exceed 1 NEAR. |
| `get_agent_storage_balance` | View; `{"owner":"owner.near","agent":"agent.near"}` | Returns decimal-string yoctoNEAR, zero if never funded. Fails `X3_STORAGE_WITHDRAWAL_PENDING` during withdrawal. |
| `withdraw_agent_storage` | Owner; `{"agent":"agent.near"}`; works after expiry/revocation | Zero deposit. Returns a promise transferring all unused credit to the owner. Its callback clears credit on success or restores it on failure. |
| `resolve_agent_storage_withdrawal` | Private receipt callback only | Reserved credit cannot be spent, topped up or withdrawn again while the transfer is pending. |

Zero-deposit `agent_register_intent`, `agent_accept_quote` and
`agent_record_outcome` charge net positive storage growth at the protocol's
storage-byte cost. Insufficient credit fails `X3_STORAGE_BUDGET_EXHAUSTED` and
the runtime rolls back the receipt. Existing calls with attached deposits
continue to pay their own storage and refund excess; they do not debit credit.
They may execute while a withdrawal is pending because they pay independently.
Approvals and mandate creation remain caller-funded owner operations.

Credit is isolated by owner and agent, persists across mandate replacement, and
is not replenished by refunds or storage shrinkage. An authorized agent can
consume its finite budget by registering many otherwise valid intents; the
payment cap only counts accepted quotes. Owners choose whether to top up again.
The fixed credit record remains allocated after withdrawal; its storage charge
is not returned. Gas comes from the agent account/key allowance separately.

## Explicit client storage mode

After reviewing the deployed Guard's prepaid-storage support and funding the
owner/agent storage credit, select `--prepaid-agent-storage` on
`near_intents.py prepare`, `near_intents.py guard-args`,
`near_agent.py register-args`, or `near_agent.py pay`. The generated JSON and
commands use `0 NEAR` for `agent_register_intent`, `agent_accept_quote`, and
`agent_record_outcome`. Live agent reports preserve the option in their next
commands. For the guided script, export `X3_AGENT_PREPAID_STORAGE=1` for each
`agent-demo.sh` invocation; its default is `0`, and other values are rejected.

This option does not verify deployed code, available credit, or mandate authorization,
and it never funds or raises a budget. Generated agent sends require the pinned-key
boundary described below.
Insufficient credit still fails on chain. The default retains legacy attached
deposits. Owner registration, quote, outcome, approval, mandate and revoke
commands retain their existing deposits even when the option is selected.
The existing settlement evidence gates and owner confirmations still apply.

## Pinned delegated signing key

For prepaid agent commands, also provide `--agent-public-key` and
`--agent-max-allowance-yocto`. The public key and positive allowance ceiling are
reviewed operator configuration; the model and receipt do not choose them. In
`agent-demo.sh`, export `X3_AGENT_PUBLIC_KEY` and
`X3_AGENT_MAX_ALLOWANCE_YOCTO` alongside `X3_AGENT_PREPAID_STORAGE=1`.
The allowance ceiling is a refusal threshold, not funding or a budget increase.

Generated prepaid commands invoke `near_agent_key.py send`, which reads one
regular, owner-readable-only credential file, derives its Ed25519 public key
using the existing `cryptography` dependency, and matches its account, declared
public key and embedded public-key suffix to the reviewed pin. Immediately
before each send it queries the selected network's fixed RPC for its chain ID
and the exact key at `final` finality. It refuses FullAccess, unlimited or zero
allowance, allowance above the reviewed ceiling, the wrong Guard receiver,
empty/wildcard method lists, any method outside the three agent mutations, and
a requested method absent from that list. RPC failures fail closed.

The signing boundary defaults to 30 Tgas. An explicit operator-reviewed
`--gas-tgas` integer from 1 through 30 selects the exact prepaid gas encoded in
both the CLI command and signed-artifact verification; it is never automatically
lowered, retried or selected from model/receipt data. A changed transaction gas
setting requires the owner's approval of that exact transaction.

Before signing, it reads protocol fees and gas price at the checked key's finalized
block. For this single zero-deposit Ed25519 FunctionCall, the allowance admission
cost is send gas at the current price plus prepaid and receipt execution gas at
`max(current_gas_price, min_gas_purchase_price)`. Missing or malformed cost data
and insufficient allowance fail closed. This upfront gas purchase is distinct
from the ultimately burnt network fee: unused gas and gas-price differences may
be refunded. The check neither raises the key allowance nor proves that the
selected gas is sufficient for contract execution. The formula follows
[nearcore 2.14 transaction cost calculation](https://github.com/near/nearcore/blob/2.14.0/runtime/runtime/src/config.rs#L449-L513).

Only a validated snapshot is passed to the existing NEAR CLI, using a temporary
0600 file in a private directory, zero attached deposit, and the queried nonce
plus one, block hash and block height. Signing uses `--offline ... display`:
[near-cli-rs 0.30.1 ignores those nonce/block flags in online signing](https://github.com/near/near-cli-rs/blob/v0.30.1/src/transaction_signature_options/sign_with_access_key_file/mod.rs#L60-L85).
Arguments are serialized once and passed as `base64-args` so CLI JSON normalization
cannot change the reviewed bytes. Captured signed bytes must match the exact V0
transaction, signer/public key, receiver, nonce, block hash and single FunctionCall
method/arguments/gas/deposit; its Ed25519 signature must verify against the pin.
Unknown formats or mismatches are refused before broadcast.

Verification older than 60 seconds is refused both before signing and before
broadcast. After printing only the deterministic transaction-hash candidate, the
wrapper makes one `send_tx` request to the same fixed network RPC, with no retry.
It accepts a successful response only with explicit success status, `FINAL` and
matching transaction identity in NEAR's flattened outcome schema. Independent
finalized transaction verification is still required. Offline CLI failures cannot
broadcast; RPC timeout, malformed response or failure remains an unknown outcome
with the hash available for reconciliation. The snapshot is removed on normal success or
failure.
An abrupt process kill can leave a temporary file, so this remains an operator
credential-handling process. No private credential is included in command lines
or returned diagnostics. Raw CLI output is suppressed because it can include the
private quote and receipt. Only stage/exit metadata, the computed transaction-hash
candidate and fixed recognized error-code candidates are emitted. These are
explicitly unverified; a zero return code is not independent finality or settlement proof.

The `check` command needs only public configuration and never opens a key file
or signs. For example, replace the placeholders with reviewed public values:

```bash
python3 x3-lang/near_agent_key.py check --account AGENT_ACCOUNT \
  --public-key REVIEWED_PUBLIC_KEY --contract GUARD_ACCOUNT \
  --method agent_register_intent --max-allowance-yocto REVIEWED_CEILING
```

This is a trusted operator harness: the model proposes `.x3` source and the owner
reviews and confirms actions. An owner recovery FullAccess key may remain on the
account and host; the checker admits only the specifically supplied delegated
key. File modes and snapshots are not OS isolation or protection against host
compromise. Never delete the owner's last recovery key for this workflow. The
RPC observations are a preflight check, not a cryptographic state proof; NEAR
still enforces the actual signing key's permission when executing the transaction.
Legacy non-prepaid commands remain direct operator commands and make no delegated
key claim. This change does not authorize a mainnet transaction.

## Upgrade compatibility

The `Contract`, `Mandate`, intent layouts and map prefixes are unchanged.
Credits use a new `X3_AGENT_STORAGE_V1:<owner>/<agent>` raw-storage namespace,
with a fixed 17-byte record (little-endian u128 balance plus pending flag).
No migration or reinitialization is required. Never call `new` on an existing
deployed account. Prior WASM cannot manage these new credits: before any rollback,
stop agents and withdraw all unused balances, waiting for callbacks to finish.
An owner-authorized code upgrade changes the pinned code hash; all signing
preflight must review and pin the new exact binary before any swap action.

Do not remove an agent's old key until the upgrade and storage path have been
verified and the new restricted key has been installed and checked. Each
mainnet deployment, funding, key addition/removal and Guard action requires its
own exact-transaction owner approval. No mainnet action is part of local tests.

## Verification and limits

Run `cargo test --locked` from `x3-lang/near/x3-guard`, and
`bash scripts/hackathon/scoped_agent_sandbox_smoke.sh` from the repository root.
The Sandbox exercises the compiled production WASM with a real function-call
signer, storage debit, mandate refusal, withdrawal and failed-registration
rollback. Unit tests cover acceptance/outcome charging, credit isolation/caps,
and failed-transfer callback restoration. Mocked promise outcomes do not prove
real failed-transfer execution. The Sandbox does not claim accepted-quote
storage-exhaustion/replay-ledger rollback or a cross-chain settlement.

Dependency reference: `near-sdk 5.30.0`, pinned in Cargo.lock. Its
[generated self-call and private-method documentation](https://docs.rs/near-sdk/5.30.0/near_sdk/attr.near.html)
defines the callback boundary. The version-matched SDK source documents
`env::promise_result_checked`; the callback reads at most zero result bytes
because a native transfer's successful result is empty.
The [NEAR access-key specification](https://near.github.io/nearcore/DataStructures/AccessKey.html)
describes restricted permissions. Guard authorizes quote terms; it does not
custody swap assets or prevent a foreign-chain wallet keyholder bypassing it.

## Reproduce the proposed binary

With Rust 1.99.1, its `rust-src` component, the WASM target and locked
Cargo dependencies installed, run from `x3-lang/near/x3-guard`:

```bash
RUSTFLAGS="--cfg near --remap-path-prefix=$(rustc --print sysroot)/lib/rustlib/src/rust=/rustc/58cf8f9c35b355199727627d07ae34a4e2846d67 --remap-path-prefix=${CARGO_HOME:-$HOME/.cargo}/registry/src=/home/lojak/.cargo/registry/src" \
  cargo build --locked --release --target wasm32-unknown-unknown
```

The expected binary is 572248 bytes, SHA-256
`9a614640ac8bc39bbbcb1674ce0f4fe2f51610ac7ba828c9c4574be92712a632`,
NEAR code hash `BPdpJAw8R4SDh6Wi7dPcXTs5T7ncJDVKCLgzuqZFb15T`.
The local-node CI workflow compares its normalized rebuild against the
[reviewed verification manifest](../../audit-artifacts/guard-scoped-storage-20261110/verification.json).
A later intentional Guard change must regenerate the reviewed artifact and
verification manifest; a mismatching binary must never be deployed under this
review packet. This proposed hash does not identify an existing mainnet deployment.
