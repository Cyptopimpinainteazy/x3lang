"""Exercise compiler behavior and the workbench's HTTP trust boundary."""
import http.client
import json
from threading import Thread
from http.server import ThreadingHTTPServer
import pytest
import dotx3_workbench as wb


def test_real_compiler_and_policy_are_distinct():
    payload = wb.examples()
    allowed = wb.compile_sources(payload)
    assert allowed['compiled'] is True
    assert allowed['decision'] == 'preflight_passed'
    denied = wb.compile_sources({**payload, 'program': payload['program'].replace(
        'to Near.USDC receiver x3-demo.near', 'to Near.USDC receiver attacker.near')})
    assert denied['compiled'] is True
    assert denied['decision'] == 'denied_by_policy'
    assert 'mandate_recipient' in denied['policy']['denied_guards']
    invalid = wb.compile_sources({**payload, 'program': payload['program'].replace(
        '    require nonce unused agent_payment_001\n', '')})
    assert invalid['compiled'] is False
    assert invalid['decision'] == 'compile_error'
    assert invalid['error']['code'] == 'X3_NEAR_NONCE'
    assert len({r['program_sha256'] for r in (allowed, denied, invalid)}) == 3
    for report in (allowed, denied, invalid):
        assert report['wallet_authorized'] is False
        assert report['settlement_proven'] is False
        assert report['onchain_authorization_verified'] is False


@pytest.mark.parametrize('payload', [None, [], {}, {'program': 'x', 'mandate': ''},
    {'program': 'x' * 32769, 'mandate': 'x'}])
def test_bad_sources(payload):
    with pytest.raises(ValueError):
        wb.compile_sources(payload)


@pytest.fixture
def server():
    httpd = ThreadingHTTPServer(('127.0.0.1', 0), wb.Handler)
    thread = Thread(target=httpd.serve_forever, daemon=True)
    thread.start()
    yield httpd
    httpd.shutdown()
    httpd.server_close()
    thread.join()


def send(server, method, path, body=None, headers=None):
    connection = http.client.HTTPConnection('127.0.0.1', server.server_port)
    connection.request(method, path, body, headers or {})
    result = connection.getresponse()
    status, response_headers, data = result.status, dict(result.getheaders()), result.read()
    connection.close()
    return status, response_headers, data


def test_http_compiles_and_serves_only_allowlisted_files(server):
    code, _, data = send(server, 'POST', '/api/compile', json.dumps(wb.examples()),
                         {'Content-Type': 'application/json'})
    assert code == 200
    assert json.loads(data)['decision'] == 'preflight_passed'
    code, headers, data = send(server, 'GET', '/')
    assert code == 200 and b'Compile &amp; check policy' in data
    assert "script-src 'self'" in headers['Content-Security-Policy']
    assert headers['Cache-Control'] == 'no-store'
    for path in ['/../../AGENTS.md', '/api/sign', '/.env', '/api/settle']:
        assert send(server, 'GET', path)[0] == 404


def test_http_origin_and_payload_boundaries(server):
    assert send(server, 'POST', '/api/compile', '{}', {'Content-Type': 'application/json',
        'Origin': 'https://evil.example'})[0] == 403
    assert send(server, 'GET', '/', headers={'Host': 'evil.example'})[0] == 403
    assert send(server, 'POST', '/api/compile', '{}', {'Content-Type': 'text/plain'})[0] == 415
    assert send(server, 'POST', '/api/compile', 'x' * (wb.MAX_BODY + 1),
        {'Content-Type': 'application/json'})[0] == 413
    for payload in ['[]', '{}', '{broken']:
        assert send(server, 'POST', '/api/compile', payload,
            {'Content-Type': 'application/json'})[0] == 400
    assert send(server, 'POST', '/api/dry-preflight', '{}',
        {'Content-Type': 'application/json'})[0] == 403
