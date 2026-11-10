# X3 Shield USDC vault design for owner review

Finalized NEAR RPC returned the agent account creation and `create_mandate`
transactions linked in the [judge console](../../x3-lang/near/demo/judge.html).
The historical mandate covers Solana USDC as input and NEAR USDC as output.
Read its current state before planning any new authorization. Its 25 USDC
approval threshold applies at the cap. An official quote, a funded payout
and a custody-enforced swap each need their own evidence.

The observed `x3lang.near` WASM at mainnet block 223959077 was 551,278 bytes.
Its code hash was `GRy6z84ucCR9WMVXyepibVDC34TyKwjWtEi9rteKJtny` and its SHA-256
was `e544adec722714c7608bc87b1971c30c69faf2d15def958c6037129b226edfbe`.
This is an RPC observation of deployed code. Source-to-WASM provenance for
this mainnet deployment remains a release gate. The historical Guard
**testnet** WASM was [544,944 bytes](GUARD_WASM_BYTE_REPRODUCIBILITY.md).

The deployed exports include `create_mandate`, `get_mandate`, `revoke_mandate`,
`agent_register_intent`, `approve_intent`, `check_agent_intent`,
`check_agent_quote`, `agent_accept_quote` and `agent_record_outcome`. See
[the source](../../x3-lang/near/x3-guard/src/mandate.rs) for their interfaces.
[PR #168](https://github.com/Cyptopimpinainteazy/x3lang/pull/168) supplies the
owner-run deployment and evidence checks for the separate **native test NEAR**
vault. It is the baseline custody demonstration. The proposed USDC vault needs
a fresh state layout and its own deployment.

## Proposed custody and execution

1. Pin a single NEP-141 token contract per deployment. Owner funds the vault
   through `ft_transfer_call`. Credit only a transfer from that token whose
   sender is the owner. Confirm storage registration of vault and merchant.
2. An owner transaction binds the compiled mandate hash, exact token, allowed
   recipient, caps, expiry and agent. Use a dedicated agent key scoped to one
   zero-deposit method. The vault authorizes from its own stored policy. Any
   Guard-integrated swap requires a separate exact authorization binding the
   owner, vault, signed quote, deposit, amount and nonce.
3. Require the next monotonic execution nonce. Reserve the amount and account
   for in-flight exposure before scheduling the token transfer. The vault
   attaches the token transfer's required one yoctoNEAR from its own reserve.
   A private callback settles or restores the reservation exactly once. An
   unresolved callback holds the reservation and blocks further execution until
   reconciled. Revocation stops new authorizations. A scheduled payout may
   still complete.
4. Limit USDC to 100,000 units per judge payment and 10,000,000 units across
   any rolling 24 hours, subject to owner approval. Bound rolling spend history
   independently of request volume and deny when capacity is exhausted. Use a
   separate total exposure cap and deadline. Owner withdrawals must exclude
   reservations, and any merchant sweep is a separate owner operation.
5. Keep the hosted runner testnet-first. Validate a strict proposal schema and
   compile the agent's .x3. Use Ollama or a hosted API in the same proposal
   interface. The provider never receives signing credentials. Put scoped test
   signing in a separate process with a single serialized submission queue.
   Reserve attempt quota and an idempotency key before any signing. Persist
   the signed transaction hash before sending and reconcile ambiguous status
   before allowing a retry. Treat local parse failures, RPC failures and
   finalized policy refusals as distinct outcomes.
6. The public console stays read-only until the endpoint passes testnet load
   and abuse checks. Use fixed testnet scenarios, server-validated Turnstile,
   per-visitor and global quotas, an allowance and balance floor, and a pause
   switch. Capture only sanitized public receipt references and verifiable
   outcomes. Keep the mainnet swap owner-signed under a separate review packet.

## Release gates

| Gate | Required evidence |
| --- | --- |
| Custody | Exact testnet WASM and code identity. Restricted agent key. One allowed payout with verified token receipt and balance change. |
| Abuse | Wrong recipient, wrong token, over cap, exhausted rolling budget, replay, revoked and expired attempts. Specific finalized refusal codes or separately labeled pre-broadcast denials. |
| Async accounting | Concurrent requests, failed transfer, failed and duplicate callbacks, pause during flight, and owner recovery under reservation. |
| Judge endpoint | 500 mixed testnet attempts, quota and pause behavior, no double send on timeout or restart, and a finalized receipt for each published chain result. |
| Mainnet 1Click | Fresh executable quote for owner-confirmed addresses and route, verified signature coverage, deposit terms, fees, refund and minimum output. Exact review before owner signs. Verify both finalized legs and retain provider status separately. |
| Publication | Public HTTPS console, clean-clone judge run, pinned evidence links, and no pending gate represented as a pass. |

Owner review covers the vault architecture before implementation, the token
and merchant account, the spending caps, and each exact deployment or funding
transaction before it is signed.
