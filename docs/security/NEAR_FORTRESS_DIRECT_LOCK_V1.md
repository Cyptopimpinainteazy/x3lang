# Fortress: native NEAR direct lock v1

Status: testnet-only candidate; no deployment, no authenticated cross-chain finality claim.

New direct lock callers should use `authenticated_lock_deposit_required` and
`lock_authenticated(recipient, hashlock, expires_at_ns, amount, salt)`. The
latter derives the key from the authenticated NEAR predecessor, never from a
caller-selected identifier. Hashlock and salt are exactly 32 bytes. A unique
salt permits multiple otherwise identical locks. Attached NEAR covers both the
principal and the prepaid storage record and payout entries.

The returned identifier is the ASCII prefix `x3-direct-v1:` followed by
lowercase hex SHA-256 of this byte sequence:

    X3-NEAR-DIRECT-LOCK-V1
    || lp(vault_account) || lp(predecessor_account) || lp(recipient_account)
    || hashlock[32] || amount_yocto_near:u128_be || expiry_ns:u64_be || salt[32]

Here lp(s) is u32 big-endian byte length followed by the UTF-8 account ID.
The namespace is disjoint from legacy 64-hex direct IDs and registered
NEAR/BSC window IDs. The contract's existing `get`, `claim` and `refund`
methods accept the returned identifier. The longer key is included in the
storage quote and charge. The existing window commitment format is unchanged.

The persisted swap state is authoritative. Internally a non-Copy, non-Clone
`Locked<NativeNear>` transitions once to `Spendable<NativeNear>` for its
bound claimant or refund owner. Claim requires the designated recipient,
matching preimage and time strictly before expiry; refund is permissionless
at or after expiry but credits only the original initiator. Neither can
release a terminal record again. Typed credits enter the existing checked
credit ledger; withdrawals remove credit into in-flight storage and failed
callbacks restore it. Successful callbacks reduce total liability. The
transient Rust types are not serialized bearer assets or X3 source syntax.
Existing Borsh state/record layouts are unchanged.

The liability invariant is locked principal plus owner credits plus pending
withdrawals; storage stake is separate. A terminal record remains as a
prepaid anti-replay tombstone. Tests reconstruct the contract from the
persisted root and maps and refuse replay. The native SDK test environment
is not a live network or a cryptographic cross-chain verifier.

Compatibility: legacy lock() remains callable for historical testnet clients
and new legacy locks require lowercase hexadecimal. Existing IDs are still
readable/settleable by exact key. Legacy caller-selected IDs can still grief
unregistered NEAR/BSC windows, as documented by the existing squatting test;
they cannot produce authenticated-window settlement logs. Retiring that
entry point and migrating clients/state requires a separate approved release.
The new ID is scoped to the vault's chain environment, not a global
genesis-bound cross-chain proof. Issue #56 remains a release blocker.
