"""Run with PYTHONPATH=x3-lang python -m arbitrage --help."""
import argparse
import json
from pathlib import Path

from .adapters import scan_cross_chain
from .execution import prepare_live_quote
from .server import App, create_server


def main():
    parser = argparse.ArgumentParser(description='X3 Arbitrage local scanner and explicit quote preparation')
    commands = parser.add_subparsers(dest='command', required=True)
    serve = commands.add_parser('serve', help='Start loopback dashboard')
    serve.add_argument('--mode', choices=('demo', 'live'), default='demo')
    serve.add_argument('--port', type=int, default=8787)
    serve.add_argument('--routes', type=Path, help='Explicit cross-chain route configuration')
    serve.add_argument('--database', type=Path, default=Path.home() / '.x3' / 'arbitrage' / 'paper.sqlite')
    scan = commands.add_parser('scan', help='One read-only cross-chain discovery pass')
    scan.add_argument('--routes', type=Path, required=True)
    prepare = commands.add_parser('prepare', help='Obtain a policy-checked live deposit quote; never sign or deposit')
    prepare.add_argument('--program', type=Path, required=True)
    prepare.add_argument('--mandate', type=Path, required=True)
    prepare.add_argument('--guard-contract', required=True)
    prepare.add_argument('--receipt', type=Path, required=True)
    prepare.add_argument('--explicit-live-quote', action='store_true', help='Explicitly permit deposit-quote creation')
    trade = commands.add_parser('trade', help='Run a durable automatic cycle; paper by default, live funding disarmed')
    trade.add_argument('--strategy', type=Path, help='JSON with route, tokens, two .x3 mandate sources and limits')
    trade.add_argument('--demo', action='store_true', help='Use a constructed paper fixture')
    trade.add_argument('--mode', choices=('paper', 'live'), default='paper')
    trade.add_argument('--key', required=True)
    trade.add_argument('--database', type=Path, default=Path.home() / '.x3' / 'arbitrage' / 'paper.sqlite')
    trade.add_argument('--scenario', default='success')
    trade.add_argument('--steps', type=int, default=16)
    status = commands.add_parser('trade-status', help='Read an existing durable cycle')
    status.add_argument('--database', type=Path, required=True)
    status.add_argument('--key', required=True)
    continuation = commands.add_parser('trade-continue', help='Continue an existing immutable cycle after restart')
    continuation.add_argument('--database', type=Path, required=True)
    continuation.add_argument('--key', required=True)
    resume = commands.add_parser('trade-resume', help='Resume bounded observation of an unresolved operation')
    resume.add_argument('--database', type=Path, required=True)
    resume.add_argument('--key', required=True)
    tokens = commands.add_parser('tokens', help='Read-only provider-listed exact token catalogue')
    tokens.add_argument('--chain', help='Provider chain identifier: near, sol, eth, arb, pol, bsc, etc.')
    tokens.add_argument('--search', default='')
    args = parser.parse_args()
    try:
        if args.command == 'tokens':
            import time
            import near_intents as ni
            from .catalogue import token_report
            report = token_report(ni.OneClickClient(timeout=8, api_key='').tokens(), now=int(time.time()))
            report['assets'] = [asset for asset in report['assets'] if (not args.chain or asset['network'] == args.chain) and args.search.lower() in (asset['symbol'] + ' ' + asset['asset_id']).lower()]
            report['selected_asset_count'] = len(report['assets'])
            print(json.dumps(report, indent=2))
            return 0
        if args.command in ('trade', 'trade-status', 'trade-resume', 'trade-continue'):
            import time
            from .trader import Trader, PaperExchange, example_strategy, strategy_from_document
            from .adapters import demo_routes
            trader = Trader(args.database)
            if args.command == 'trade-status':
                result = trader.get(args.key)
            elif args.command in ('trade-resume', 'trade-continue'):
                if trader.get(args.key)['mode'] != 'paper':
                    raise ValueError('live reconciliation requires the configured trusted wallet host')
                if args.command == 'trade-resume':
                    trader.resume_observation(args.key)
                result = trader.run(args.key, PaperExchange(trader.store))
            else:
                if bool(args.demo) == bool(args.strategy):
                    raise ValueError('choose exactly one of --demo or --strategy')
                if args.demo:
                    routes, tokens = demo_routes(now=int(time.time()))
                    strategy = example_strategy(routes[0], tokens, now=int(time.time()))
                else:
                    strategy = strategy_from_document(json.loads(args.strategy.read_text()))
                trader.create(args.key, strategy, mode=args.mode, scenario=args.scenario)
                if args.mode == 'live':
                    from .live import LiveExchange
                    backend = LiveExchange()  # Never arm a wallet from this CLI or session.
                else:
                    backend = PaperExchange(trader.store)
                result = trader.run(args.key, backend, max_steps=args.steps)
            print(json.dumps(result, indent=2))
            return 0 if result['phase'] == 'COMPLETED' else 3
        if args.command == 'prepare':
            prepare_live_quote(args.program, mandate=args.mandate, guard_contract=args.guard_contract,
                               receipt=args.receipt, explicit_live=args.explicit_live_quote)
            print(json.dumps({'receipt_written': str(args.receipt), 'transactions_submitted': 0}))
            return 0
        config = json.loads(args.routes.read_text()) if args.routes else None
        if args.command == 'scan':
            import time
            from .engine import rank
            routes, _, errors = scan_cross_chain(config)
            print(json.dumps({'mode': 'live_discovery', 'routes': rank(routes, now=int(time.time())),
                              'errors': errors, 'transactions_submitted': 0}, indent=2))
            return 1 if errors else 0
        app = App(mode=args.mode, database=args.database, config=config)
        server = create_server(app, port=args.port)
        if args.mode == 'live':
            app.start_refresh()
        print(f'X3 Arbitrage · {args.mode} · http://127.0.0.1:{server.server_port}', flush=True)
        try:
            server.serve_forever()
        except KeyboardInterrupt:
            pass
        finally:
            server.server_close()
        return 0
    except Exception as exc:
        print(json.dumps({'error': getattr(exc, 'code', type(exc).__name__),
                          'message': str(exc)[:300] if type(exc) is ValueError else 'Operation refused. Check explicit inputs and provider availability.',
                          'transactions_submitted': 0}))
        return 2


if __name__ == '__main__':
    raise SystemExit(main())
