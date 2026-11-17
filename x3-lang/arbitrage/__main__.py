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
    args = parser.parse_args()
    try:
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
                          'message': 'Operation refused. Check explicit inputs and provider availability.',
                          'transactions_submitted': 0}))
        return 2


if __name__ == '__main__':
    raise SystemExit(main())
