"""Real private Unix sockets and Ed25519; no chain RPC, keys or money are used."""
import base64
import json
import os
import socket
import tempfile
import threading
from hashlib import sha256
from pathlib import Path

import pytest
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from cryptography.hazmat.primitives.serialization import Encoding, PublicFormat
from cryptography.exceptions import InvalidSignature
from near_liquidity.quote import canonical_json
from arbitrage.live import WalletGateway


@pytest.mark.parametrize('behavior', ['valid', 'wrong_binding', 'bad_signature'])
def test_private_gateway_transport_binds_signed_response_to_challenged_request(behavior):
    with tempfile.TemporaryDirectory(prefix='x3-wallet-test-') as directory:
        path = Path(directory) / 'wallet.sock'
        key = Ed25519PrivateKey.generate()
        public = key.public_key().public_bytes(Encoding.Raw, PublicFormat.Raw).hex()
        listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        listener.bind(str(path)); path.chmod(0o600); listener.listen(1)
        failures = []
        def server():
            try:
                with listener.accept()[0] as client:
                    data = bytearray()
                    while b'\n' not in data:
                        data.extend(client.recv(65536))
                    wire = bytes(data).strip()
                    request = json.loads(wire)
                    assert request['method'] == 'observe'
                    assert request['payload']['operation'] == 'paper-operation'
                    body = {'request_sha256': sha256(wire).hexdigest(),
                            'result': {'operation': 'paper-operation', 'status': 'pending'}}
                    if behavior == 'wrong_binding': body['request_sha256'] = '0' * 64
                    signature = key.sign(canonical_json(body)) if behavior != 'bad_signature' else bytes(64)
                    response = {'body': body, 'signature': base64.b64encode(signature).decode()}
                    client.sendall(canonical_json(response) + b'\n')
            except Exception as error:
                failures.append(error)
        worker = threading.Thread(target=server, daemon=True); worker.start()
        try:
            gateway = WalletGateway(path, public_key_hex=public, timeout=1)
            if behavior == 'valid':
                assert gateway.request('observe', {'operation': 'paper-operation'})['status'] == 'pending'
            else:
                with pytest.raises((ValueError, InvalidSignature)):
                    gateway.request('observe', {'operation': 'paper-operation'})
        finally:
            worker.join(timeout=3); listener.close()
        assert not worker.is_alive()
        assert not failures


def test_gateway_rejects_world_accessible_socket_before_connecting():
    with tempfile.TemporaryDirectory(prefix='x3-wallet-test-') as directory:
        path = Path(directory) / 'wallet.sock'
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as listener:
            listener.bind(str(path)); path.chmod(0o666)
            with pytest.raises(ValueError, match='private socket'):
                WalletGateway(path, public_key_hex='01' * 32).request('observe', {})


def test_gateway_total_deadline_stops_dribbling_response():
    import time
    with tempfile.TemporaryDirectory(prefix='x3-wallet-test-') as directory:
        path = Path(directory) / 'wallet.sock'
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as listener:
            listener.bind(str(path)); path.chmod(0o600); listener.listen(1)
            def dribble():
                with listener.accept()[0] as client:
                    client.recv(65536)
                    try:
                        for _ in range(30):
                            client.sendall(b' ')
                            time.sleep(0.05)
                    except OSError:
                        pass
            worker = threading.Thread(target=dribble, daemon=True); worker.start()
            started = time.monotonic()
            with pytest.raises(TimeoutError):
                WalletGateway(path, public_key_hex='01' * 32, timeout=0.16).request('observe', {})
            elapsed = time.monotonic() - started
            worker.join(timeout=2)
            assert elapsed < 0.6
            assert not worker.is_alive()


def test_cached_receipt_cannot_weaken_prepared_program_policy(tmp_path):
    from arbitrage.live import validate_prepared_receipt
    from arbitrage.trader import Trader, PaperExchange
    from test_arbitrage_trader import strategy
    import near_intents as ni
    trader = Trader(tmp_path / 'ledger.sqlite', clock=lambda: 1000)
    trader.create('cycle', strategy())
    state = trader.tick('cycle', PaperExchange(trader.store))
    prepared = state['prepared']
    source = prepared['source']
    path = tmp_path / 'source.x3'; path.write_text(source)
    _, compiled = ni.compile_program(str(path), now=__import__('datetime').datetime.fromtimestamp(1000,__import__('datetime').timezone.utc))
    receipt = {'quote_response': {'timestamp': '1970-01-01T00:16:40Z'},
               'program_sha256': sha256(source.encode()).hexdigest(), 'leg': dict(compiled, min_receive=1)}
    with pytest.raises(ValueError, match='policy differs'):
        validate_prepared_receipt(receipt, source, prepared, strategy().tokens, now=1000)
