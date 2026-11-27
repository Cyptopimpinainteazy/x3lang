# Mt. Gox-inspired accounting replay — local, synthetic

Run:

```bash
python3 scripts/hackathon/replay_mtgox_accounting.py --json
python3 -m unittest discover -s scripts/hackathon -p 'test_replay_mtgox_accounting.py' -v
```

This is an **offline reference model**, not a real attack, Bitcoin consensus implementation,
deployed Guard behavior, actual chain receipt, or proof of solvency. The model tests two
safeguards a future X3 exchange/custody adapter would need: (1) link transaction-ID
variants to one stable withdrawal intent so a retry cannot double-debit; (2) halt new
withdrawals when reported custodial reserves fall below outstanding obligations.
It includes an authorized-control withdrawal. All amounts, withdrawals, and
transaction identifiers are synthetic.

## Historical caveat

Mt. Gox blamed Bitcoin transaction malleability for withdrawal difficulties in 2014.
Christian Decker and Roger Wattenhofer's research found that malleability could
not explain the scale of the exchange's missing funds. Primary research:
https://arxiv.org/abs/1403.6676

This test is inspired by the *failure class*, not a reconstruction of the historical theft.

## Production gap

A production implementation requires authenticated UTXO/receipt validation,
durable idempotency, concurrency-safe locking, on-chain custody-balance proofs,
incident recovery, and independent review. In this model, the caller supplies
the intent/txid relationship; it does not cryptographically prove the relationship.
No NEAR Guard contract, wallet, RPC, keys or public network is accessed or changed.

**Do not include these synthetic checks in the mainnet proof meter or count them as
evidence that X3 Guard currently prevents Bitcoin exchange theft.**
