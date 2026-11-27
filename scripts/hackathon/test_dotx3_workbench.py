"""Exercise compiler behavior and the workbench's HTTP trust boundary."""
import http.client
import json
from pathlib import Path
import subprocess
import sys
from threading import Thread
from http.server import ThreadingHTTPServer
import pytest
import dotx3_workbench as wb


def test_hosted_compiler_import_does_not_start_the_workbench():
    code = """
import sys
from offline_compiler import compile_sources
assert callable(compile_sources)
assert not {'dotx3_workbench', 'agent_law', 'forcefield', 'mainnet_agent_preflight'}.intersection(sys.modules)
"""
    subprocess.run([sys.executable, "-c", code], cwd=Path(__file__).parent, check=True)


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
    wb.reset_door()
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


def test_agent_law_route_stays_local_and_pending_card_stays(server):
    code, _, data = send(server, 'GET', '/api/agent-law')
    assert code == 200
    body = json.loads(data)
    assert body['ok'] is True
    assert body['permitted'] is True
    assert body['forbidden_reason'] == 'mandate_recipient'
    assert body['outside_reason'] == 'route'
    assert body['revoked_reason'] == 'revoked'
    assert body['court_receipts'] == 1
    assert body['broadcast'] is False
    assert body['settlement'] is False
    assert body['settlement_proven'] is False
    assert body['wallet_authorized'] is False
    assert body['solana_custody'] == 'outside_this_path'
    assert set(body['stages_seen']) == set(body['architecture'])
    assert send(server, 'GET', '/api/agent-law', headers={'Host': 'evil.example'})[0] == 403
    page = send(server, 'GET', '/')[2]
    assert b'PENDING FOR THIS INTENT' in page
    assert b'This screen does not establish that proof.' in page
    assert b'id="law-status"' in page
    assert b'id="enforcement"' in page
    assert b'id="door-status"' in page
    assert b'does not replace the archived invoice_06 run' in page


def test_editor_door_permits_the_example_and_refuses_a_stuffed_bypass(server):
    payload = wb.examples()
    code, _, data = send(server, 'POST', '/api/agent-law/act', json.dumps({
        'act': 'birth', 'mandate': payload['mandate'], 'program': payload['program'],
        'privileged': ['aa'], 'role': 'agent',
    }), {'Content-Type': 'application/json'})
    assert code == 200
    assert json.loads(data)['reason'] == 'born'
    code, _, data = send(server, 'POST', '/api/agent-law/act', json.dumps({
        'act': 'test', **payload,
    }), {'Content-Type': 'application/json'})
    assert code == 200 and json.loads(data)['ok'] is True
    code, _, data = send(server, 'POST', '/api/agent-law/act', json.dumps({
        'act': 'authorize', **payload,
    }), {'Content-Type': 'application/json'})
    body = json.loads(data)
    assert code == 200
    assert body['ok'] is True
    assert body['broadcast'] is False
    assert body['settlement'] is False
    assert body['settlement_proven'] is False
    assert body['wallet_authorized'] is False
    assert body['solana_custody'] == 'outside_this_path'
    assert body['receipt']['recipient'] == 'x3-demo.near'
    assert body['receipt']['broadcast'] is False
    attacker = payload['program'].replace(
        'to Near.USDC receiver x3-demo.near', 'to Near.USDC receiver attacker.near')
    code, _, data = send(server, 'POST', '/api/agent-law/act', json.dumps({
        'act': 'authorize', 'mandate': payload['mandate'], 'program': attacker,
        'privileged': ['ff' * 32],
    }), {'Content-Type': 'application/json'})
    blocked = json.loads(data)
    assert code == 200
    assert blocked['ok'] is False
    assert blocked['reason'] == 'not_tested'
    assert blocked['spent'] == 25_000_000
    code, _, data = send(server, 'POST', '/api/agent-law/act', json.dumps({
        'act': 'revoke', 'mandate': payload['mandate'], 'program': payload['program'],
    }), {'Content-Type': 'application/json'})
    assert json.loads(data)['status'] == 'revoked'
    later = payload['program'].replace('agent_payment_001', 'agent_payment_002')
    send(server, 'POST', '/api/agent-law/act', json.dumps({
        'act': 'test', 'mandate': payload['mandate'], 'program': later,
    }), {'Content-Type': 'application/json'})
    code, _, data = send(server, 'POST', '/api/agent-law/act', json.dumps({
        'act': 'authorize', 'mandate': payload['mandate'], 'program': later,
    }), {'Content-Type': 'application/json'})
    stopped = json.loads(data)
    assert stopped['reason'] == 'revoked'
    assert stopped['court_receipts'] == 1
    archived = json.loads(send(server, 'GET', '/api/agent-law')[2])
    assert archived['ok'] is True
    assert archived['forbidden_reason'] == 'mandate_recipient'
    assert archived['court_receipts'] == 1
    assert send(server, 'POST', '/api/agent-law/act', json.dumps({'act': 'birth'}),
                {'Content-Type': 'application/json'})[0] == 400
    assert send(server, 'POST', '/api/agent-law/act', json.dumps({
        'act': 'authorize', **payload,
    }), {'Content-Type': 'application/json', 'Host': 'evil.example'})[0] == 403


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
