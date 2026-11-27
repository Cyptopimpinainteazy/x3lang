# Judge local security showcase

Run from the repository root in a Python environment with the project
requirements installed (see [JUDGES.md](../../JUDGES.md)):

```bash
python3 scripts/hackathon/judge_local_showcase.py
python3 scripts/hackathon/judge_local_showcase.py --json
python3 -m unittest discover -s scripts/hackathon -p 'test_judge_local_showcase.py' -v
```

The launcher invokes existing demos and requires specific outcomes:

- **Grok-inspired local policy replay:** three malicious programs rejected by
  deterministic local checks and an ordinary authorized control accepted.
- **Signed local Agent Law:** Ed25519 identities and scripted jury keys,
  successful local compilation, replay blocked, policy-violation retirement
  and retirement still effective after restarting the SQLite runtime.
- **Mt. Gox-inspired accounting model:** six checks over synthetic events:
  changed transaction ID cannot trigger another debit, reserve shortfall
  freezes withdrawals, ordinary withdrawal allowed.

**Evidence classes must not be conflated.** All three launcher stages are
**LOCAL**. None deploys or calls NEAR Guard, holds a real funding key, pays
a GPU provider, verifies public RPC, or demonstrates on-chain Reaper/Scrapyard.
The separately archived **MAINNET** owner-assisted payment evidence is in
[PROOF_CARD.md](../../PROOF_CARD.md): 25 USDC deposited on Solana, 24.9952
USDC received on NEAR, with Guard quote acceptance and seven limitations.

The launcher intentionally aborts on unexpected or incomplete results.
It does not modify the nine-capability claim index, security proof meter,
submission copy, contract, or website. Run any final judge commands on a
known exact source commit.
