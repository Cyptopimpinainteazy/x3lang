# Stage 2 funding cutoff and read-only nonfunding evidence

## Implemented prototype

The existing experimental NEAR escrow now stores a registrar-controlled, immutable `FundingWindow` for each 32-byte swap ID. A registrar must pre-register the expected LP account, destination recipient, 32-byte hashlock, exact yoctoNEAR amount, last funding time and NEAR escrow expiry. The LP can then call `fund_window` only with the exact amount **before** the deadline. After the deadline anyone can call `close_unfunded`, which changes the on-chain window irreversibly from `Pending` to `NotFunded` and prevents all future funding of that swap ID. Already-funded windows cannot be marked `NotFunded` and the original `lock()` method cannot bypass a registered swap ID. `get_window(id)` is a read-only method. Existing JSON escrow amounts and new funding window amounts are serialized as exact decimal strings, including `u128` values above JavaScript's safe integer range.

The separate testnet-only BSC `AttestedBnbEscrow` now recognizes a third EIP-712 outcome (`NotFunded`), distinct from `Claimed` and `Refunded`. It requires two valid committee signatures and BSC expiry to release a corresponding BNB depositor refund. A claim certificate cannot be reused as a nonfunding certificate. **This contract authenticates signatures, not NEAR state.**

A separate Python `observe_window` function requests `get_window(id)` at RPC `finality: final` from distinct HTTPS hosts and demands matching on-chain terminal state, expected LP/recipient/hashlock/amount/deadline/expiry and exact block. It returns `authorized_to_sign=False`; no keys, signatures or transfer actions are implemented. Missing accounts, RPC errors and null responses are never proof of nonfunding.

## Critical remaining requirements (Stage 2 is NOT complete)

- **Authenticated NEAR consensus and storage evidence:** equal RPC response strings are not independent finality proofs. A corrupt pair of endpoints can fabricate both. A production signer must validate genuine finalized NEAR contract execution and immutable state, including pinned deployed bytecode hash, chain identity and storage/receipt proofs, independently of those endpoints.
- **Trusted registrar:** registration is restricted to the deploying actor, whose storage is funded by the operator. Governance, storage economics, full operational signing, deployment identities, and malicious-registrar threat model are unreviewed. No public registration is enabled.
- **Cross-chain binding:** production certificates must bind NEAR registrar, actual vault, funding window, BSC escrow and exact swap terms with independently verified chain IDs; two-of-three attesters can still lie or censor.
- **No storage migration:** the prototype changes `near_sdk` serialized contract state (new registrar and window map) and JSON amount encoding. It is for a fresh testnet deployment only. Existing deployed contract state would require a reviewed migration.
- **No durable distributed consensus:** SQLite watch leases cannot enforce committee anti-equivocation. No verified signer network, key management, malicious committee test, external attestation witness, authenticated proof transport, funded BSC/NEAR swap, or safe signer-independent rescue exists.
- **No production relayer:** the watcher only schedules/retries and records `REVIEW_REQUIRED` states. It cannot broadcast, execute, sign, settle or prove finality. An unavailable committee can still lock BNB indefinitely.

**Release gate: NO_MAINNET_FUNDS.** No deployment or real transfers authorized, no claim of atomic/permissionless settlement. Manual security review, exact-head CI, and two-chain funded testnet evidence remain required.
