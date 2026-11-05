# NEAR Chain Signatures authorization architecture

**Status: design only.** This branch does not request an MPC signature, create a Chain
Signatures account, deploy a signer contract, or send a funded transaction.

## Why a separate authorization gate is required

A BSC transaction cannot be authorized by a 1Click quote alone. The quote signs swap terms; it
does not prove that the owner approved the X3 program, and it does not authorize an MPC key to
sign an EVM transaction. The intended order is:

```text
owner mandate + exact X3 payment program
                  |
                  v
       compiler and local policy checks
                  |
                  v
        live 1Click quote and destination-account preflight
                  |
                  v
   x3-guard stores/approves a policy bound to:
     - owner and agent
     - program SHA-256
     - origin/destination assets
     - amount, recipient and refund destination
     - quote fields and deadline
                  |
                  v
 NEAR Chain Signatures signer contract verifies the
 Guard authorization and constructs the exact EVM payload
                  |
                  v
          MPC-derived BSC account signs
                  |
                  v
       BSC transaction and independently verified evidence
```

## Required bindings

An implementation must bind all of these before signing:

1. The exact `.x3` source SHA-256.
2. The owner's mandate version and current revocation state.
3. The agent/approver authorization recorded by x3-guard.
4. The signed 1Click quote hash and complete request echo.
5. The final BSC chain ID, nonce, gas fields, deposit address and BNB value.
6. A one-use intent nonce, rejected again by the signer contract.
7. An expiry no later than both the quote and mandate deadlines.

The signer contract must reconstruct or verify the EVM payload. It must not accept an
arbitrary transaction hash supplied by an agent.

## Access-control choices

- The agent may receive a restricted NEAR function-call access key for x3-guard methods.
- The owner, not the agent, creates or replaces the mandate.
- Approvers must be distinct from the agent.
- The signer contract should accept only a Guard-attested authorization record.
- The MPC key must never be exposed to the agent process or browser.

## Failure conditions

The path must fail closed on:

- quote signature or request-echo mismatch;
- missing, expired, replaced or revoked mandate;
- insufficient approvals;
- nonce reuse;
- a different destination account, amount, chain ID or refund target;
- an uninitialized NEAR delivery account;
- provider errors and unavailable finality data;
- disagreement between configured evidence endpoints.

## What is not proven

- No Chain Signatures account or signer contract exists in this branch.
- No MPC signature or BSC transaction was produced.
- A quote is not authorization.
- An observed BSC deposit and NEAR balance rise do not prove cross-chain causality without a
  verifier that binds both observations.

The current safe milestone is therefore **prepare, inspect and refuse**, not autonomous signing.
