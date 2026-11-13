"""Credential transport regressions using only synthetic keys and loopback HTTP.

Run with pytest, or with the standard library:
    python3 -m unittest discover -s x3-lang/tests -p test_near_intents_transport.py -v
"""
import importlib.util
import json
import os
import sys
import unittest
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path
from threading import Thread
from unittest.mock import patch


ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
try:
    spec = importlib.util.spec_from_file_location("near_intents_transport_under_test", ROOT / "near_intents.py")
    ni = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(ni)
finally:
    sys.path.pop(0)


class CredentialTransportTests(unittest.TestCase):
    def test_refuses_redirects_before_forwarding_credentials(self):
        for credential_arg in ("api_key", "jwt", "shield"):
            for status in (301, 302, 303, 307, 308):
                with self.subTest(credential_arg=credential_arg, status=status):
                    self._check_redirect(credential_arg, status)

    def _check_redirect(self, credential_arg, status):
        credential = "synthetic-redirect-credential"
        origin_requests, destination_requests = [], []

        class Destination(BaseHTTPRequestHandler):
            def do_GET(self):
                destination_requests.append(dict(self.headers))
                self.send_response(200)
                self.send_header("Content-Type", "application/json")
                self.end_headers()
                self.wfile.write(b"[]")

            def log_message(self, *args):
                pass

        with HTTPServer(("127.0.0.1", 0), Destination) as destination:
            target = f"http://127.0.0.1:{destination.server_port}/unexpected"

            class Origin(BaseHTTPRequestHandler):
                def do_GET(self):
                    origin_requests.append(dict(self.headers))
                    self.send_response(status)
                    self.send_header("Location", target)
                    self.send_header("Content-Type", "application/json")
                    self.end_headers()
                    # Valid JSON must not turn a refused redirect into success.
                    self.wfile.write(b"[]")

                def log_message(self, *args):
                    pass

            with HTTPServer(("127.0.0.1", 0), Origin) as origin:
                servers = (destination, origin)
                workers = [Thread(target=server.serve_forever,
                                  kwargs={"poll_interval": 0.01}, daemon=True)
                           for server in servers]
                for worker in workers:
                    worker.start()
                try:
                    # Never consult proxy settings or ambient credentials for this test.
                    with patch.dict(os.environ, {"no_proxy": "127.0.0.1"}):
                        options = {"base_url": f"http://127.0.0.1:{origin.server_port}", "timeout": 2}
                        if credential_arg == "shield":
                            operation = ni.ShieldIncidents(token=credential, **options).active
                            expected_code, expected_detail = "X3_NEAR_SHIELD_UNREACHABLE", None
                        else:
                            operation = ni.OneClickClient(**options, **{credential_arg: credential}).tokens
                            expected_code = "X3_NEAR_REDIRECT"
                            expected_detail = {"status": status, "retryable": False}
                        with self.assertRaises(ni.NearIntentsError) as err:
                            operation()
                    self.assertEqual(err.exception.code, expected_code)
                    self.assertEqual(err.exception.detail, expected_detail)
                    self.assertNotIn(credential, json.dumps(err.exception.to_dict()))
                finally:
                    for server in servers:
                        server.shutdown()
                    for worker in workers:
                        worker.join(timeout=2)

        self.assertEqual(len(origin_requests), 1)
        headers = {key.lower(): value for key, value in origin_requests[0].items()}
        selected = "x-api-key" if credential_arg == "api_key" else "authorization"
        expected = credential if credential_arg == "api_key" else f"Bearer {credential}"
        self.assertEqual(headers[selected], expected)
        self.assertEqual(destination_requests, [])

    def test_refuses_3xx_from_injected_transport(self):
        for status in (300, 301, 302, 303, 304, 305, 306, 307, 308, 399):
            with self.subTest(status=status):
                calls = []

                def transport(method, url, headers, body, timeout):
                    calls.append((method, url))
                    return status, b"[]"

                client = ni.OneClickClient(api_key="synthetic-redirect-credential", transport=transport)
                with self.assertRaises(ni.NearIntentsError) as err:
                    client.tokens()
                self.assertEqual(err.exception.code, "X3_NEAR_REDIRECT")
                self.assertEqual(err.exception.detail, {"status": status, "retryable": False})
                self.assertEqual(len(calls), 1)

    def test_shield_refuses_injected_redirect_even_with_operational_body(self):
        for status in (300, 301, 302, 303, 304, 305, 306, 307, 308, 399):
            with self.subTest(status=status):
                calls = []

                def transport(method, url, headers, body, timeout):
                    calls.append((method, url))
                    return status, b'{"status":"operational","incidents":[]}'

                client = ni.ShieldIncidents(token="synthetic-shield-credential", transport=transport)
                with self.assertRaises(ni.NearIntentsError) as err:
                    client.active()
                self.assertEqual(err.exception.code, "X3_NEAR_SHIELD_HTTP")
                self.assertEqual(err.exception.detail, {"status": status})
                self.assertEqual(len(calls), 1)


if __name__ == "__main__":
    unittest.main()
