# X3CCP — X3Lang policies enforced by a Solidity contract

**Stage:** X3CCP-01 (language + compiler) and X3CCP-02 (on-chain enforcer). Experimental and
unaudited. The compiler accepts testnet targets (`base-sepolia`, `local`) and, by owner decision,
mainnet targets (`base`, `ethereum`/`mainnet`, `optimism`, `arbitrum`, `bsc`). A mainnet policy
compiles with an `X3CCP_MAINNET_UNAUDITED` warning and `explain()` says so on the target line.
Not deployed to any public chain by this change.

X3CCP moves X3Lang from an agent *authorization* language toward a cross-chain smart contract
*control* language. An owner writes a `control` block; it compiles to canonical policy bytes; a
contract that inherits `X3Controlled` enforces them on every marked call, in the same transaction.

```
control treasury_guard {
    target base-sepolia 0x…                 // the X3Controlled contract, nothing else
    owner 0x…                               // must equal the contract's owner
    allow callers 0x…, 0x…
    allow fn withdraw(address to, uint256 amount)
    limit withdraw.amount <= 50_000_000
    limit withdraw.to in 0x…, 0x…
    rate withdraw.amount per 24h <= 200_000_000
    rate withdraw calls per 1h <= 10
    expires 30d
}
```

A refusal reverts with `X3Refused(uint16 clause, uint8 code)`, where `clause` is the **line of the
`.x3` source** that refused. A failed transaction on an explorer therefore points straight at the
rule that stopped it (`python3 x3-lang/x3ccp.py decode-revert 0x…`).

## Authority boundary

X3CCP controls **only contracts whose owners integrate `X3Controlled` and install a policy**. It
cannot take control of an arbitrary immutable third-party contract, and no X3CCP feature claims to.
Controlling how *you* interact with contracts you can't change (an executor proxy, approval
firewalls) is separate future work.

## Pieces

| File | Role |
|---|---|
| `x3-lang/x3ccp.py` | Strict parser, compiler to X3CCP v1 bytes, strict decoder, `predict()` / `simulate()`, `explain()`, CLI |
| `src/X3Controlled.sol` | Abstract enforcer: `installPolicy`, `revokePolicy`, `x3Controlled` modifier |
| `src/X3ControlledTreasury.sol` | Demo ETH treasury: `withdraw`, `setMemo`, `sweep` all controlled |
| `test/X3ControlledTreasury.t.sol` | 27 Foundry tests on the compiler's real output (pinned fixtures) |
| `x3-lang/tests/test_x3ccp_anvil.py` | Predictor vs **320 mined transactions** on a local anvil chain |
| `script/x3ccp_demo.py` | End-to-end demo: `--local` (anvil) or `--base-sepolia` (keystores, explicit approval) |
| `x3-lang/x3ccp_ai.py`, `x3ccp.gbnf`, `x3ccp_mcp.py` | AI-language layer (below) |

## What is enforced, in order

Solidity's ABI decoder runs first (a short call or a dirty `address`/`bool` word reverts with empty
data). Then `X3Controlled`: policy installed → chain id → expiry → caller allowlist → function
allowlist (unlisted = refused) → exact calldata length → each clause in source order. Rate usage
is written only on success, and a later refusal reverts it in the same transaction. `predict()`
mirrors this order exactly; the anvil parity test proves it, byte for byte on the revert data.

| Code | Reason | Code | Reason |
|---|---|---|---|
| 1 | NO_POLICY | 7 | ABOVE_MAX |
| 2 | WRONG_CHAIN | 8 | BELOW_MIN |
| 3 | EXPIRED | 9 | NOT_EQUAL |
| 4 | CALLER_NOT_ALLOWED | 10 | NOT_IN_SET |
| 5 | FUNCTION_NOT_ALLOWED | 11 | RATE_SUM_EXCEEDED |
| 6 | BAD_CALLDATA | 12 | RATE_CALLS_EXCEEDED |

Gas (forge gas report, optimizer 200): `installPolicy` ≈ 0.53M for the example policy; a controlled
`withdraw` ≈ 44k median, up to ≈ 145k on the first call of a new rate window. Treasury runtime size 7.0 KB.

## AI-language layer

AI writes `.x3`; the compiler checks it; the owner signs. Nothing an AI writes authorizes anything.

- **`x3ccp.gbnf`** — grammar for llama.cpp-style constrained decoding, so a local model (e.g. Qwen
  via llama.cpp) emits only syntactically valid policies. Tested against every committed source by
  an exact GBNF recognizer, so it cannot silently drift from the compiler.
- **Repairable diagnostics** — every compile error has a stable code, the line, the field and a
  concrete hint (`"parameters of withdraw(address,uint256): to, amount"`).
- **`explain()`** — one plain-English sentence per clause, keyed by line, for the owner to read
  before installing.
- **MCP server** — `python3 x3-lang/x3ccp_mcp.py` (stdio). Tools: `x3ccp_language`,
  `x3ccp_compile`, `x3ccp_explain`, `x3ccp_simulate`, `x3ccp_decode_revert`. All read-only: no
  network, keys, signing or transactions.

  ```json
  {"mcpServers": {"x3ccp": {"command": "python3", "args": ["/path/to/x3lang/x3-lang/x3ccp_mcp.py"]}}}
  ```

## Reproduce

From the repository root (Foundry + `pip install pycryptodome pytest`):

```bash
(cd x3-lang/x3ccp-evm && forge fmt --check && forge test --fuzz-runs 256 -vv)
python3 x3-lang/tools/x3ccp_fixtures.py --check
PYTHONPATH=x3-lang python3 -m pytest x3-lang/tests/test_x3ccp*.py -v
python3 x3-lang/x3ccp-evm/script/x3ccp_demo.py --local
```

## Base Sepolia pilot — NOT RUN, needs owner approval

`script/x3ccp_demo.py --base-sepolia` deploys the treasury, installs a policy compiled for the
deployed address, funds it with 0.0001 ETH, and mines one allowed withdrawal and two refused
ones (fixed gas limit, so the failures are visible on the explorer), checking each against
`predict()`. It uses Foundry keystore accounts (`--owner-account`, `--agent-account`); no key
passes through the script. It refuses to send anything without `--broadcast
--i-approve-testnet-transactions`. The agent account needs a little Base Sepolia ETH for gas.

## Security limitations

- Unaudited, and mainnet targets are allowed. Neither the constructor nor the compiler restricts a
  policy to testnets; `installPolicy` requires `block.chainid` to match the policy, so a policy
  compiled for one chain cannot be installed on another. A mainnet policy guards real funds with
  an enforcer that has had no external review.
- The owner key is all-powerful: it installs, replaces and revokes policies. Compromise of the
  owner key defeats the policy. There is no timelock on policy changes in v1.
- Installing a new policy starts a new epoch and resets rate usage.
- Rate budgets are shared by all allowed callers of a function: one allowed caller can use up a
  window's budget for the others. Give separate agents separate contracts or functions if that matters.
- Rate windows are **fixed and epoch-aligned**, not rolling: up to 2× a window's budget can pass
  across a window boundary.
- `x3Controlled` reads `msg.sig`/`msg.data`: apply it to **external** functions only. Functions
  without the modifier are not controlled; the contract author decides which are.
- v1 reads static one-word ABI arguments only (`address`, `bool`, `bytes32`, `uintN`). Dynamic
  arguments (`bytes`, `string`, arrays) and function overloads are not supported.
- Revocation takes effect when its transaction executes; it does not undo earlier calls.
- `predict()` assumes no ETH is sent to a non-payable function (that reverts before decoding too).
