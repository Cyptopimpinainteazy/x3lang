# Agent law, one path

The historical ten-commandment Score and the AGENT_LAW misconduct ladder stay the lists named in [the constitution](../security/X3_AGENT_CONSTITUTION_V1.md). This path does not rename them. The law an agent is born under here is the hash of its written mandate.

What already existed:

- `agent_constitution.py` decides whether a child manifest is admissible. It does not mint an identity, revoke one, or stop a call.
- `agent_admission_signed.py` checks an operator signature over that manifest. A valid signature grants no tool.
- x3-guard checks a mandate at `agent_accept_quote` and the owner can revoke that mandate.
- On invoice_06, Guard approved the quote. The Solana USDC moved from a separate wallet.

This path is the missing local sequence for an X3-routed call:

```bash
python3 scripts/hackathon/agent_law.py
```

The [exhibit](agent-law-proof.html) is that run, generated from the same report. The loopback workbench shows it beside the proof cards at `http://127.0.0.1:8793`. The pending-settlement card stays pending. Do not hand-edit the exhibit; regenerate it with `agent_law.proof_page`.

The same page has a second control, under the editors. Birth, test, authorize, and revoke run that source through the same functions. The adversary is the recipient named in the program, plus one look-alike of that account. The client cannot submit a role, a privilege hash, or a receipt. The session lives in the workbench process and clears when that process stops. It does not replace the archived invoice_06 exhibit.

Birth → compile → test → authorize → execute → monitor → judge.

| Stage | What the run shows |
|---|---|
| Birth | The operator binds `x3-agent.x3lang.near` to the invoice_06 mandate hash, the capability `x3.quote.accept`, the spending limit, and the expiry. The agent cannot birth itself, and birth refuses a Solana transfer capability. |
| Compile | The proposed `.x3` program is compiled and checked against that mandate. |
| Test | Privilege is granted only after an `attacker.near` edit and a look-alike recipient both fail. |
| Authorize and execute | `authorize` is the only function that records a receipt. The receipt names `agent_accept_quote` and records `broadcast: false`. |
| Monitor | The log keeps the spend, the refusal, and the route. |
| Terminate | Operator revocation leaves the receipt in place and the next program returns `revoked`. |
| Court and jury | The exhibit keeps the proposal, the receipt, and the revocation. The agent's own acquittal is stored and ignored. A jury request to reverse the payment leaves the receipt as it was. |
| Evolve | The agent cannot file a replacement. An operator proposal is inert until an independent review. Adopting it keeps the birth mandate hash and the birth spending limit. |

The forbidden `attacker.near` program is marked privileged by hand before the call. The mandate check still refuses it, and the spend stays at the one permitted receipt. A `route: solana` transfer is refused at the same door.

## Limits

- The receipt is local. It does not broadcast a transaction.
- Solana custody stays outside this path. Holding the X3 signer does not hold the wallet that moved the USDC.
- A jury finding cannot reverse a receipt that already exists.
- This is not the production admission registry in the constitution. Root admission there still fails closed, and the live FullAccess key on the agent account is unchanged.
