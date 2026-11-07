# X3Lang judge package

**Project:** X3Lang — the programmable trust layer for AI agents and cross-chain finance.

**Track ids:** `security`, `agents`, `defi`.

This package reproduces the current offline policy demonstration. It deliberately separates what
the repository proves locally from wallet authorization, on-chain enforcement, signed quotes,
transactions and settlement.

## One-command demonstration

From the repository root:

```bash
python3 scripts/hackathon/demo.py
```

The command is deterministic and offline. It uses one owner mandate, one policy checker and
three checked-in agent payments. It prints:

| Panel | What a judge sees |
|---|---|
| 1. Owner policy | The actual mandate source, owner, agent, spending caps, recipient allowlist, assets, fees, slippage, approval threshold, refund pin and expiry |
| 2. Agent requests | Three actual X3 programs with hashes: an allowed 25 USDC payment, a 40 USDC payment requiring approval, and a payment to an unlisted recipient |
| 3. Security decision | Local policy verdicts and the exact failed guard (`mandate_recipient` for the attack) |
| 4. Cross-chain path | The lowered Solana USDC to NEAR USDC leg, minimum receipt, recipient and pinned refund address |
| 5. Verification limits | What is proven, what was not contacted, and why no wallet, chain or settlement proof is claimed |

The three programs are checked-in examples. This offline path does not call an AI provider and does
not label a template as model output. The separate AI Money Agent PR is the path that records real
provider/model/response evidence when credentials are available.

For the browser version:

```bash
python3 -m http.server 8000 --directory x3-lang/near/demo
# Open http://localhost:8000/judge.html
```

The browser console shows nine scenarios. Python and the browser evaluate the same mandate through
the same guard order and the tests require identical decisions.

## Shared architecture

```text
English/task request or handwritten agent intent
                    |
                    v
          X3Lang payment program
                    |
                    v
       owner's X3Lang mandate file
                    |
                    v
  Python track_control.evaluate / x3guard.mjs
                    |
          +---------+----------+
          |                    |
     local refusal       local pass/approval
                               |
                               v
                 x3-guard contract checks
                               |
                               v
                     NEAR Intents route
                               |
                               v
                wallet transfer, then evidence
```

The boxes below the local checker are existing release-gate components. This branch does not claim
that the offline demonstration activated them.

## Adversarial coverage

The existing public interfaces are exercised in:

- `x3-lang/tests/test_track_control.py`
- `x3-lang/tests/test_hackathon_support.py`
- `x3-lang/tests/test_near_mandate.py`
- `x3-lang/tests/test_near_intents.py`
- `x3-lang/near/x3-guard/src/mandate_tests.rs`
- `x3-lang/near/x3-guard/src/tests.rs`

| Attack | Local boundary | Contract boundary |
|---|---|---|
| Unauthorized recipient | `mandate_recipient` | `parent` tests cover the same guard |
| Oversized payment | `mandate_per_payment` | covered in mandate tests |
| Wrong origin asset | `mandate_asset` | covered in policy cases |
| Wrong destination asset | `mandate_destination_asset` | covered in policy cases |
| Refund redirect | `mandate_refund_to` | covered in policy cases |
| Fee or slippage above mandate | `mandate_fees`, `mandate_slippage` | covered in policy cases |
| Rolling spend limit | explicit assumed local state | contract state test |
| Revocation | explicit assumed local state | `revoke_mandate` tests |
| Wrong owner | `mandate_owner` before terms | contract owner/agent binding |
| Approval threshold | local `approval_required` | `approve_intent` and accept-time test |
| Nonce replay | local nonce store plus displayed contract-only case | `a_nonce_registers_once` |
| Malformed `.x3` | parser/typechecker failure | not applicable before registration |

The local rolling-spend and revocation scenarios use stated assumptions so the page is
reproducible offline. They are never presented as chain reads.

## Evidence

Run the existing evidence recorder, terminal demo and readiness projection:

```bash
bash scripts/hackathon/verify.sh
```

This produces `/tmp/x3lang-hackathon-readiness.json` and prints the exact test commands and counts.
To create the browser evidence panel:

```bash
python3 x3-lang/near/demo/judge/collect_evidence.py --with-contract
```

The readiness report never calculates a completion percentage. It reports:

- whether evidence describes the current clean commit;
- whether Python, browser, scenario-freshness and contract runs passed;
- which claims are verified locally;
- which chain, wallet, quote and settlement evidence is absent;
- which external GitHub checks must pass on the exact commit.

## Honest claim matrix

| Claim | Current status |
|---|---|
| The mandate and agent intent compile | verified locally |
| The shared policy checker accepts the allowed scenario | verified locally |
| The shared policy checker requires approval above threshold | verified locally |
| An unlisted recipient is refused with `mandate_recipient` | verified locally |
| Browser and Python policy decisions agree | verified locally |
| x3-guard implements the same mandate checks | verified by Rust tests; no deployment asserted |
| An owner approved a request on NEAR | not verified |
| x3-guard accepted or refused a real transaction | not verified |
| A real wallet signed a quote or sent funds | not verified |
| A source-chain deposit and destination delivery settled | not verified |
| One transaction caused the other | not proven |

## Historical disclosure

The compiler, trading VM and much of X3Lang predate the NEAR hackathon. Read
[`PRIOR_WORK.md`](../../PRIOR_WORK.md) and the `baseline...master` comparison. The NEAR Intents
adapter, mandates, Guard and judge-control work are separate features with their own commits.

## Security boundary

The Guard contract does not custody the owner's wallet. An agent that receives the wallet key can
bypass Guard methods. The intended boundary is a restricted agent account that can call Guard,
never a holder of the origin-wallet secret.
