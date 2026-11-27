#!/usr/bin/env python3
"""Exercise automatic .x3 paper cycles through real compiler, policies and SQLite.

All quote fills and wallets here are constructed simulations; no network or signer runs.
"""
import argparse
import json
import sys
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'x3-lang'))
from arbitrage.adapters import demo_routes
from arbitrage.trader import Trader, PaperExchange, example_strategy


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    now = int(time.time())
    expected = {'success': ('COMPLETED', 2), 'submit_timeout': ('COMPLETED', 2),
                'outbound_overfill': ('COMPLETED', 2), 'return_loss': ('HOLDING', 1),
                'outbound_refund': ('REFUNDED', 0), 'return_refund': ('HOLDING', 1),
                'rpc_failure': ('RESOLUTION_REQUIRED', 0), 'timeout': ('RESOLUTION_REQUIRED', 0)}
    results = []
    for scenario, (phase, leg_count) in expected.items():
        with tempfile.TemporaryDirectory(prefix='x3-paper-proof-') as directory:
            trader = Trader(Path(directory) / 'wallet.sqlite', clock=lambda: now)
            routes, tokens = demo_routes(now=now)
            strategy = example_strategy(routes[0], tokens, now=now)
            trader.create('proof-' + scenario, strategy, scenario=scenario)
            result = trader.run('proof-' + scenario, PaperExchange(trader.store), max_steps=20)
            assert result['phase'] == phase, (scenario, result['phase'])
            assert len(result['completed_legs']) == leg_count
            assert result['real_transactions_submitted'] == 0
            assert result['simulation_only'] is True
            if scenario == 'success':
                assert result['paper_profit_units'] == '2000000'
                assert result['inventory']['near/demo:near-usdc'] == '0'
            results.append({'scenario': scenario, 'result': result})
    evidence = {'schema': 'x3.arbitrage.automatic-paper-proof.v1', 'at': now,
                'execution': 'real compiler, .x3 mandate checks, SQLite transactions; simulated wallets and fills',
                'real_transactions_submitted': 0, 'network_calls': 0, 'scenarios': results}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(evidence, indent=2) + '\n')
    print(json.dumps({'result': 'pass', 'scenarios': len(results), 'real_transactions_submitted': 0}))


if __name__ == '__main__':
    main()
